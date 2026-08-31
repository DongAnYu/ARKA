use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Serialize;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

use crate::models::model_settings::{EmbeddingModelConfig, DEFAULT_LLM_CONCURRENCY};
use crate::models::note::Note;

use super::chunker::{self, MarkdownChunk};
use super::embedding::{prepare_embedding_service, EmbeddingService, EmbeddingServiceError};
use super::graph_generation::{
    bundle_builder, consolidator,
    entity_resolution::{
        pipeline::{
            resolve_graph_entities_with_progress, EntityResolutionConfig,
            EntityResolutionPipelineError, EntityResolutionProgress,
        },
        semantic_verifier::EntityVerificationError,
    },
    graph_index,
    stage_a_prompt::format_stage_a_graph_user_prompt,
    stage_a_schema::{parse_stage_a_output, stage_a_format_schema},
    stage_b_generation::generate_mcq,
    stage_b_schema::GeneratedMCQ,
    types::{ExtractedKnowledge, GraphContextBundle, QuestionType},
};
use super::llm::{
    LlmFailure, LlmFailureCode, LlmRetryEvent, LlmRetryState, LlmService, LlmServiceError,
    StageBMcq, StructuredGenerationRequest,
};
use super::{database, filesystem};

const PAUSE_POLL_MS: u64 = 250;

fn configured_llm_service() -> Result<Arc<LlmService>, String> {
    LlmService::from_runtime_or_env()
        .map(Arc::new)
        .map_err(|err| {
            log::warn!("Generation cannot start because LLM configuration is unavailable: {err}");
            err.to_failure().message
        })
}

/// Loads the single concurrency limit shared by every application LLM stage.
async fn configured_llm_concurrency() -> Result<usize, String> {
    let model_config = database::load_model_config()
        .await
        .map_err(|error| format!("Failed to load LLM concurrency setting: {error}"))?;
    let concurrency = model_config.validated_llm_concurrency()?;
    log::info!("Generation LLM concurrency resolved (limit={concurrency})");
    Ok(concurrency)
}

#[derive(Debug)]
struct PreparedEmbeddingService {
    service: Option<Arc<EmbeddingService>>,
    warning: Option<LlmFailure>,
}

/// Loads and validates the saved embedding settings before graph work starts.
///
/// An empty model is an intentional opt-out for the MVP and produces a visible
/// non-terminal warning. Once a model is selected, malformed settings are a
/// setup error and reject the job instead of silently disabling resolution.
async fn configured_embedding_service() -> Result<PreparedEmbeddingService, String> {
    let model_config = database::load_model_config()
        .await
        .map_err(|error| format!("Failed to load embedding settings: {error}"))?;

    prepare_embedding_service_for_generation(&model_config.embedding_config())
}

