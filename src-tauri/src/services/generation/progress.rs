//! Shared preview contracts, progress snapshots, and failure/retry reporting.

use std::sync::Arc;

use serde::Serialize;

use crate::models::learning_item::LearningItemDraft;
use crate::services::chunker::MarkdownChunk;
use crate::services::llm::{
    LlmFailure, LlmFailureCode, LlmRetryEvent, LlmRetryState, LlmService, LlmServiceError,
};

use super::default::selection::DefaultSelectionReport;
use super::job::{preview_jobs, PreviewJob};

/// Lightweight chunk metadata returned to callers for observability.
///
/// This keeps UI payloads compact while still exposing enough context to
/// inspect what the chunker produced before an LLM is integrated.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkPreview {
    pub note_path: String,
    pub note_title: String,
    pub heading: String,
    pub section_index: usize,
    pub chunk_index: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub char_count: usize,
    pub preview_text: String,
    pub llm_result: ChunkLlmResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkLlmResult {
    pub status: String,
    pub key_points: Vec<String>,
    /// Canonical Stage B output for the chunk pipeline. Graph generation keeps
    /// this empty until it is migrated onto the same learning-item contract.
    pub items: Vec<LearningItemDraft>,
    /// Graph-pipeline output. Chunk generation uses `items` exclusively.
    pub questions: Vec<ChunkLlmQuestionPreview>,
    pub error: Option<String>,
}

pub(super) fn chunk_preview_is_ready(preview: &ChunkPreview) -> bool {
    !preview.llm_result.items.is_empty()
}

#[derive(Debug, Clone, Serialize)]
pub struct ChunkLlmQuestionPreview {
    pub question: String,
    pub option_a: String,
    pub option_b: String,
    pub option_c: String,
    pub option_d: String,
    pub correct_answer: String,
    pub explanation: String,
}

/// Per-note generation metrics.
#[derive(Debug, Clone, Serialize)]
pub struct NoteGenerationReport {
    pub note_path: String,
    pub note_title: String,
    pub total_chunks: usize,
}

/// Aggregated output for one orchestration run.
///
/// Includes per-chunk Stage A/B model output for preview-only inspection.
/// No DB writes are performed in this phase.
#[derive(Debug, Clone, Serialize)]
pub struct GenerationSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_selection: Option<DefaultSelectionReport>,
    pub total_notes: usize,
    pub total_chunks: usize,
    pub notes_with_chunks: usize,
    pub note_reports: Vec<NoteGenerationReport>,
    pub chunk_previews: Vec<ChunkPreview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerationProgressSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_selection: Option<DefaultSelectionReport>,
    pub job_id: String,
    pub total_notes: usize,
    pub total_chunks: usize,
    pub notes_with_chunks: usize,
    pub completed_chunks: usize,
    /// Chunks or graph bundles skipped after their LLM retries were exhausted.
    pub failed_chunks: usize,
    pub mcq_generated: usize,
    /// Recall questions completed by the graph generation pipeline.
    pub recall_mcq_generated: usize,
    /// Relational questions completed by the graph generation pipeline.
    pub relational_mcq_generated: usize,
    pub progress_percent: u8,
    pub is_paused: bool,
    pub is_cancelled: bool,
    pub is_finished: bool,
    /// Structured, user-facing failure for a terminal generation error.
    pub error: Option<LlmFailure>,
    /// Non-terminal failures for chunks that were skipped while the job continued.
    pub warnings: Vec<LlmFailure>,
    /// Completed previews that already contain validated generated content and
    /// are safe for the frontend to review before the full job finishes.
    pub ready_previews: Vec<ChunkPreview>,
    pub summary: Option<GenerationSummary>,
    /// Human-readable phase description (e.g. "Extracting knowledge" / "Generating questions").
    /// Default and graph jobs each report their own phases.
    pub phase_label: Option<String>,
    /// One-based index of the chunk or bundle currently being processed.
    pub current_chunk: Option<usize>,
    /// Human-readable description of the work currently happening in the background.
    pub activity: Option<String>,
}

pub(super) fn finish_cancelled_job(job: &PreviewJob) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.is_cancelled = true;
    snapshot.is_finished = true;
    snapshot.is_paused = false;
    snapshot.phase_label = None;
    snapshot.current_chunk = None;
    snapshot.activity = None;
}

fn set_progress_percent(snapshot: &mut GenerationProgressSnapshot) {
    snapshot.progress_percent = if snapshot.total_chunks == 0 {
        100
    } else {
        ((snapshot.completed_chunks as f64 / snapshot.total_chunks as f64) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u8
    };
}

/// Records an exhausted or non-retryable LLM error as a terminal job failure.
///
/// Request-level retries have already completed inside `LlmService` before an
/// error reaches this boundary. The detailed error is logged for diagnostics,
/// while the progress snapshot receives its frontend-facing [`LlmFailure`].
pub(super) fn finish_job_with_llm_error(
    job: &PreviewJob,
    error: &LlmServiceError,
    partial_summary: Option<GenerationSummary>,
) {
    log::error!("Generation stopped after terminal LLM failure: {error}");

    finish_job_with_failure(job, error.to_failure(), partial_summary);
}

pub(super) fn finish_job_with_failure(
    job: &PreviewJob,
    failure: LlmFailure,
    partial_summary: Option<GenerationSummary>,
) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.error = Some(failure);
    snapshot.summary = partial_summary;
    snapshot.is_finished = true;
    snapshot.is_paused = false;
    snapshot.phase_label = None;
    snapshot.current_chunk = None;
    snapshot.activity = None;
}

