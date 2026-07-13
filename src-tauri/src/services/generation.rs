use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

use crate::models::note::Note;

use super::chunker::{self, MarkdownChunk};
use super::filesystem;
use super::llm::{LlmService, StageBMcq};

const DEFAULT_MAX_CONCURRENT_CHUNKS: usize = 3;
const PAUSE_POLL_MS: u64 = 250;

static PREVIEW_JOBS: OnceLock<Mutex<HashMap<String, Arc<PreviewJob>>>> = OnceLock::new();
static NEXT_PREVIEW_JOB_ID: AtomicU64 = AtomicU64::new(1);

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
    pub questions: Vec<ChunkLlmQuestionPreview>,
    pub error: Option<String>,
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
    pub total_notes: usize,
    pub total_chunks: usize,
    pub notes_with_chunks: usize,
    pub note_reports: Vec<NoteGenerationReport>,
    pub chunk_previews: Vec<ChunkPreview>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GenerationProgressSnapshot {
    pub job_id: String,
    pub total_notes: usize,
    pub total_chunks: usize,
    pub notes_with_chunks: usize,
    pub completed_chunks: usize,
    pub mcq_generated: usize,
    pub progress_percent: u8,
    pub is_paused: bool,
    pub is_cancelled: bool,
    pub is_finished: bool,
    pub error: Option<String>,
    pub summary: Option<GenerationSummary>,
}

#[derive(Debug)]
struct PreviewJob {
    paused: AtomicBool,
    cancelled: AtomicBool,
    snapshot: Mutex<GenerationProgressSnapshot>,
}

