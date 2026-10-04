//! Saves reviewed drafts while retaining job ownership and original provenance.

use std::collections::HashMap;

use crate::models::learning_item::{
    validate_generated_item, GeneratedLearningItemSaveInput, GenerationPipeline, LearningItem,
};
use crate::services::{database, learning_items};

use super::job::preview_job;

pub async fn save_generated_learning_items(
    job_id: &str,
    space_id: i64,
    reviewed_items: Vec<GeneratedLearningItemSaveInput>,
) -> Result<Vec<LearningItem>, String> {
    if reviewed_items.is_empty() {
        return Err(String::from(
            "Choose at least one generated learning item to save.",
        ));
    }

    let job = preview_job(job_id)?;
    if job.generation.pipeline != Some(GenerationPipeline::Chunk) {
        return Err(String::from(
            "Canonical learning-item saves are only available for chunk generation jobs.",
        ));
    }

    let (requested_ids, persistence_inputs) = {
        let snapshot = job
            .snapshot
            .lock()
            .map_err(|_| String::from("Preview job snapshot is unavailable."))?;
        let previews = snapshot
            .summary
            .as_ref()
            .map(|summary| summary.chunk_previews.as_slice())
            .unwrap_or(snapshot.ready_previews.as_slice());
        let drafts = previews
            .iter()
            .flat_map(|preview| preview.llm_result.items.iter())
            .map(|draft| (draft.draft_id.as_str(), draft))
            .collect::<HashMap<_, _>>();

        let mut requested_ids = std::collections::HashSet::new();
        let mut persistence_inputs = Vec::with_capacity(reviewed_items.len());
        for (index, reviewed) in reviewed_items.iter().enumerate() {
            if !requested_ids.insert(reviewed.draft_id.clone()) {
                return Err(format!(
                    "Generated learning-item draft '{}' was submitted more than once.",
                    reviewed.draft_id
                ));
            }
            let draft = drafts.get(reviewed.draft_id.as_str()).ok_or_else(|| {
                format!(
                    "Generated learning-item draft '{}' does not belong to this job.",
                    reviewed.draft_id
                )
            })?;
            if reviewed.content.knowledge_point_id != draft.content.knowledge_point_id {
                return Err(format!(
                    "Generated learning-item draft '{}' changed its knowledge point ID.",
                    reviewed.draft_id
                ));
            }
            validate_generated_item(
                &reviewed.content,
                &[draft.content.knowledge_point_id.as_str()],
            )
            .map_err(|reason| {
                format!(
                    "Generated learning-item draft '{}' failed validation: {reason}",
                    reviewed.draft_id
                )
            })?;
            persistence_inputs.push(
                learning_items::from_generated(
                    reviewed.content.clone(),
                    space_id,
                    draft.generation.clone(),
                    draft.source.clone(),
                    index,
                )
                .map_err(|error| error.to_string())?,
            );
        }
        (requested_ids, persistence_inputs)
    };

    {
        let mut saved = job
            .saved_draft_ids
            .lock()
            .map_err(|_| String::from("Generated draft save state is unavailable."))?;
        if let Some(duplicate) = requested_ids.iter().find(|id| saved.contains(*id)) {
            return Err(format!(
                "Generated learning-item draft '{duplicate}' has already been saved."
            ));
        }
        saved.extend(requested_ids.iter().cloned());
    }

    match database::save_learning_items(persistence_inputs).await {
        Ok(items) => Ok(items),
        Err(error) => {
            if let Ok(mut saved) = job.saved_draft_ids.lock() {
                for id in requested_ids {
                    saved.remove(&id);
                }
            }
            Err(format!("Failed to save generated learning items: {error}"))
        }
    }
}