/// Returns whether a terminal request result is local to one chunk and can be skipped.
pub(super) fn is_skippable_chunk_error(error: &LlmServiceError) -> bool {
    matches!(
        error.to_failure().code,
        LlmFailureCode::InvalidResponse | LlmFailureCode::RequestRejected
    )
}

pub(super) fn llm_with_job_retry_activity(
    llm_service: &LlmService,
    job: &Arc<PreviewJob>,
) -> Arc<LlmService> {
    let retry_job = Arc::clone(job);
    Arc::new(llm_service.with_retry_observer(move |event| {
        let mut snapshot = retry_job
            .snapshot
            .lock()
            .expect("preview job snapshot mutex should remain available");
        if snapshot.is_finished || snapshot.is_cancelled || snapshot.is_paused {
            return;
        }
        snapshot.activity = Some(retry_activity(&event));
    }))
}

/// Formats retry state as concise progress-dashboard activity.
pub(super) fn retry_activity(event: &LlmRetryEvent) -> String {
    if event.state == LlmRetryState::Retrying {
        return format!(
            "Retrying LLM request — attempt {} of {}",
            event.next_attempt, event.max_attempts
        );
    }

    let reason = match event.failure.code {
        LlmFailureCode::RateLimited => "Rate limited",
        LlmFailureCode::ProviderUnavailable => "Provider unavailable",
        LlmFailureCode::Connection => "Connection interrupted",
        LlmFailureCode::InvalidResponse => "Invalid model response",
        _ => "LLM request failed",
    };
    let seconds = event.delay.as_secs();
    format!("{reason} — retrying in {seconds} seconds")
}

/// Records a non-terminal LLM failure while allowing the job to continue.
pub(super) fn record_skipped_chunk(job: &PreviewJob, error: &LlmServiceError) {
    log::warn!("Skipping generation unit after exhausted LLM retries: {error}");

    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.failed_chunks += 1;
    snapshot.warnings.push(error.to_failure());
}

/// Builds a diagnostic preview for a skipped markdown chunk without any questions.
pub(super) fn skipped_chunk_preview(
    chunk: &MarkdownChunk,
    error: &LlmServiceError,
) -> ChunkPreview {
    ChunkPreview {
        note_path: chunk.note_path.clone(),
        note_title: chunk.note_title.clone(),
        heading: chunk.heading.clone(),
        section_index: chunk.section_index,
        chunk_index: chunk.chunk_index,
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        char_count: chunk.content.chars().count(),
        preview_text: build_preview_text(&chunk.content, 220),
        llm_result: ChunkLlmResult {
            status: String::from("skipped"),
            key_points: Vec::new(),
            items: Vec::new(),
            questions: Vec::new(),
            error: Some(error.to_failure().message),
        },
    }
}

/// Maps completed work into one bounded portion of the overall progress ring.
pub(super) fn phase_percent(done: usize, total: usize, start_percent: u8, end_percent: u8) -> u8 {
    debug_assert!(start_percent <= end_percent);
    if total == 0 {
        return end_percent;
    }

    let ratio = (done.min(total) as f64 / total as f64).clamp(0.0, 1.0);
    let span = end_percent.saturating_sub(start_percent) as f64;
    (start_percent as f64 + ratio * span)
        .round()
        .clamp(start_percent as f64, end_percent as f64) as u8
}

pub fn get_preview_generation_progress(job_id: &str) -> Result<GenerationProgressSnapshot, String> {
    let jobs = preview_jobs()
        .lock()
        .map_err(|_| String::from("Preview job state is unavailable."))?;

    let job = jobs
        .get(job_id)
        .ok_or_else(|| format!("Preview job '{job_id}' was not found."))?;

    let snapshot = job
        .snapshot
        .lock()
        .map_err(|_| String::from("Preview job snapshot is unavailable."))?
        .clone();

    Ok(snapshot)
}

/// Builds a compact, single-line preview snippet for UI inspection.
pub(super) fn build_preview_text(content: &str, max_chars: usize) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    let total_chars = normalized.chars().count();

    if total_chars <= max_chars {
        return normalized;
    }

    if max_chars <= 3 {
        return String::from("...");
    }

    let head_chars = max_chars.saturating_mul(2) / 3;
    let tail_chars = max_chars.saturating_sub(head_chars);

    let head: String = normalized.chars().take(head_chars).collect();
    let tail: String = normalized
        .chars()
        .skip(total_chars.saturating_sub(tail_chars))
        .collect();

    format!("{} ... {}", head.trim_end(), tail.trim_start())
}
