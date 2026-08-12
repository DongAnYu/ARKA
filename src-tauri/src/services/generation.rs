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
use super::graph_generation::{
    bundle_builder, consolidator, graph_index,
    stage_a_prompt::format_stage_a_graph_user_prompt,
    stage_a_schema::{parse_stage_a_output, stage_a_format_schema},
    stage_b_generation::generate_mcq,
    types::ExtractedKnowledge,
};
use super::llm::{
    JsonGenerationRequest, LlmFailure, LlmFailureCode, LlmService, LlmServiceError, StageBMcq,
};

const DEFAULT_MAX_CONCURRENT_CHUNKS: usize = 3;
const PAUSE_POLL_MS: u64 = 250;

fn configured_llm_service() -> Result<Arc<LlmService>, String> {
    LlmService::from_runtime_or_env()
        .map(Arc::new)
        .map_err(|err| {
            log::warn!("Generation cannot start because LLM configuration is unavailable: {err}");
            err.to_failure().message
        })
}

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
    /// Chunks or graph bundles skipped after their LLM retries were exhausted.
    pub failed_chunks: usize,
    pub mcq_generated: usize,
    pub progress_percent: u8,
    pub is_paused: bool,
    pub is_cancelled: bool,
    pub is_finished: bool,
    /// Structured, user-facing failure for a terminal generation error.
    pub error: Option<LlmFailure>,
    /// Non-terminal failures for chunks that were skipped while the job continued.
    pub warnings: Vec<LlmFailure>,
    pub summary: Option<GenerationSummary>,
    /// Human-readable phase description (e.g. "Extracting knowledge" / "Generating questions").
    /// None for the default single-phase pipeline.
    pub phase_label: Option<String>,
    /// One-based index of the chunk or bundle currently being processed.
    pub current_chunk: Option<usize>,
    /// Human-readable description of the work currently happening in the background.
    pub activity: Option<String>,
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

/// Records an exhausted or non-retryable LLM error as a terminal job failure.
///
/// Request-level retries have already completed inside `LlmService` before an
/// error reaches this boundary. The detailed error is logged for diagnostics,
/// while the progress snapshot receives its frontend-facing [`LlmFailure`].
fn finish_job_with_llm_error(
    job: &PreviewJob,
    error: &LlmServiceError,
    partial_summary: Option<GenerationSummary>,
) {
    log::error!("Generation stopped after terminal LLM failure: {error}");

    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.error = Some(error.to_failure());
    snapshot.summary = partial_summary;
    snapshot.is_finished = true;
    snapshot.is_paused = false;
    snapshot.phase_label = None;
    snapshot.current_chunk = None;
    snapshot.activity = None;
}

/// Returns whether a terminal request result is local to one chunk and can be skipped.
fn is_skippable_chunk_error(error: &LlmServiceError) -> bool {
    matches!(
        error.to_failure().code,
        LlmFailureCode::InvalidResponse | LlmFailureCode::RequestRejected
    )
}

/// Updates the progress snapshot before starting work on a chunk or graph bundle.
fn set_job_activity(job: &PreviewJob, current_chunk: usize, activity: String) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.current_chunk = Some(current_chunk);
    snapshot.activity = Some(activity);
}

/// Records a non-terminal LLM failure while allowing the job to continue.
fn record_skipped_chunk(job: &PreviewJob, error: &LlmServiceError) {
    log::warn!("Skipping generation unit after exhausted LLM retries: {error}");

    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    snapshot.failed_chunks += 1;
    snapshot.warnings.push(error.to_failure());
}

/// Builds a diagnostic preview for a skipped markdown chunk without any questions.
fn skipped_chunk_preview(chunk: &MarkdownChunk, error: &LlmServiceError) -> ChunkPreview {
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
            questions: Vec::new(),
            error: Some(error.to_failure().message),
        },
    }
}

/// Compute a non-regressing two-phase progress percent.
/// Phase 1 maps to 0–50 %, Phase 2 maps to 50–100 %.
fn two_phase_percent(
    phase1_done: usize,
    phase1_total: usize,
    phase2_done: usize,
    phase2_total: usize,
) -> u8 {
    let p1 = if phase1_total == 0 {
        50.0
    } else {
        (phase1_done as f64 / phase1_total as f64) * 50.0
    };
    let p2 = if phase2_total == 0 {
        0.0
    } else {
        (phase2_done as f64 / phase2_total as f64) * 50.0
    };
    (p1 + p2).round().clamp(0.0, 100.0) as u8
}