fn preview_jobs() -> &'static Mutex<HashMap<String, Arc<PreviewJob>>> {
    PREVIEW_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_preview_job_id() -> String {
    format!(
        "preview-{}",
        NEXT_PREVIEW_JOB_ID.fetch_add(1, Ordering::Relaxed)
    )
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

pub async fn start_preview_generation_job(vault_path: &str) -> Result<String, String> {
    let notes = filesystem::load_vault_notes(vault_path)?;
    let mut note_reports = Vec::new();
    let mut all_chunks = Vec::new();
    let mut notes_with_chunks = 0;

    for note in &notes {
        let chunks = chunker::chunk_note(note);
        if !chunks.is_empty() {
            notes_with_chunks += 1;
        }

        note_reports.push(NoteGenerationReport {
            note_path: note.path.clone(),
            note_title: note.title.clone(),
            total_chunks: chunks.len(),
        });

        all_chunks.extend(chunks);
    }

    let job_id = next_preview_job_id();
    let initial_snapshot = GenerationProgressSnapshot {
        job_id: job_id.clone(),
        total_notes: notes.len(),
        total_chunks: all_chunks.len(),
        notes_with_chunks,
        completed_chunks: 0,
        mcq_generated: 0,
        progress_percent: 0,
        is_paused: false,
        is_cancelled: false,
        is_finished: false,
        error: None,
        summary: None,
    };

    let job = Arc::new(PreviewJob {
        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        snapshot: Mutex::new(initial_snapshot),
    });

    preview_jobs()
        .lock()
        .expect("preview job map mutex should remain available")
        .insert(job_id.clone(), Arc::clone(&job));

    tauri::async_runtime::spawn(async move {
        let processor = ChunkProcessor::new(LlmService::from_runtime_or_env().ok().map(Arc::new));
        let total_chunks = all_chunks.len();
        let mut ordered_previews = vec![None; total_chunks];

        for (order, chunk) in all_chunks.into_iter().enumerate() {
            if job.cancelled.load(Ordering::Relaxed) {
                break;
            }

            while job.paused.load(Ordering::Relaxed) {
                if job.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(PAUSE_POLL_MS)).await;
            }

            if job.cancelled.load(Ordering::Relaxed) {
                break;
            }

            let preview = processor.process(&chunk).await;
            let mcq_count = preview.llm_result.questions.len();
            ordered_previews[order] = Some(preview);

            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.completed_chunks += 1;
            snapshot.mcq_generated += mcq_count;
            set_progress_percent(&mut snapshot);
        }

        let chunk_previews = ordered_previews.into_iter().flatten().collect::<Vec<_>>();
        let summary = GenerationSummary {
            total_notes: notes.len(),
            total_chunks,
            notes_with_chunks,
            note_reports,
            chunk_previews,
        };

        let mut snapshot = job
            .snapshot
            .lock()
            .expect("preview job snapshot mutex should remain available");
        snapshot.is_cancelled = job.cancelled.load(Ordering::Relaxed);
        if !snapshot.is_cancelled {
            snapshot.summary = Some(summary);
        }
        snapshot.is_finished = true;
        snapshot.is_paused = false;
        set_progress_percent(&mut snapshot);
    });

    Ok(job_id)
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

pub fn set_preview_generation_paused(job_id: &str, paused: bool) -> Result<(), String> {
    let jobs = preview_jobs()
        .lock()
        .map_err(|_| String::from("Preview job state is unavailable."))?;

    let job = jobs
        .get(job_id)
        .ok_or_else(|| format!("Preview job '{job_id}' was not found."))?;

    job.paused.store(paused, Ordering::Relaxed);
    let mut snapshot = job
        .snapshot
        .lock()
        .map_err(|_| String::from("Preview job snapshot is unavailable."))?;
    snapshot.is_paused = paused;

    Ok(())
}

pub fn cancel_preview_generation(job_id: &str) -> Result<(), String> {
    let jobs = preview_jobs()
        .lock()
        .map_err(|_| String::from("Preview job state is unavailable."))?;

    let job = jobs
        .get(job_id)
        .ok_or_else(|| format!("Preview job '{job_id}' was not found."))?;

    job.cancelled.store(true, Ordering::Relaxed);
    // Also unpause so the loop can exit
    job.paused.store(false, Ordering::Relaxed);

    Ok(())
}

#[derive(Debug, Clone)]
struct ChunkProcessor {
    llm_service: Option<Arc<LlmService>>,
}

impl ChunkProcessor {
    fn new(llm_service: Option<Arc<LlmService>>) -> Self {
        Self { llm_service }
    }

    async fn process(&self, chunk: &MarkdownChunk) -> ChunkPreview {
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
            llm_result: self.run_llm_pipeline(chunk).await,
        }
    }

    async fn run_llm_pipeline(&self, chunk: &MarkdownChunk) -> ChunkLlmResult {
        let Some(service) = self.llm_service.as_deref() else {
            return ChunkLlmResult {
                status: String::from("error_init"),
                key_points: Vec::new(),
                questions: Vec::new(),
                error: Some(String::from(
                    "LLM service could not be initialized from runtime settings or env",
                )),
            };
        };

        let stage_a = match service.generate_stage_a_key_points(&chunk.content).await {
            Ok(value) => value,
            Err(err) => {
                return ChunkLlmResult {
                    status: String::from("error_stage_a"),
                    key_points: Vec::new(),
                    questions: Vec::new(),
                    error: Some(err.to_string()),
                };
            }
        };

        let key_points = stage_a
            .key_points
            .into_iter()
            .map(|item| item.knowledge_point)
            .collect::<Vec<_>>();

        if key_points.is_empty() {
            return ChunkLlmResult {
                status: String::from("no_content"),
                key_points,
                questions: Vec::new(),
                error: None,
            };
        }

        let stage_b = match service
            .generate_stage_b_mcqs(&chunk.content, &key_points)
            .await
        {
            Ok(value) => value,
            Err(err) => {
                return ChunkLlmResult {
                    status: String::from("error_stage_b"),
                    key_points,
                    questions: Vec::new(),
                    error: Some(err.to_string()),
                };
            }
        };

        ChunkLlmResult {
            status: String::from("ok"),
            key_points,
            questions: stage_b.questions.into_iter().map(mcq_to_preview).collect(),
            error: None,
        }
    }
}

