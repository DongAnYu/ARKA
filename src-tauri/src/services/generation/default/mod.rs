//! Whole-note Default generation, candidate sources, and fixed selection policy.

use std::sync::Arc;

use crate::models::generation_options::GenerationOptions;
use crate::models::learning_item::{GenerationMetadata, GenerationPipeline};
use crate::models::model_settings::DEFAULT_LLM_CONCURRENCY;
use crate::models::note::Note;
use crate::services::filesystem;
use crate::services::llm::LlmService;

use super::job::{
    configured_llm_concurrency, configured_llm_service, generation_metadata, next_preview_job_id,
    preview_jobs,
};
use super::progress::{finish_job_with_failure, llm_with_job_retry_activity, GenerationSummary};

pub(crate) mod candidates;
mod pipeline;
pub(crate) mod selection;

pub async fn start_preview_generation_job(
    vault_path: &str,
    options: Option<GenerationOptions>,
) -> Result<String, String> {
    let options = GenerationOptions::resolve(options);
    let llm_service = configured_llm_service()?;
    let generation = generation_metadata(&llm_service, GenerationPipeline::Chunk);
    let concurrency = configured_llm_concurrency().await?;
    let notes = filesystem::load_vault_notes(vault_path)?;
    let job_id = next_preview_job_id();
    let (job, reports) = pipeline::prepare_job(job_id.clone(), &notes, options, generation);
    preview_jobs()
        .lock()
        .expect("preview job map remains available")
        .insert(job_id.clone(), Arc::clone(&job));
    let llm_service = llm_with_job_retry_activity(&llm_service, &job);
    tauri::async_runtime::spawn(pipeline::run(job, llm_service, concurrency, reports));
    Ok(job_id)
}

/// Runs the Default flow with the same ceiling and selection as preview jobs.
pub async fn orchestrate_notes(notes: &[Note]) -> GenerationSummary {
    orchestrate_notes_with_concurrency(notes, DEFAULT_LLM_CONCURRENCY, false).await
}

pub async fn orchestrate_notes_for_evaluation(notes: &[Note]) -> GenerationSummary {
    orchestrate_notes_with_concurrency(notes, DEFAULT_LLM_CONCURRENCY, true).await
}

async fn orchestrate_notes_with_concurrency(
    notes: &[Note],
    concurrency: usize,
    preserve_failed_chunks: bool,
) -> GenerationSummary {
    let llm = LlmService::from_runtime_or_env();
    let generation = llm
        .as_ref()
        .ok()
        .map(|llm| generation_metadata(llm, GenerationPipeline::Chunk))
        .unwrap_or(GenerationMetadata {
            model: None,
            provider: None,
            pipeline: Some(GenerationPipeline::Chunk),
            generated_at: None,
        });
    let (job, reports) = pipeline::prepare_job(
        next_preview_job_id(),
        notes,
        GenerationOptions::default(),
        generation,
    );
    match llm {
        Ok(llm) => pipeline::run(Arc::clone(&job), Arc::new(llm), concurrency, reports).await,
        Err(error) => {
            let failure = error.to_failure();
            let snapshot = job.snapshot.lock().unwrap();
            let summary = GenerationSummary {
                total_notes: snapshot.total_notes,
                total_chunks: snapshot.total_chunks,
                notes_with_chunks: snapshot.notes_with_chunks,
                note_reports: reports,
                chunk_previews: job
                    .default_generation
                    .as_ref()
                    .unwrap()
                    .chunks()
                    .iter()
                    .map(|chunk| {
                        let mut preview = pipeline::preview(chunk);
                        preview.llm_result.status = "error".into();
                        preview.llm_result.error = Some(failure.message.clone());
                        preview
                    })
                    .collect(),
                default_selection: snapshot.default_selection.clone(),
            };
            drop(snapshot);
            finish_job_with_failure(&job, failure, Some(summary));
        }
    }
    let mut summary = job
        .snapshot
        .lock()
        .unwrap()
        .summary
        .clone()
        .expect("in-memory generation retains its summary");
    if preserve_failed_chunks {
        for preview in &mut summary.chunk_previews {
            if preview.llm_result.status == "skipped" {
                preview.llm_result.status = "error".into();
            }
        }
    }
    summary
}

/// Convenience entry point that loads notes from a vault path and then
/// delegates to `orchestrate_notes`.
pub async fn orchestrate_vault(vault_path: &str) -> Result<GenerationSummary, String> {
    let notes = filesystem::load_vault_notes(vault_path)?;
    let llm_concurrency = configured_llm_concurrency().await?;
    Ok(orchestrate_notes_with_concurrency(&notes, llm_concurrency, false).await)
}