pub async fn start_preview_generation_job(vault_path: &str) -> Result<String, String> {
    let llm_service = configured_llm_service()?;
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
        failed_chunks: 0,
        mcq_generated: 0,
        progress_percent: 0,
        is_paused: false,
        is_cancelled: false,
        is_finished: false,
        error: None,
        warnings: Vec::new(),
        summary: None,
        phase_label: None,
        current_chunk: None,
        activity: Some(String::from("Preparing generation")),
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
        let processor = ChunkProcessor::new(Some(llm_service));
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

            set_job_activity(
                &job,
                order + 1,
                format!(
                    "Generating questions for chunk {} of {total_chunks}",
                    order + 1
                ),
            );

            let preview = match processor.process(&chunk).await {
                Ok(preview) => preview,
                Err(err) => {
                    if is_skippable_chunk_error(&err) {
                        record_skipped_chunk(&job, &err);
                        skipped_chunk_preview(&chunk, &err)
                    } else {
                        let partial_summary = GenerationSummary {
                            total_notes: notes.len(),
                            total_chunks,
                            notes_with_chunks,
                            note_reports,
                            chunk_previews: ordered_previews.into_iter().flatten().collect(),
                        };
                        finish_job_with_llm_error(&job, &err, Some(partial_summary));
                        return;
                    }
                }
            };
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
        snapshot.current_chunk = None;
        snapshot.activity = None;
        set_progress_percent(&mut snapshot);
    });

    Ok(job_id)
}

const GRAPH_STAGE_A_SYSTEM_PROMPT: &str =
    "You are a knowledge graph extraction specialist. Output only valid JSON.";