/// Runs the chunking pipeline for all notes in memory.
///
/// Orchestration flow:
/// 1. Iterate notes.
/// 2. Chunk each note with the markdown chunker.
/// 3. Collect note-level and chunk-level metrics.
pub async fn orchestrate_notes(notes: &[Note]) -> GenerationSummary {
    let mut note_reports = Vec::new();
    let mut all_chunks = Vec::new();
    let mut notes_with_chunks = 0;
    let processor = Arc::new(ChunkProcessor::new(
        LlmService::from_runtime_or_env().ok().map(Arc::new),
    ));

    for note in notes {
        let chunks = chunker::chunk_note(note);
        if !chunks.is_empty() {
            notes_with_chunks += 1;
        }

        note_reports.push(NoteGenerationReport {
            note_path: note.path.clone(),
            note_title: note.title.clone(),
            total_chunks: chunks.len(),
        });

        all_chunks.extend(chunks);
    }

    let total_chunks = all_chunks.len();
    let mut ordered_previews = vec![None; total_chunks];
    if total_chunks > 0 {
        let semaphore = Arc::new(Semaphore::new(DEFAULT_MAX_CONCURRENT_CHUNKS)); // Arc: Atomically Reference Counted
        let mut jobs = JoinSet::new();

        for (order, chunk) in all_chunks.into_iter().enumerate() {
            let semaphore = Arc::clone(&semaphore);
            let processor = Arc::clone(&processor);
            jobs.spawn(async move {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("semaphore should remain open");
                let preview = processor.process(&chunk).await;
                (order, preview)
            });
        }

        while let Some(job_result) = jobs.join_next().await {
            match job_result {
                Ok((order, preview)) => ordered_previews[order] = Some(preview),
                Err(err) => {
                    log::error!("Chunk generation task failed: {err}");
                }
            }
        }
    }

    let chunk_previews = ordered_previews.into_iter().flatten().collect::<Vec<_>>();

    GenerationSummary {
        total_notes: notes.len(),
        total_chunks,
        notes_with_chunks,
        note_reports,
        chunk_previews,
    }
}

/// Convenience entry point that loads notes from a vault path and then
/// delegates to `orchestrate_notes`.
pub async fn orchestrate_vault(vault_path: &str) -> Result<GenerationSummary, String> {
    let notes = filesystem::load_vault_notes(vault_path)?;
    Ok(orchestrate_notes(&notes).await)
}

fn mcq_to_preview(item: StageBMcq) -> ChunkLlmQuestionPreview {
    ChunkLlmQuestionPreview {
        question: item.question,
        option_a: item.option_a,
        option_b: item.option_b,
        option_c: item.option_c,
        option_d: item.option_d,
        correct_answer: item.correct_answer,
        explanation: item.explanation,
    }
}

/// Builds a compact, single-line preview snippet for UI inspection.
fn build_preview_text(content: &str, max_chars: usize) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orchestrator_counts_chunks_across_multiple_notes() {
        let notes = vec![
			Note {
				id: None,
				path: String::from("/vault/rust.md"),
				title: String::from("rust"),
				content: String::from(
					"## Ownership\nRust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves. Rust ownership rules govern borrowing and moves.",
				),
				last_modified: String::from("0"),
			},
			Note {
				id: None,
				path: String::from("/vault/short.md"),
				title: String::from("short"),
				content: String::from("tiny"),
				last_modified: String::from("0"),
			},
		];

        let summary = tauri::async_runtime::block_on(orchestrate_notes(&notes));

        assert_eq!(summary.total_notes, 2);
        assert_eq!(summary.notes_with_chunks, 2);
        assert!(summary.total_chunks >= 2);
        assert_eq!(summary.note_reports.len(), 2);
    }
}
