//! Desktop graph jobs using the shared registry, controls, and progress contracts.
//!
//! The evaluation tools call `pipeline` stages directly; this adapter starts a
//! background job and reports extraction, entity resolution, and question work.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration};

use crate::models::learning_item::GenerationPipeline;
use crate::models::model_settings::EmbeddingModelConfig;

use super::{
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
use crate::services::chunker::{self, MarkdownChunk};
use crate::services::embedding::{
    prepare_embedding_service, EmbeddingService, EmbeddingServiceError,
};
use crate::services::llm::{
    LlmFailure, LlmFailureCode, LlmService, LlmServiceError, StructuredGenerationRequest,
};
use crate::services::{database, filesystem};

use super::super::job::{
    configured_llm_concurrency, configured_llm_service, generation_metadata, next_preview_job_id,
    preview_jobs, run_with_job_control, PreviewJob,
};
use super::super::progress::{
    build_preview_text, finish_cancelled_job, finish_job_with_failure, finish_job_with_llm_error,
    is_skippable_chunk_error, llm_with_job_retry_activity, phase_percent, record_skipped_chunk,
    ChunkLlmQuestionPreview, ChunkLlmResult, ChunkPreview, GenerationProgressSnapshot,
    GenerationSummary, NoteGenerationReport,
};

const PAUSE_POLL_MS: u64 = 250;

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

type GraphStageAJobResult = (usize, Option<Result<ExtractedKnowledge, LlmServiceError>>);

/// Starts one independent graph-extraction request with owned task data.
fn spawn_graph_stage_a_job(
    jobs: &mut JoinSet<GraphStageAJobResult>,
    job: &Arc<PreviewJob>,
    order: usize,
    chunk: &MarkdownChunk,
    llm: &Arc<LlmService>,
    format_schema: &serde_json::Value,
) {
    let job = Arc::clone(job);
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
        let operation = llm.generate_json_with_retries(request, |raw_json| {
            parse_stage_a_output(raw_json, chunk_id.clone())
                .map_err(|error| LlmServiceError::InvalidOutput(error.to_string()))
        });
        let result = run_with_job_control(job, operation)
            .await
            .map(|result| result.map(|(extracted, _, _)| extracted));
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
    if snapshot.is_finished || snapshot.is_cancelled || snapshot.is_paused {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(format!(
        "Extracted {} of {total_chunks} chunks · {in_flight} active (limit {concurrency_limit})",
        snapshot.completed_chunks
    ));
}

type GraphStageBJobResult = (usize, Option<Result<GeneratedMCQ, LlmServiceError>>);

/// Starts one independent question-generation request.
fn spawn_graph_stage_b_job(
    jobs: &mut JoinSet<GraphStageBJobResult>,
    job: &Arc<PreviewJob>,
    bundle_index: usize,
    bundle: &GraphContextBundle,
    llm: &Arc<LlmService>,
) {
    let job = Arc::clone(job);
    let bundle = bundle.clone();
    let llm = Arc::clone(llm);
    jobs.spawn(async move {
        let result = run_with_job_control(job, generate_mcq(&bundle, &llm)).await;
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
    if snapshot.is_finished || snapshot.is_cancelled || snapshot.is_paused {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(format!(
        "Generated {} of {total_bundles} questions · {in_flight} active (limit {concurrency_limit})",
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
    if snapshot.is_finished || snapshot.is_cancelled || snapshot.is_paused {
        return;
    }
    snapshot.current_chunk = None;
    snapshot.activity = Some(activity);
    // Retry notifications and concurrent verifier completions may be observed
    // close together. Never let a late event move the progress ring backwards.
    snapshot.progress_percent = snapshot.progress_percent.max(progress_percent);
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
    let generation = generation_metadata(&llm_service, GenerationPipeline::Graph);
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
        default_selection: None,
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
        ready_previews: Vec::new(),
        summary: None,
        phase_label: Some(String::from("Extracting knowledge")),
        current_chunk: None,
        activity: Some(String::from("Preparing knowledge extraction")),
    };

    let job = Arc::new(PreviewJob {
        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        control_changed: Notify::new(),
        snapshot: Mutex::new(initial_snapshot),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation,
        default_generation: None,
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
                    &job,
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
                        default_selection: None,
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

            let Some(result) = result else {
                jobs.abort_all();
                break;
            };

            match result {
                Ok(extracted) => extracted_by_order[order] = Some(extracted),
                Err(error) if is_skippable_chunk_error(&error) => {
                    record_skipped_chunk(&job, &error);
                }
                Err(error) => {
                    jobs.abort_all();
                    let partial_summary = GenerationSummary {
                        default_selection: None,
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
                    verifier: super::entity_resolution::semantic_verifier::VerifierConfig {
                        max_concurrency: llm_concurrency,
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let resolution_operation = resolve_graph_entities_with_progress(
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
                );
                let Some(resolution) =
                    run_with_job_control(Arc::clone(&job), resolution_operation).await
                else {
                    finish_cancelled_job(&job);
                    return;
                };

                let resolution = match resolution {
                    Ok(result) => result,
                    Err(error) => {
                        let partial_summary = GenerationSummary {
                            default_selection: None,
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

            // Do not schedule new work while paused. Existing workers retain
            // their request futures and resume polling them after the signal.
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
                spawn_graph_stage_b_job(
                    &mut jobs,
                    &job,
                    next_bundle_idx,
                    &bundles[next_bundle_idx],
                    llm,
                );
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
                        default_selection: None,
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

            let Some(result) = result else {
                jobs.abort_all();
                break;
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
                        items: Vec::new(),
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
                            items: Vec::new(),
                            questions: Vec::new(),
                            error: Some(err.to_failure().message),
                        }
                    } else {
                        jobs.abort_all();
                        let partial_summary = GenerationSummary {
                            default_selection: None,
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
            let ready_preview = ordered_previews[bundle_idx]
                .as_ref()
                .filter(|preview| !preview.llm_result.questions.is_empty())
                .cloned();

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
            if let Some(preview) = ready_preview {
                snapshot.ready_previews.push(preview);
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
            default_selection: None,
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

#[cfg(test)]
mod tests;