/// Starts an async graph-based generation job and returns its job ID.
///
/// Pipeline:
/// 1. Phase 1 — Graph Stage A: extract entities + knowledge points per chunk.
/// 2. Consolidation: merge per-chunk extractions into a single PropositionGraph.
/// 3. Phase 2 — Stage B MCQ: generate one MCQ per bundle from the graph.
///
/// Progress is reported through the shared PreviewJob snapshot and is
/// compatible with `get_preview_generation_progress` / pause / cancel.
pub async fn start_graph_generation_job(vault_path: &str) -> Result<String, String> {
    let llm_service = configured_llm_service()?;
    let notes = filesystem::load_vault_notes(vault_path)?;
    let mut note_reports = Vec::new();
    let mut all_chunks: Vec<MarkdownChunk> = Vec::new();
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

    let total_chunks = all_chunks.len();
    let job_id = next_preview_job_id();
    let initial_snapshot = GenerationProgressSnapshot {
        job_id: job_id.clone(),
        total_notes: notes.len(),
        // Phase 1: total_chunks = num_chunks; updated to bundle_count once Phase 2 starts
        total_chunks,
        notes_with_chunks,
        completed_chunks: 0,
        failed_chunks: 0,
        mcq_generated: 0,
        progress_percent: 0,
        is_paused: false,
        is_cancelled: false,
        is_finished: false,
        error: None,
        warnings: Vec::new(),
        summary: None,
        phase_label: Some(String::from("Extracting knowledge")),
        current_chunk: None,
        activity: Some(String::from("Preparing knowledge extraction")),
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

    let notes_clone = notes.clone();
    tauri::async_runtime::spawn(async move {
        let llm_arc = Some(llm_service);

        // ── Phase 1: Graph Stage A extraction per chunk ───────────────────
        let format_schema = stage_a_format_schema();
        let mut extracted_chunks: Vec<ExtractedKnowledge> = Vec::new();
        let mut chunk_previews_phase1: Vec<(usize, String, String, String)> = Vec::new();
        // (order, note_path, note_title, heading)

        for (order, chunk) in all_chunks.iter().enumerate() {
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

            set_job_activity(
                &job,
                order + 1,
                format!(
                    "Extracting knowledge from chunk {} of {total_chunks}",
                    order + 1
                ),
            );

            chunk_previews_phase1.push((
                order,
                chunk.note_path.clone(),
                chunk.note_title.clone(),
                chunk.heading.clone(),
            ));

            if let Some(llm) = llm_arc.as_deref() {
                let user_prompt =
                    format_stage_a_graph_user_prompt(&chunk.content, "(graph pipeline)");
                let chunk_id = format!("chunk-{}", order);
                let request = JsonGenerationRequest {
                    stage_label: "Graph Stage A",
                    system_prompt: GRAPH_STAGE_A_SYSTEM_PROMPT,
                    user_prompt: &user_prompt,
                    format_schema: format_schema.clone(),
                    payload_preview_chars: 800,
                };

                match llm
                    .generate_json_with_retries(request, |raw_json| {
                        parse_stage_a_output(raw_json, chunk_id.clone())
                            .map_err(|err| LlmServiceError::InvalidOutput(err.to_string()))
                    })
                    .await
                {
                    Ok((extracted, _raw_json, _attempts)) => {
                        extracted_chunks.push(extracted);
                    }
                    Err(err) => {
                        if is_skippable_chunk_error(&err) {
                            record_skipped_chunk(&job, &err);
                        } else {
                            let partial_summary = GenerationSummary {
                                total_notes: notes_clone.len(),
                                total_chunks,
                                notes_with_chunks,
                                note_reports,
                                chunk_previews: Vec::new(),
                            };
                            finish_job_with_llm_error(&job, &err, Some(partial_summary));
                            return;
                        }
                    }
                }
            }

            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.completed_chunks += 1;
            // Phase 1 maps to 0–50 % (bundle_count unknown, so phase2 = 0)
            snapshot.progress_percent =
                two_phase_percent(snapshot.completed_chunks, total_chunks, 0, 0);
        }

        if job.cancelled.load(Ordering::Relaxed) {
            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.is_cancelled = true;
            snapshot.is_finished = true;
            snapshot.is_paused = false;
            snapshot.current_chunk = None;
            snapshot.activity = None;
            return;
        }

        // ── Consolidation: merge extracted chunks into PropositionGraph ───
        {
            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.current_chunk = None;
            snapshot.activity = Some(String::from("Building the knowledge graph"));
        }
        let graph = consolidator::consolidate(extracted_chunks);
        let index = graph_index::build_index(&graph);
        let bundles = bundle_builder::assemble_bundles(&graph, &index);
        let bundle_count = bundles.len();

        // Transition to Phase 2: update label, total_chunks = bundle_count, reset completed
        {
            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.phase_label = Some(String::from("Generating questions"));
            snapshot.total_chunks = bundle_count;
            snapshot.completed_chunks = 0;
            snapshot.current_chunk = None;
            snapshot.activity = Some(String::from("Preparing question generation"));
            snapshot.progress_percent =
                two_phase_percent(total_chunks, total_chunks, 0, bundle_count);
        }

        // ── Phase 2: MCQ generation per bundle ────────────────────────────
        // Build a map from (note_path, heading) -> chunk metadata for summary
        let mut chunk_meta: HashMap<String, (String, String, usize, usize, usize, usize, String)> =
            HashMap::new();
        for chunk in all_chunks.iter() {
            let key = format!("{}::{}", chunk.note_path, chunk.heading);
            chunk_meta.entry(key).or_insert_with(|| {
                (
                    chunk.note_path.clone(),
                    chunk.note_title.clone(),
                    chunk.section_index,
                    chunk.chunk_index,
                    chunk.start_line,
                    chunk.end_line,
                    build_preview_text(&chunk.content, 220),
                )
            });
        }

        let mut ordered_previews: Vec<Option<ChunkPreview>> = vec![None; bundle_count];

        for (bundle_idx, bundle) in bundles.iter().enumerate() {
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

            set_job_activity(
                &job,
                bundle_idx + 1,
                format!("Generating question {} of {bundle_count}", bundle_idx + 1),
            );

            // Determine source chunk context for this bundle from entity chunk_ids
            let source_chunk_id = bundle
                .root_point
                .chunk_id
                .strip_prefix("chunk-")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);

            let (
                note_path,
                note_title,
                section_index,
                chunk_index,
                start_line,
                end_line,
                preview_text,
            ) = all_chunks
                .get(source_chunk_id)
                .map(|c| {
                    (
                        c.note_path.clone(),
                        c.note_title.clone(),
                        c.section_index,
                        c.chunk_index,
                        c.start_line,
                        c.end_line,
                        build_preview_text(&c.content, 220),
                    )
                })
                .unwrap_or_else(|| {
                    (
                        String::from("unknown"),
                        String::from("unknown"),
                        0,
                        bundle_idx,
                        0,
                        0,
                        String::new(),
                    )
                });

            let llm_result = if let Some(llm) = llm_arc.as_deref() {
                match generate_mcq(bundle, llm).await {
                    Ok(mcq) => {
                        let correct_answer = match mcq.correct_index {
                            0 => "A",
                            1 => "B",
                            2 => "C",
                            _ => "D",
                        };
                        let options = &mcq.options;
                        ChunkLlmResult {
                            status: String::from("ok"),
                            key_points: vec![bundle.root_point.point.clone()],
                            questions: vec![ChunkLlmQuestionPreview {
                                question: mcq.question,
                                option_a: options.get(0).cloned().unwrap_or_default(),
                                option_b: options.get(1).cloned().unwrap_or_default(),
                                option_c: options.get(2).cloned().unwrap_or_default(),
                                option_d: options.get(3).cloned().unwrap_or_default(),
                                correct_answer: correct_answer.to_string(),
                                explanation: mcq.explanation,
                            }],
                            error: None,
                        }
                    }
                    Err(err) => {
                        if is_skippable_chunk_error(&err) {
                            record_skipped_chunk(&job, &err);
                            ChunkLlmResult {
                                status: String::from("skipped"),
                                key_points: vec![bundle.root_point.point.clone()],
                                questions: Vec::new(),
                                error: Some(err.to_failure().message),
                            }
                        } else {
                            let partial_summary = GenerationSummary {
                                total_notes: notes_clone.len(),
                                total_chunks: bundle_count,
                                notes_with_chunks,
                                note_reports,
                                chunk_previews: ordered_previews.into_iter().flatten().collect(),
                            };
                            finish_job_with_llm_error(&job, &err, Some(partial_summary));
                            return;
                        }
                    }
                }
            } else {
                ChunkLlmResult {
                    status: String::from("error_init"),
                    key_points: Vec::new(),
                    questions: Vec::new(),
                    error: Some(String::from(
                        "LLM service could not be initialized from runtime settings or env",
                    )),
                }
            };

            let mcq_count = llm_result.questions.len();
            ordered_previews[bundle_idx] = Some(ChunkPreview {
                note_path,
                note_title,
                heading: bundle.root_point.point.chars().take(60).collect::<String>(),
                section_index,
                chunk_index,
                start_line,
                end_line,
                char_count: bundle.root_point.point.chars().count(),
                preview_text,
                llm_result,
            });

            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.completed_chunks += 1;
            snapshot.mcq_generated += mcq_count;
            // Phase 2 maps to 50–100 %
            snapshot.progress_percent = two_phase_percent(
                total_chunks,
                total_chunks,
                snapshot.completed_chunks,
                bundle_count,
            );
        }

        let chunk_previews = ordered_previews.into_iter().flatten().collect::<Vec<_>>();
        let summary = GenerationSummary {
            total_notes: notes_clone.len(),
            total_chunks: bundle_count,
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
        snapshot.progress_percent = 100;
        snapshot.phase_label = None;
        snapshot.current_chunk = None;
        snapshot.activity = None;
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

    async fn process(&self, chunk: &MarkdownChunk) -> Result<ChunkPreview, LlmServiceError> {
        Ok(ChunkPreview {
            note_path: chunk.note_path.clone(),
            note_title: chunk.note_title.clone(),
            heading: chunk.heading.clone(),
            section_index: chunk.section_index,
            chunk_index: chunk.chunk_index,
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            char_count: chunk.content.chars().count(),
            preview_text: build_preview_text(&chunk.content, 220),
            llm_result: self.run_llm_pipeline(chunk).await?,
        })
    }

    async fn run_llm_pipeline(
        &self,
        chunk: &MarkdownChunk,
    ) -> Result<ChunkLlmResult, LlmServiceError> {
        let Some(service) = self.llm_service.as_deref() else {
            return Err(LlmServiceError::InvalidOutput(String::from(
                "LLM service could not be initialized from runtime settings or env",
            )));
        };

        let stage_a = service.generate_stage_a_key_points(&chunk.content).await?;

        let key_points = stage_a
            .key_points
            .into_iter()
            .map(|item| item.knowledge_point)
            .collect::<Vec<_>>();

        if key_points.is_empty() {
            return Ok(ChunkLlmResult {
                status: String::from("no_content"),
                key_points,
                questions: Vec::new(),
                error: None,
            });
        }

        let stage_b = service
            .generate_stage_b_mcqs(&chunk.content, &key_points)
            .await?;

        Ok(ChunkLlmResult {
            status: String::from("ok"),
            key_points,
            questions: stage_b.questions.into_iter().map(mcq_to_preview).collect(),
            error: None,
        })
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
                Ok((order, Ok(preview))) => ordered_previews[order] = Some(preview),
                Ok((_, Err(err))) => {
                    log::error!("Chunk generation failed: {err}");
                }
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
    use crate::services::llm::LlmFailureCode;

    #[test]
    fn generation_progress_serializes_structured_llm_failure() {
        let snapshot = GenerationProgressSnapshot {
            job_id: String::from("preview-1"),
            total_notes: 1,
            total_chunks: 1,
            notes_with_chunks: 1,
            completed_chunks: 0,
            failed_chunks: 0,
            mcq_generated: 0,
            progress_percent: 0,
            is_paused: false,
            is_cancelled: false,
            is_finished: true,
            error: Some(LlmFailure {
                code: LlmFailureCode::Connection,
                message: String::from("The LLM connection failed."),
                retryable: true,
                retry_after_secs: None,
            }),
            warnings: Vec::new(),
            summary: None,
            phase_label: None,
            current_chunk: None,
            activity: None,
        };

        let json = serde_json::to_value(snapshot).expect("generation progress should serialize");

        assert_eq!(json["error"]["code"], "connection");
        assert_eq!(json["error"]["message"], "The LLM connection failed.");
        assert_eq!(json["error"]["retryable"], true);
        assert!(json["error"]["retry_after_secs"].is_null());
        assert_eq!(json["failed_chunks"], 0);
        assert!(json["warnings"].is_array());
        assert!(json["current_chunk"].is_null());
        assert!(json["activity"].is_null());
    }

    #[test]
    fn terminal_llm_error_finishes_job_and_records_failure() {
        let job = PreviewJob {
            paused: AtomicBool::new(true),
            cancelled: AtomicBool::new(false),
            snapshot: Mutex::new(GenerationProgressSnapshot {
                job_id: String::from("preview-terminal-error"),
                total_notes: 1,
                total_chunks: 2,
                notes_with_chunks: 1,
                completed_chunks: 1,
                failed_chunks: 0,
                mcq_generated: 0,
                progress_percent: 50,
                is_paused: true,
                is_cancelled: false,
                is_finished: false,
                error: None,
                warnings: Vec::new(),
                summary: None,
                phase_label: Some(String::from("Generating questions")),
                current_chunk: Some(2),
                activity: Some(String::from("Generating question 2 of 2")),
            }),
        };

        finish_job_with_llm_error(&job, &LlmServiceError::MissingApiKey, None);

        let snapshot = job
            .snapshot
            .lock()
            .expect("test snapshot should be available");
        let failure = snapshot.error.as_ref().expect("failure should be recorded");
        assert_eq!(failure.code, LlmFailureCode::Account);
        assert!(snapshot.is_finished);
        assert!(!snapshot.is_paused);
        assert!(snapshot.phase_label.is_none());
        assert!(snapshot.current_chunk.is_none());
        assert!(snapshot.activity.is_none());
        assert_eq!(snapshot.progress_percent, 50);
        assert!(snapshot.summary.is_none());
    }

    #[test]
    fn invalid_output_is_skippable_but_account_failure_is_terminal() {
        assert!(is_skippable_chunk_error(&LlmServiceError::InvalidOutput(
            String::from("invalid MCQ")
        )));
        assert!(!is_skippable_chunk_error(&LlmServiceError::MissingApiKey));
    }

    #[test]
    fn skipped_chunk_records_warning_without_finishing_job() {
        let job = PreviewJob {
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            snapshot: Mutex::new(GenerationProgressSnapshot {
                job_id: String::from("preview-skipped-error"),
                total_notes: 1,
                total_chunks: 2,
                notes_with_chunks: 1,
                completed_chunks: 0,
                failed_chunks: 0,
                mcq_generated: 0,
                progress_percent: 0,
                is_paused: false,
                is_cancelled: false,
                is_finished: false,
                error: None,
                warnings: Vec::new(),
                summary: None,
                phase_label: None,
                current_chunk: Some(1),
                activity: Some(String::from("Generating questions for chunk 1 of 2")),
            }),
        };

        record_skipped_chunk(
            &job,
            &LlmServiceError::InvalidOutput(String::from("invalid MCQ")),
        );

        let snapshot = job
            .snapshot
            .lock()
            .expect("test snapshot should be available");
        assert_eq!(snapshot.failed_chunks, 1);
        assert_eq!(snapshot.warnings.len(), 1);
        assert_eq!(snapshot.warnings[0].code, LlmFailureCode::InvalidResponse);
        assert!(!snapshot.is_finished);
        assert!(snapshot.error.is_none());
    }

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
