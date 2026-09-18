//! Learning-item contracts shared by generation, review, scheduling, and persistence.
//! IDs and provenance belong to the backend; the LLM returns GeneratedItem content only.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedItemsOutput {
    /// An empty batch is valid when the source has no suitable targets.
    pub items: Vec<GeneratedItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedItem {
    pub knowledge_point_id: String,
    pub target: String,
    pub answer: String,
    pub explanation: Option<String>,
    pub flashcard: GeneratedFlashcard,
    pub mcq: Option<GeneratedMcq>,
    pub mcq_omission_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedFlashcard {
    pub prompt: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedMcq {
    /// None means reuse the flashcard prompt; resolve before saving.
    pub prompt: Option<String>,
    pub distractors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationPipeline {
    Chunk,
    Graph,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationMetadata {
    /// Preserve the old model string; unknown legacy fields remain None.
    pub model: Option<String>,
    pub provider: Option<String>,
    pub pipeline: Option<GenerationPipeline>,
    /// UTC RFC3339 for new generation, unknown for legacy rows.
    pub generated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceReference {
    pub note_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub knowledge_point: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningItemDraft {
    pub draft_id: String,
    pub generation: GenerationMetadata,
    pub source: SourceReference,
    pub content: GeneratedItem,
}

/// Reviewed generated content. Provenance is resolved from `draft_id` by the backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneratedLearningItemSaveInput {
    pub draft_id: String,
    pub content: GeneratedItem,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewState {
    pub repetitions: i32,
    pub interval_days: i32,
    pub ease_factor: f64,
    /// Preserve existing SQLite timestamp strings during migration.
    pub next_review_at: Option<String>,
    pub last_reviewed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McqOption {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionVariant {
    Mcq {
        id: i64,
        prompt: String,
        options: Vec<McqOption>,
        correct_option_id: String,
    },
    Flashcard {
        id: i64,
        prompt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallState {
    New,
    Scheduled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningItem {
    pub id: i64,
    /// Unknown for migrated MCQs; never invent a learning target.
    pub target: Option<String>,
    /// Nullable for compatibility with the legacy learning-item schema.
    pub answer: Option<String>,
    pub explanation: Option<String>,
    pub space_id: i64,
    pub generation: GenerationMetadata,
    pub source: Option<SourceReference>,
    pub recall_state: RecallState,
    pub schedule: ReviewState,
    /// Legacy items contain only their MCQ. Absence is not an empty flashcard.
    /// New generation and stored legacy data intentionally have different shapes.
    pub variants: Vec<QuestionVariant>,
}

/// Internal persistence input. This is never exposed through Tauri IPC.
#[derive(Debug, Clone)]
pub(crate) struct LearningItemInput {
    pub target: Option<String>,
    pub answer: String,
    pub explanation: Option<String>,
    pub space_id: i64,
    pub generation: GenerationMetadata,
    pub source: Option<SourceReference>,
    pub variants: Vec<VariantInput>,
}

/// Editable content only. Generation and source provenance remain backend-owned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearningItemEditInput {
    pub target: Option<String>,
    pub answer: String,
    pub explanation: Option<String>,
    pub space_id: i64,
    pub variants: Vec<VariantInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum VariantInput {
    Mcq {
        prompt: String,
        options: Vec<McqOption>,
        correct_option_id: String,
    },
    Flashcard {
        prompt: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewRating {
    Again,
    Hard,
    Good,
    Easy,
}

/// Read-only interval previews, calculated from the stored parent schedule.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReviewIntervals {
    pub again: i32,
    pub hard: i32,
    pub good: i32,
    pub easy: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReviewResponse {
    /// Backend computes correctness and scheduling rating.
    Mcq {
        selected_option_id: String,
    },
    Flashcard {
        rating: ReviewRating,
    },
}

/// Transient input only; only the resulting parent schedule is persisted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewSubmission {
    pub learning_item_id: i64,
    pub variant_id: i64,
    pub response: ReviewResponse,
}

/// Structural checks only: source support and equivalent meaning require review.
pub fn validate_generated_item(
    item: &GeneratedItem,
    known_point_ids: &[&str],
) -> Result<(), &'static str> {
    if !known_point_ids.contains(&item.knowledge_point_id.as_str()) {
        return Err("unknown_knowledge_point");
    }
    if [&item.target, &item.answer, &item.flashcard.prompt]
        .iter()
        .any(|s| s.trim().is_empty())
    {
        return Err("empty_core_field");
    }
    match &item.mcq {
        None if item
            .mcq_omission_reason
            .as_ref()
            .map_or(true, |reason| reason.trim().is_empty()) =>
        {
            Err("missing_omission_reason")
        }
        None => Ok(()),
        Some(_) if item.mcq_omission_reason.is_some() => Err("unexpected_omission_reason"),
        Some(mcq) => {
            if mcq.prompt.as_ref().is_some_and(|s| s.trim().is_empty()) {
                return Err("empty_mcq_prompt");
            }
            if mcq.distractors.len() != 3 {
                return Err("distractor_count");
            }
            let mut seen = std::collections::HashSet::new();
            seen.insert(item.answer.trim().to_lowercase());
            for distractor in &mcq.distractors {
                let normalized = distractor.trim().to_lowercase();
                if normalized.is_empty() {
                    return Err("empty_distractor");
                }
                if !seen.insert(normalized) {
                    return Err("duplicate_option");
                }
            }
            Ok(())
        }
    }
}

pub fn validate_generated_output(
    output: &GeneratedItemsOutput,
    known_point_ids: &[&str],
) -> Result<(), &'static str> {
    if output.items.len() > 4 {
        return Err("too_many_items");
    }
    let mut points = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
    for item in &output.items {
        validate_generated_item(item, known_point_ids)?;
        if !points.insert(&item.knowledge_point_id) {
            return Err("duplicate_knowledge_point");
        }
        if !targets.insert(item.target.trim().to_lowercase()) {
            return Err("duplicate_target");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