fn prepare_embedding_service_for_generation(
    settings: &EmbeddingModelConfig,
) -> Result<PreparedEmbeddingService, String> {
    if settings.selected_model.trim().is_empty() {
        let message = String::from(
            "Entity resolution was skipped because no embedding model is configured. Configure one in Models to enable entity deduplication.",
        );
        log::warn!("{message}");

        return Ok(PreparedEmbeddingService {
            service: None,
            warning: Some(LlmFailure {
                code: LlmFailureCode::Setup,
                message,
                retryable: false,
                retry_after_secs: None,
            }),
        });
    }

    let (provider, service) = prepare_embedding_service(settings).map_err(|error| {
        log::warn!(
            "Graph generation cannot start because embedding configuration is invalid: {error}"
        );
        error.to_string()
    })?;
    log::info!(
        "Embedding config resolved for graph generation (provider={}, base_url={}, model={}, timeout_secs={})",
        provider.as_str(),
        service.config().base_url(),
        service.config().model(),
        service.config().timeout_secs()
    );

    Ok(PreparedEmbeddingService {
        service: Some(Arc::new(service)),
        warning: None,
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

    finish_job_with_failure(job, error.to_failure(), partial_summary);
}

/// Records an entity-resolution failure using the progress dashboard's shared
/// provider-neutral failure shape.
fn finish_job_with_entity_resolution_error(
    job: &PreviewJob,
    error: &EntityResolutionPipelineError,
    partial_summary: Option<GenerationSummary>,
) {
    log::error!("Generation stopped after entity resolution failed: {error}");

    finish_job_with_failure(job, entity_resolution_failure(error), partial_summary);
}

fn finish_job_with_failure(
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

fn entity_resolution_failure(error: &EntityResolutionPipelineError) -> LlmFailure {
    if let EntityResolutionPipelineError::Verification(EntityVerificationError::Llm(source)) = error
    {
        let mut failure = source.to_failure();
        failure.message = format!("Entity resolution verifier failed: {}", failure.message);
        return failure;
    }

    let (code, retryable) = match error {
        EntityResolutionPipelineError::Embedding(source) => match source {
            EmbeddingServiceError::HttpClientBuild(_) => (LlmFailureCode::Setup, false),
            EmbeddingServiceError::Connect { .. } | EmbeddingServiceError::Http(_) => {
                (LlmFailureCode::Connection, true)
            }
            EmbeddingServiceError::HttpStatus { status, .. }
                if matches!(status.as_u16(), 401 | 402 | 403) =>
            {
                (LlmFailureCode::Account, false)
            }
            EmbeddingServiceError::HttpStatus { status, .. } if status.as_u16() == 404 => {
                (LlmFailureCode::Setup, false)
            }
            EmbeddingServiceError::HttpStatus { status, .. } if status.as_u16() == 429 => {
                (LlmFailureCode::RateLimited, true)
            }
            EmbeddingServiceError::HttpStatus { status, .. } if status.is_server_error() => {
                (LlmFailureCode::ProviderUnavailable, true)
            }
            EmbeddingServiceError::HttpStatus { .. } => (LlmFailureCode::RequestRejected, false),
            EmbeddingServiceError::ResponseDecode(_)
            | EmbeddingServiceError::InvalidResponse(_) => (LlmFailureCode::InvalidResponse, false),
        },
        EntityResolutionPipelineError::CandidateGeneration(_) => (LlmFailureCode::Setup, false),
        EntityResolutionPipelineError::Verification(_) => (LlmFailureCode::Unknown, false),
        EntityResolutionPipelineError::MergePlanning(_)
        | EntityResolutionPipelineError::GraphRewrite(_) => (LlmFailureCode::Unknown, false),
    };

    LlmFailure {
        code,
        message: format!("Entity resolution failed: {error}"),
        retryable,
        retry_after_secs: None,
    }
}

/// Returns whether a terminal request result is local to one chunk and can be skipped.
fn is_skippable_chunk_error(error: &LlmServiceError) -> bool {
    matches!(
        error.to_failure().code,
        LlmFailureCode::InvalidResponse | LlmFailureCode::RequestRejected
    )
}

type GraphStageAJobResult = (usize, Result<ExtractedKnowledge, LlmServiceError>);

/// Starts one independent graph-extraction request with owned task data.
fn spawn_graph_stage_a_job(
    jobs: &mut JoinSet<GraphStageAJobResult>,
    order: usize,
    chunk: &MarkdownChunk,
    llm: &Arc<LlmService>,
    format_schema: &serde_json::Value,
) {
    let chunk = chunk.clone();
    let llm = Arc::clone(llm);
    let format_schema = format_schema.clone();
    jobs.spawn(async move {
        let user_prompt = format_stage_a_graph_user_prompt(&chunk.content, "(graph pipeline)");
        let chunk_id = format!("chunk-{order}");
        let request = StructuredGenerationRequest {
            stage_label: "Graph Stage A",
            schema_name: "graph_stage_a",
            system_prompt: GRAPH_STAGE_A_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            schema: format_schema,
            payload_preview_chars: 800,
        };
        let result = llm
            .generate_json_with_retries(request, |raw_json| {
                parse_stage_a_output(raw_json, chunk_id.clone())
                    .map_err(|error| LlmServiceError::InvalidOutput(error.to_string()))
            })
            .await
            .map(|(extracted, _, _)| extracted);
        (order, result)
    });
}

/// Shows aggregate work because several chunks may be active simultaneously.
fn update_parallel_stage_a_activity(
    job: &PreviewJob,
    total_chunks: usize,
    in_flight: usize,
    concurrency_limit: usize,
) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    if snapshot.is_finished || snapshot.is_cancelled {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(format!(
        "Extracted {} of {total_chunks} chunks · {in_flight} active (limit {concurrency_limit})",
        snapshot.completed_chunks
    ));
}

type GraphStageBJobResult = (usize, Result<GeneratedMCQ, LlmServiceError>);

/// Starts one independent question-generation request.
fn spawn_graph_stage_b_job(
    jobs: &mut JoinSet<GraphStageBJobResult>,
    bundle_index: usize,
    bundle: &GraphContextBundle,
    llm: &Arc<LlmService>,
) {
    let bundle = bundle.clone();
    let llm = Arc::clone(llm);
    jobs.spawn(async move {
        let result = generate_mcq(&bundle, &llm).await;
        (bundle_index, result)
    });
}

/// Shows aggregate Stage B work because requests can finish out of order.
fn update_parallel_stage_b_activity(
    job: &PreviewJob,
    total_bundles: usize,
    in_flight: usize,
    concurrency_limit: usize,
) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    if snapshot.is_finished || snapshot.is_cancelled {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(format!(
        "Generated {} of {total_bundles} questions · {in_flight} active (limit {concurrency_limit})",
        snapshot.completed_chunks
    ));
}

/// Shows aggregate progress for the legacy per-chunk LLM pipeline.
fn update_parallel_chunk_activity(
    job: &PreviewJob,
    total_chunks: usize,
    in_flight: usize,
    concurrency_limit: usize,
) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    if snapshot.is_finished || snapshot.is_cancelled {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(format!(
        "Generated {} of {total_chunks} chunks · {in_flight} active (limit {concurrency_limit})",
        snapshot.completed_chunks
    ));
}

const GRAPH_STAGE_A_END_PERCENT: u8 = 50;
const GRAPH_ENTITY_RESOLUTION_END_PERCENT: u8 = 65;
const GRAPH_VERIFICATION_END_PERCENT: u8 = GRAPH_ENTITY_RESOLUTION_END_PERCENT - 1;

/// Maps provider-neutral resolver milestones onto the app's progress activity.
fn record_entity_resolution_progress(
    job: &PreviewJob,
    progress: EntityResolutionProgress,
    concurrency_limit: usize,
) {
    let (activity, progress_percent) = match progress {
        EntityResolutionProgress::GeneratingEmbeddings { entity_count } => {
            (
                format!("Generating embeddings for {entity_count} entities"),
                GRAPH_STAGE_A_END_PERCENT,
            )
        }
        EntityResolutionProgress::CandidatesGenerated { candidate_count } => {
            (
                format!(
                    "Found {candidate_count} candidate entity pairs for semantic verification"
                ),
                GRAPH_STAGE_A_END_PERCENT + 1,
            )
        }
        // Candidate-selection events are useful to eval/reporting callers but
        // arrive synchronously as one burst, so they should not churn the UI.
        EntityResolutionProgress::CandidateSelected { .. } => return,
        EntityResolutionProgress::VerifyingCandidates {
            completed_pairs,
            total_pairs,
            in_flight_pairs,
            ..
        } => (
            format!(
                "Verified {completed_pairs} of {total_pairs} entity pairs · {in_flight_pairs} active (limit {concurrency_limit})"
            ),
            phase_percent(
                completed_pairs,
                total_pairs,
                GRAPH_STAGE_A_END_PERCENT + 1,
                GRAPH_VERIFICATION_END_PERCENT,
            ),
        ),
        EntityResolutionProgress::Finalizing {
            verified_pair_count,
        } => (
            format!("Applying entity decisions from {verified_pair_count} verified pairs"),
            GRAPH_ENTITY_RESOLUTION_END_PERCENT,
        ),
    };

    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview job snapshot mutex should remain available");
    if snapshot.is_finished || snapshot.is_cancelled {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(activity);
    // Retry notifications and concurrent verifier completions may be observed
    // close together. Never let a late event move the progress ring backwards.
    snapshot.progress_percent = snapshot.progress_percent.max(progress_percent);
}

/// Attaches live retry activity to one job without sharing progress state globally.
fn llm_with_job_retry_activity(llm_service: &LlmService, job: &Arc<PreviewJob>) -> Arc<LlmService> {
    let retry_job = Arc::clone(job);
    Arc::new(llm_service.with_retry_observer(move |event| {
        let mut snapshot = retry_job
            .snapshot
            .lock()
            .expect("preview job snapshot mutex should remain available");
        if snapshot.is_finished || snapshot.is_cancelled {
            return;
        }
        snapshot.activity = Some(retry_activity(&event));
    }))
}

/// Formats retry state as concise progress-dashboard activity.
fn retry_activity(event: &LlmRetryEvent) -> String {
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

/// Maps completed work into one bounded portion of the overall progress ring.
fn phase_percent(done: usize, total: usize, start_percent: u8, end_percent: u8) -> u8 {
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

pub async fn start_preview_generation_job(vault_path: &str) -> Result<String, String> {
    let llm_service = configured_llm_service()?;
    let llm_concurrency = configured_llm_concurrency().await?;
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
        recall_mcq_generated: 0,
        relational_mcq_generated: 0,
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
        let llm_service = llm_with_job_retry_activity(&llm_service, &job);
        let processor = Arc::new(ChunkProcessor::new(Some(llm_service)));
        let total_chunks = all_chunks.len();
        let mut ordered_previews = vec![None; total_chunks];
        let mut jobs = JoinSet::new();
        let mut next_order = 0usize;

        while next_order < total_chunks || !jobs.is_empty() {
            if job.cancelled.load(Ordering::Relaxed) {
                jobs.abort_all();
                break;
            }

            while job.paused.load(Ordering::Relaxed) && jobs.is_empty() {
                if job.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(PAUSE_POLL_MS)).await;
            }

            if job.cancelled.load(Ordering::Relaxed) {
                jobs.abort_all();
                break;
            }

            while !job.paused.load(Ordering::Relaxed)
                && !job.cancelled.load(Ordering::Relaxed)
                && next_order < total_chunks
                && jobs.len() < llm_concurrency
            {
                let order = next_order;
                let chunk = all_chunks[order].clone();
                let processor = Arc::clone(&processor);
                jobs.spawn(async move { (order, processor.process(&chunk).await) });
                next_order += 1;
            }

            update_parallel_chunk_activity(&job, total_chunks, jobs.len(), llm_concurrency);
            let Some(joined) = jobs.join_next().await else {
                continue;
            };
            let (order, result) = match joined {
                Ok(completed) => completed,
                Err(error) => {
                    jobs.abort_all();
                    let partial_summary = GenerationSummary {
                        total_notes: notes.len(),
                        total_chunks,
                        notes_with_chunks,
                        note_reports,
                        chunk_previews: ordered_previews.into_iter().flatten().collect(),
                    };
                    finish_job_with_failure(
                        &job,
                        LlmFailure {
                            code: LlmFailureCode::Unknown,
                            message: format!("Chunk generation worker failed: {error}"),
                            retryable: false,
                            retry_after_secs: None,
                        },
                        Some(partial_summary),
                    );
                    return;
                }
            };

            let preview = match result {
                Ok(preview) => preview,
                Err(err) => {
                    if is_skippable_chunk_error(&err) {
                        record_skipped_chunk(&job, &err);
                        skipped_chunk_preview(&all_chunks[order], &err)
                    } else {
                        jobs.abort_all();
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
/// 3. Entity resolution: embed, verify, merge, rewrite, and rebuild the index.
/// 4. Phase 2 — Stage B MCQ: generate one MCQ per bundle from the resolved graph.
///
/// Progress is reported through the shared PreviewJob snapshot and is
/// compatible with `get_preview_generation_progress` / pause / cancel.
pub async fn start_graph_generation_job(vault_path: &str) -> Result<String, String> {
    let llm_service = configured_llm_service()?;
    let llm_concurrency = configured_llm_concurrency().await?;
    // Load and validate embedding settings before reading notes or creating a
    // background job. The prepared service is consumed by entity resolution in
    // the next graph-pipeline integration step.
    let prepared_embedding = configured_embedding_service().await?;
    let embedding_warning = prepared_embedding.warning;
    let embedding_service = prepared_embedding.service;
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
        recall_mcq_generated: 0,
        relational_mcq_generated: 0,
        progress_percent: 0,
        is_paused: false,
        is_cancelled: false,
        is_finished: false,
        error: None,
        warnings: embedding_warning.into_iter().collect(),
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
        let llm_arc = Some(llm_with_job_retry_activity(&llm_service, &job));

        // ── Phase 1: Graph Stage A extraction per chunk ───────────────────
        let format_schema = stage_a_format_schema();
        let mut extracted_by_order = vec![None; total_chunks];
        let mut jobs = JoinSet::new();
        let mut next_order = 0usize;

        while next_order < total_chunks || !jobs.is_empty() {
            if job.cancelled.load(Ordering::Relaxed) {
                jobs.abort_all();
                break;
            }

            while job.paused.load(Ordering::Relaxed) && jobs.is_empty() {
                if job.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(PAUSE_POLL_MS)).await;
            }

            while !job.paused.load(Ordering::Relaxed)
                && !job.cancelled.load(Ordering::Relaxed)
                && next_order < total_chunks
                && jobs.len() < llm_concurrency
            {
                spawn_graph_stage_a_job(
                    &mut jobs,
                    next_order,
                    &all_chunks[next_order],
                    llm_arc
                        .as_ref()
                        .expect("graph generation should always have an LLM service"),
                    &format_schema,
                );
                next_order += 1;
            }

            update_parallel_stage_a_activity(&job, total_chunks, jobs.len(), llm_concurrency);
            let Some(joined) = jobs.join_next().await else {
                continue;
            };
            let (order, result) = match joined {
                Ok(completed) => completed,
                Err(error) => {
                    jobs.abort_all();
                    let partial_summary = GenerationSummary {
                        total_notes: notes_clone.len(),
                        total_chunks,
                        notes_with_chunks,
                        note_reports,
                        chunk_previews: Vec::new(),
                    };
                    finish_job_with_failure(
                        &job,
                        LlmFailure {
                            code: LlmFailureCode::Unknown,
                            message: format!("Knowledge extraction worker failed: {error}"),
                            retryable: false,
                            retry_after_secs: None,
                        },
                        Some(partial_summary),
                    );
                    return;
                }
            };

            match result {
                Ok(extracted) => extracted_by_order[order] = Some(extracted),
                Err(error) if is_skippable_chunk_error(&error) => {
                    record_skipped_chunk(&job, &error);
                }
                Err(error) => {
                    jobs.abort_all();
                    let partial_summary = GenerationSummary {
                        total_notes: notes_clone.len(),
                        total_chunks,
                        notes_with_chunks,
                        note_reports,
                        chunk_previews: Vec::new(),
                    };
                    finish_job_with_llm_error(&job, &error, Some(partial_summary));
                    return;
                }
            }

            let mut snapshot = job
                .snapshot
                .lock()
                .expect("preview job snapshot mutex should remain available");
            snapshot.completed_chunks += 1;
            snapshot.progress_percent = phase_percent(
                snapshot.completed_chunks,
                total_chunks,
                0,
                GRAPH_STAGE_A_END_PERCENT,
            );
        }

        let extracted_chunks = extracted_by_order.into_iter().flatten().collect::<Vec<_>>();

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
            snapshot.phase_label = Some(String::from("Building knowledge graph"));
            snapshot.activity = Some(String::from("Building the knowledge graph"));
        }
        let graph = consolidator::consolidate(extracted_chunks);
        let (graph, index) = if let Some(embedding_service) = embedding_service.as_deref() {
            if graph.entities.len() < 2 {
                log::info!(
                    "Skipping entity resolution because the consolidated graph has fewer than two entities"
                );
                let index = graph_index::build_index(&graph);
                (graph, index)
            } else {
                {
                    let mut snapshot = job
                        .snapshot
                        .lock()
                        .expect("preview job snapshot mutex should remain available");
                    snapshot.phase_label = Some(String::from("Resolving entities"));
                    snapshot.activity = Some(format!(
                        "Resolving {} entities in the knowledge graph",
                        graph.entities.len()
                    ));
                }

                let resolution_job = Arc::clone(&job);
                let resolution_config = EntityResolutionConfig {
                    verifier: super::graph_generation::entity_resolution::semantic_verifier::VerifierConfig {
                        max_concurrency: llm_concurrency,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let resolution = resolve_graph_entities_with_progress(
                    &graph,
                    embedding_service,
                    llm_arc
                        .as_deref()
                        .expect("graph generation should always have an LLM service"),
                    &resolution_config,
                    move |progress| {
                        record_entity_resolution_progress(
                            &resolution_job,
                            progress,
                            llm_concurrency,
                        );
                    },
                )
                .await;

                let resolution = match resolution {
                    Ok(result) => result,
                    Err(error) => {
                        let partial_summary = GenerationSummary {
                            total_notes: notes_clone.len(),
                            total_chunks,
                            notes_with_chunks,
                            note_reports,
                            chunk_previews: Vec::new(),
                        };
                        finish_job_with_entity_resolution_error(
                            &job,
                            &error,
                            Some(partial_summary),
                        );
                        return;
                    }
                };

                log::info!(
                    "Entity resolution completed (entities_before={}, entities_after={}, candidates={}, same_entity={}, different_entity={}, uncertain={}, merge_groups={})",
                    resolution.metrics.entity_count_before,
                    resolution.metrics.entity_count_after,
                    resolution.metrics.candidate_pair_count,
                    resolution.metrics.same_entity_count,
                    resolution.metrics.different_entity_count,
                    resolution.metrics.unresolved_pair_count,
                    resolution.metrics.merge_group_count
                );

                (resolution.graph, resolution.index)
            }
        } else {
            let index = graph_index::build_index(&graph);
            (graph, index)
        };

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

        let bundles = bundle_builder::assemble_bundles(&graph, &index);
        let bundle_count = bundles.len();

        // Transition to question generation: expose bundle-sized work while
        // preserving the 65% already earned by extraction and resolution.
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
            snapshot.progress_percent = GRAPH_ENTITY_RESOLUTION_END_PERCENT;
        }

        // ── Bounded-concurrent question generation per graph bundle ──────
        let mut ordered_previews: Vec<Option<ChunkPreview>> = vec![None; bundle_count];
        let llm = llm_arc
            .as_ref()
            .expect("graph generation should always have an LLM service");
        let mut jobs = JoinSet::new();
        let mut next_bundle_idx = 0usize;

        while next_bundle_idx < bundle_count || !jobs.is_empty() {
            if job.cancelled.load(Ordering::Relaxed) {
                jobs.abort_all();
                break;
            }

            // Do not start new requests while paused. Requests already sent to
            // the provider are allowed to finish and are still recorded.
            while job.paused.load(Ordering::Relaxed) && jobs.is_empty() {
                if job.cancelled.load(Ordering::Relaxed) {
                    break;
                }
                sleep(Duration::from_millis(PAUSE_POLL_MS)).await;
            }
            if job.cancelled.load(Ordering::Relaxed) {
                jobs.abort_all();
                break;
            }

            while !job.paused.load(Ordering::Relaxed)
                && !job.cancelled.load(Ordering::Relaxed)
                && next_bundle_idx < bundle_count
                && jobs.len() < llm_concurrency
            {
                spawn_graph_stage_b_job(&mut jobs, next_bundle_idx, &bundles[next_bundle_idx], llm);
                next_bundle_idx += 1;
            }

            update_parallel_stage_b_activity(&job, bundle_count, jobs.len(), llm_concurrency);
            let Some(joined) = jobs.join_next().await else {
                continue;
            };
            let (bundle_idx, result) = match joined {
                Ok(completed) => completed,
                Err(error) => {
                    jobs.abort_all();
                    let partial_summary = GenerationSummary {
                        total_notes: notes_clone.len(),
                        total_chunks: bundle_count,
                        notes_with_chunks,
                        note_reports,
                        chunk_previews: ordered_previews.into_iter().flatten().collect(),
                    };
                    finish_job_with_failure(
                        &job,
                        LlmFailure {
                            code: LlmFailureCode::Unknown,
                            message: format!("Question generation worker failed: {error}"),
                            retryable: false,
                            retry_after_secs: None,
                        },
                        Some(partial_summary),
                    );
                    return;
                }
            };
            let bundle = &bundles[bundle_idx];

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

            let llm_result = match result {
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
                            option_a: options.first().cloned().unwrap_or_default(),
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
                        jobs.abort_all();
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
            match bundle.question_type {
                QuestionType::Recall => snapshot.recall_mcq_generated += mcq_count,
                QuestionType::Relational => snapshot.relational_mcq_generated += mcq_count,
            }
            // Question generation occupies the remaining 35%.
            snapshot.progress_percent = phase_percent(
                snapshot.completed_chunks,
                bundle_count,
                GRAPH_ENTITY_RESOLUTION_END_PERCENT,
                100,
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
    orchestrate_notes_with_concurrency(notes, DEFAULT_LLM_CONCURRENCY).await
}

async fn orchestrate_notes_with_concurrency(
    notes: &[Note],
    llm_concurrency: usize,
) -> GenerationSummary {
    debug_assert!(llm_concurrency > 0);
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
        let semaphore = Arc::new(Semaphore::new(llm_concurrency));
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
    let llm_concurrency = configured_llm_concurrency().await?;
    Ok(orchestrate_notes_with_concurrency(&notes, llm_concurrency).await)
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

    fn embedding_settings(model: &str) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            provider: String::from("ollama"),
            base_url: String::from("http://127.0.0.1:11434"),
            selected_model: model.to_string(),
            timeout_secs: 60,
            api_key: None,
        }
    }

    #[test]
    fn missing_embedding_model_becomes_a_non_terminal_setup_warning() {
        let prepared = prepare_embedding_service_for_generation(&embedding_settings("   "))
            .expect("an unconfigured optional model should not reject generation");

        assert!(prepared.service.is_none());
        let warning = prepared
            .warning
            .expect("the skipped stage should be visible");
        assert_eq!(warning.code, LlmFailureCode::Setup);
        assert!(!warning.retryable);
        assert!(warning.message.contains("Entity resolution was skipped"));
    }

    #[test]
    fn configured_embedding_model_is_prepared_or_rejected_before_the_job() {
        let prepared = prepare_embedding_service_for_generation(&embedding_settings(
            "nomic-embed-text:latest",
        ))
        .expect("valid saved settings should prepare an embedding service");
        assert!(prepared.service.is_some());
        assert!(prepared.warning.is_none());

        let mut invalid = embedding_settings("nomic-embed-text:latest");
        invalid.provider = String::from("invalid-provider");
        let error = prepare_embedding_service_for_generation(&invalid)
            .expect_err("configured invalid settings must reject generation");
        assert!(error.contains("Unsupported embedding provider"));
    }

    #[test]
    fn phase_percent_maps_work_into_the_requested_progress_range() {
        assert_eq!(phase_percent(0, 10, 50, 64), 50);
        assert_eq!(phase_percent(5, 10, 50, 64), 57);
        assert_eq!(phase_percent(10, 10, 50, 64), 64);
        assert_eq!(phase_percent(12, 10, 50, 64), 64);
        assert_eq!(phase_percent(0, 0, 65, 100), 100);
    }

    #[test]
    fn entity_verification_advances_progress_without_regressing() {
        use crate::services::graph_generation::entity_resolution::semantic_verifier::{
            EntityMatchDecision, EntityVerificationSource,
        };

        let job = PreviewJob {
            paused: AtomicBool::new(false),
            cancelled: AtomicBool::new(false),
            snapshot: Mutex::new(GenerationProgressSnapshot {
                job_id: String::from("preview-resolution-progress"),
                total_notes: 1,
                total_chunks: 5,
                notes_with_chunks: 1,
                completed_chunks: 5,
                failed_chunks: 0,
                mcq_generated: 0,
                recall_mcq_generated: 0,
                relational_mcq_generated: 0,
                progress_percent: GRAPH_STAGE_A_END_PERCENT,
                is_paused: false,
                is_cancelled: false,
                is_finished: false,
                error: None,
                warnings: Vec::new(),
                summary: None,
                phase_label: Some(String::from("Resolving entities")),
                current_chunk: None,
                activity: None,
            }),
        };

        record_entity_resolution_progress(
            &job,
            EntityResolutionProgress::CandidatesGenerated {
                candidate_count: 100,
            },
            5,
        );
        record_entity_resolution_progress(
            &job,
            EntityResolutionProgress::VerifyingCandidates {
                completed_pairs: 50,
                total_pairs: 100,
                in_flight_pairs: 5,
                entity_id: String::from("a"),
                candidate_entity_id: String::from("b"),
                similarity: 0.9,
                decision: EntityMatchDecision::DifferentEntity,
                source: EntityVerificationSource::Llm,
            },
            5,
        );
        assert_eq!(
            job.snapshot
                .lock()
                .expect("snapshot available")
                .progress_percent,
            58
        );
        assert!(job
            .snapshot
            .lock()
            .expect("snapshot available")
            .activity
            .as_deref()
            .is_some_and(|activity| activity.contains("5 active (limit 5)")));

        record_entity_resolution_progress(
            &job,
            EntityResolutionProgress::Finalizing {
                verified_pair_count: 100,
            },
            5,
        );
        record_entity_resolution_progress(
            &job,
            EntityResolutionProgress::GeneratingEmbeddings { entity_count: 20 },
            5,
        );

        let snapshot = job.snapshot.lock().expect("snapshot available");
        assert_eq!(
            snapshot.progress_percent,
            GRAPH_ENTITY_RESOLUTION_END_PERCENT
        );
        assert!(snapshot
            .activity
            .as_deref()
            .is_some_and(|activity| activity.contains("Generating embeddings")));
    }

    #[test]
    fn embedding_provider_failures_map_to_generation_failure_codes() {
        let rate_limit =
            EntityResolutionPipelineError::Embedding(EmbeddingServiceError::HttpStatus {
                status: reqwest::StatusCode::TOO_MANY_REQUESTS,
                message: String::from("rate limited"),
            });
        let failure = entity_resolution_failure(&rate_limit);
        assert_eq!(failure.code, LlmFailureCode::RateLimited);
        assert!(failure.retryable);
        assert!(failure.message.contains("Entity resolution failed"));

        let invalid_response =
            EntityResolutionPipelineError::Embedding(EmbeddingServiceError::InvalidResponse(
                crate::services::embedding::EmbeddingValidationError::VectorCountMismatch {
                    expected: 2,
                    actual: 1,
                },
            ));
        let failure = entity_resolution_failure(&invalid_response);
        assert_eq!(failure.code, LlmFailureCode::InvalidResponse);
        assert!(!failure.retryable);
    }

    #[test]
    fn verifier_provider_failure_retains_shared_llm_classification() {
        let error = EntityResolutionPipelineError::Verification(EntityVerificationError::Llm(
            LlmServiceError::MissingApiKey {
                provider: crate::services::llm::LlmProvider::OpenAi,
            },
        ));

        let failure = entity_resolution_failure(&error);

        assert_eq!(failure.code, LlmFailureCode::Account);
        assert!(!failure.retryable);
        assert!(failure
            .message
            .starts_with("Entity resolution verifier failed:"));
    }

    #[test]
    fn retry_activity_shows_rate_limit_countdown() {
        let event = LlmRetryEvent {
            failure: LlmFailure {
                code: LlmFailureCode::RateLimited,
                message: String::from("Provider request failed"),
                retryable: true,
                retry_after_secs: None,
            },
            delay: Duration::from_secs(12),
            next_attempt: 2,
            max_attempts: 5,
            state: LlmRetryState::Waiting,
        };

        assert_eq!(
            retry_activity(&event),
            "Rate limited — retrying in 12 seconds"
        );
    }

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
            recall_mcq_generated: 0,
            relational_mcq_generated: 0,
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
        assert_eq!(json["recall_mcq_generated"], 0);
        assert_eq!(json["relational_mcq_generated"], 0);
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
                recall_mcq_generated: 0,
                relational_mcq_generated: 0,
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

        finish_job_with_llm_error(
            &job,
            &LlmServiceError::MissingApiKey {
                provider: crate::services::llm::LlmProvider::OpenRouter,
            },
            None,
        );

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
        assert!(!is_skippable_chunk_error(&LlmServiceError::MissingApiKey {
            provider: crate::services::llm::LlmProvider::OpenRouter,
        }));
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
                recall_mcq_generated: 0,
                relational_mcq_generated: 0,
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
