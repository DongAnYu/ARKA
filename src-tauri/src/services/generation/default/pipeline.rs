//! Whole-note Default orchestration. The graph pipeline does not use this module.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::task::JoinSet;

use crate::models::generation_options::GenerationOptions;
use crate::models::learning_item::{GenerationMetadata, LearningItemDraft};
use crate::models::note::Note;
use crate::services::chunker::{self, MarkdownChunk};
use crate::services::llm::default_generation_schema::StageAKeyPointsOutput;
use crate::services::llm::{LlmFailure, LlmFailureCode, LlmService, LlmServiceError};

use super::super::job::{next_learning_item_draft_id, run_with_job_control, PreviewJob};
use super::super::progress::{
    build_preview_text, chunk_preview_is_ready, finish_cancelled_job, finish_job_with_failure,
    finish_job_with_llm_error, is_skippable_chunk_error, phase_percent, record_skipped_chunk,
    skipped_chunk_preview, ChunkLlmResult, ChunkPreview, GenerationProgressSnapshot,
    GenerationSummary, NoteGenerationReport,
};
use super::candidates::{DefaultGenerationJob, KnowledgeCandidate};
use super::selection::{select_candidates, CandidateAssessment, DefaultSelectionReport};

const EXTRACTION_END: u8 = 30;
const ASSESSMENT_END: u8 = 70;
const SELECTION_END: u8 = 75;

pub(super) fn prepare_job(
    job_id: String,
    notes: &[Note],
    options: GenerationOptions,
    generation: GenerationMetadata,
) -> (Arc<PreviewJob>, Vec<NoteGenerationReport>) {
    let mut chunks = Vec::new();
    let mut note_reports = Vec::new();
    for note in notes {
        let note_chunks = chunker::chunk_note(note);
        note_reports.push(NoteGenerationReport {
            note_path: note.path.clone(),
            note_title: note.title.clone(),
            total_chunks: note_chunks.len(),
        });
        chunks.extend(note_chunks);
    }
    let total_chunks = chunks.len();
    let state = Arc::new(DefaultGenerationJob::with_notes(options, chunks, notes));
    let job = Arc::new(PreviewJob {
        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        control_changed: Notify::new(),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation,
        default_generation: Some(state),
        snapshot: Mutex::new(GenerationProgressSnapshot {
            job_id,
            total_notes: notes.len(),
            total_chunks,
            notes_with_chunks: note_reports
                .iter()
                .filter(|note| note.total_chunks > 0)
                .count(),
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
            ready_previews: Vec::new(),
            summary: None,
            phase_label: Some("Extracting knowledge".into()),
            current_chunk: None,
            activity: Some("Preparing knowledge extraction".into()),
            default_selection: Some(DefaultSelectionReport::new(options)),
        }),
    });
    (job, note_reports)
}

#[derive(Debug)]
enum PipelineError {
    Cancelled,
    Llm(LlmServiceError),
    State(String),
}

impl From<LlmServiceError> for PipelineError {
    fn from(error: LlmServiceError) -> Self {
        Self::Llm(error)
    }
}

/// Bounded task creation as well as bounded HTTP concurrency. Both the scheduler
/// and each owned request obey pause/cancel, including between pipeline phases.
async fn bounded<T, R, F, Fut, C>(
    job: &Arc<PreviewJob>,
    items: Vec<T>,
    concurrency: usize,
    operation: F,
    mut complete: C,
) -> Result<(), PipelineError>
where
    T: Send + 'static,
    R: Send + 'static,
    F: Fn(T) -> Fut,
    Fut: Future<Output = R> + Send + 'static,
    C: FnMut(R) -> Result<(), PipelineError>,
{
    let mut pending = items.into_iter().peekable();
    let mut tasks = JoinSet::new();
    while pending.peek().is_some() || !tasks.is_empty() {
        run_with_job_control(Arc::clone(job), async {})
            .await
            .ok_or(PipelineError::Cancelled)?;
        while tasks.len() < concurrency
            && !job.paused.load(Ordering::Relaxed)
            && !job.cancelled.load(Ordering::Relaxed)
        {
            let Some(item) = pending.next() else {
                break;
            };
            let request = operation(item);
            let worker_job = Arc::clone(job);
            tasks.spawn(async move { run_with_job_control(worker_job, request).await });
        }
        let joined = run_with_job_control(Arc::clone(job), tasks.join_next())
            .await
            .ok_or(PipelineError::Cancelled)?;
        if let Some(joined) = joined {
            let result = joined
                .map_err(|error| {
                    PipelineError::State(format!("Default generation worker failed: {error}"))
                })?
                .ok_or(PipelineError::Cancelled)?;
            complete(result)?;
        }
    }
    Ok(())
}

fn set_phase(job: &PreviewJob, label: &str, percent: u8) {
    let mut snapshot = job
        .snapshot
        .lock()
        .expect("preview snapshot remains available");
    snapshot.phase_label = Some(label.into());
    snapshot.current_chunk = None;
    snapshot.activity = Some(label.into());
    snapshot.progress_percent = snapshot.progress_percent.max(percent);
}

fn update_phase_progress(
    snapshot: &mut GenerationProgressSnapshot,
    completed: usize,
    total: usize,
    start_percent: u8,
    end_percent: u8,
    activity: String,
) {
    snapshot.progress_percent =
        snapshot
            .progress_percent
            .max(phase_percent(completed, total, start_percent, end_percent));
    snapshot.activity = Some(activity);
}

fn empty_result(status: &str) -> ChunkLlmResult {
    ChunkLlmResult {
        status: status.into(),
        key_points: Vec::new(),
        items: Vec::new(),
        questions: Vec::new(),
        error: None,
    }
}

pub(super) fn preview(chunk: &MarkdownChunk) -> ChunkPreview {
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
        llm_result: empty_result("pending"),
    }
}

async fn assess_one(
    state: Arc<DefaultGenerationJob>,
    llm: Arc<LlmService>,
    candidate: KnowledgeCandidate,
) -> Result<(KnowledgeCandidate, CandidateAssessment), LlmServiceError> {
    let context = state
        .note_context(&candidate)
        .map_err(LlmServiceError::InvalidOutput)?;
    let comparisons = state
        .chunk_candidates(candidate.source_chunk_index)
        .map_err(LlmServiceError::InvalidOutput)?
        .into_iter()
        .filter(|point| point.id != candidate.id)
        .map(|point| point.knowledge_point)
        .collect::<Vec<_>>();
    let mut used_source_context = false;
    let mut assessment = llm
        .assess_knowledge_candidate(
            context,
            &candidate,
            &comparisons,
            state.options().purpose(),
            None,
        )
        .await;
    if assessment
        .as_ref()
        .is_ok_and(|assessment| assessment.requires_source_context)
    {
        used_source_context = true;
        // At most one recovery request, using the same provider/retry contract.
        assessment = llm
            .assess_knowledge_candidate(
                context,
                &candidate,
                &comparisons,
                state.options().purpose(),
                Some(&state.chunks()[candidate.source_chunk_index].content),
            )
            .await;
    }
    let result = match assessment {
        Ok(assessment) if assessment.requires_source_context => {
            Err("Source context remained insufficient after one reassessment".into())
        }
        Ok(assessment) => Ok(assessment),
        Err(error) if is_skippable_chunk_error(&error) => Err(error.to_failure().message),
        Err(error) => return Err(error),
    };
    Ok((
        candidate,
        CandidateAssessment {
            result,
            used_source_context,
        },
    ))
}

async fn generate_selected(
    state: Arc<DefaultGenerationJob>,
    llm: Arc<LlmService>,
    generation: GenerationMetadata,
    selected: Vec<KnowledgeCandidate>,
) -> Result<Vec<LearningItemDraft>, LlmServiceError> {
    let chunk_index = selected
        .first()
        .ok_or_else(|| LlmServiceError::InvalidOutput("Empty Stage B allocation".into()))?
        .source_chunk_index;
    if selected
        .iter()
        .any(|candidate| candidate.source_chunk_index != chunk_index)
    {
        return Err(LlmServiceError::InvalidOutput(
            "Stage B allocation spans source chunks".into(),
        ));
    }
    let points = selected
        .iter()
        .map(|candidate| candidate.knowledge_point.clone())
        .collect::<Vec<_>>();
    let generated = llm
        .generate_stage_b_learning_items(&state.chunks()[chunk_index].content, &points)
        .await?;
    generated
        .items
        .into_iter()
        .map(|content| {
            let source = state
                .source_for_selected_point(&selected, &content.knowledge_point_id)
                .map_err(LlmServiceError::InvalidOutput)?;
            Ok(LearningItemDraft {
                draft_id: next_learning_item_draft_id(),
                generation: generation.clone(),
                source,
                content,
            })
        })
        .collect()
}

async fn run_stages(
    job: &Arc<PreviewJob>,
    llm: &Arc<LlmService>,
    concurrency: usize,
    previews: &mut [ChunkPreview],
) -> Result<(), PipelineError> {
    let state = job
        .default_generation
        .as_ref()
        .expect("Default job retains state");

    extract_candidates(job, state, llm, concurrency, previews).await?;
    let candidates = state.candidates().map_err(PipelineError::State)?;
    assess_candidates(job, state, llm, concurrency, &candidates).await?;
    let selected = select_learning_targets(job, state, &candidates, previews).await?;
    generate_learning_items(job, state, llm, concurrency, selected, previews).await
}

async fn extract_candidates(
    job: &Arc<PreviewJob>,
    state: &Arc<DefaultGenerationJob>,
    llm: &Arc<LlmService>,
    concurrency: usize,
    previews: &mut [ChunkPreview],
) -> Result<(), PipelineError> {
    let total_chunks = state.chunks().len();
    set_phase(job, "Extracting knowledge", 0);
    bounded(
        job,
        (0..total_chunks).collect(),
        concurrency,
        |index| {
            let state = Arc::clone(state);
            let llm = Arc::clone(llm);
            async move {
                (
                    index,
                    llm.generate_stage_a_key_points(&state.chunks()[index].content)
                        .await,
                )
            }
        },
        |(index, result)| {
            record_extraction_completed(
                job,
                state,
                &mut previews[index],
                index,
                total_chunks,
                result,
            )
        },
    )
    .await
}

async fn assess_candidates(
    job: &Arc<PreviewJob>,
    state: &Arc<DefaultGenerationJob>,
    llm: &Arc<LlmService>,
    concurrency: usize,
    candidates: &[KnowledgeCandidate],
) -> Result<(), PipelineError> {
    let total_candidates = candidates.len();
    job.snapshot
        .lock()
        .unwrap()
        .default_selection
        .as_mut()
        .unwrap()
        .candidate_count = total_candidates;
    set_phase(job, "Assessing importance", EXTRACTION_END);
    bounded(
        job,
        candidates.to_vec(),
        concurrency,
        |candidate| assess_one(Arc::clone(state), Arc::clone(llm), candidate),
        |result| {
            let (candidate, outcome) = result?;
            record_assessment_completed(job, state, &candidate, outcome, total_candidates)
        },
    )
    .await
}

async fn select_learning_targets(
    job: &Arc<PreviewJob>,
    state: &Arc<DefaultGenerationJob>,
    candidates: &[KnowledgeCandidate],
    previews: &mut [ChunkPreview],
) -> Result<BTreeMap<usize, Vec<KnowledgeCandidate>>, PipelineError> {
    set_phase(job, "Selecting concepts", ASSESSMENT_END);
    let assessments = state.assessments().map_err(PipelineError::State)?;
    let selection = run_with_job_control(Arc::clone(job), async {
        select_candidates(candidates, &assessments, state.options())
    })
    .await
    .ok_or(PipelineError::Cancelled)?;
    record_selection_completed(job, selection.report);
    for chunk in previews
        .iter_mut()
        .filter(|chunk| chunk.llm_result.status == "extracted")
    {
        chunk.llm_result.status = "not_selected".into();
    }
    Ok(selection.by_chunk)
}

async fn generate_learning_items(
    job: &Arc<PreviewJob>,
    state: &Arc<DefaultGenerationJob>,
    llm: &Arc<LlmService>,
    concurrency: usize,
    selected: BTreeMap<usize, Vec<KnowledgeCandidate>>,
    previews: &mut [ChunkPreview],
) -> Result<(), PipelineError> {
    set_phase(job, "Generating learning items", SELECTION_END);
    let allocations = selected.into_iter().collect::<Vec<_>>();
    bounded(
        job,
        allocations,
        concurrency,
        |(index, selected)| {
            let state = Arc::clone(state);
            let llm = Arc::clone(llm);
            let generation = job.generation.clone();
            async move {
                let allocated = selected.len();
                let result = generate_selected(state, llm, generation, selected).await;
                (index, allocated, result)
            }
        },
        |(index, allocated, result)| {
            record_generation_completed(job, &mut previews[index], allocated, result)
        },
    )
    .await
}

fn record_extraction_completed(
    job: &PreviewJob,
    state: &DefaultGenerationJob,
    preview: &mut ChunkPreview,
    index: usize,
    total_chunks: usize,
    result: Result<StageAKeyPointsOutput, LlmServiceError>,
) -> Result<(), PipelineError> {
    match result {
        Ok(extracted) => {
            let points = extracted
                .key_points
                .into_iter()
                .map(|point| point.knowledge_point)
                .collect::<Vec<_>>();
            state
                .register_stage_a(index, &points)
                .map_err(PipelineError::State)?;
            preview.llm_result.status = if points.is_empty() {
                "no_content"
            } else {
                "extracted"
            }
            .into();
            preview.llm_result.key_points = points;
            let mut snapshot = job.snapshot.lock().unwrap();
            snapshot
                .default_selection
                .as_mut()
                .unwrap()
                .extracted_chunks += 1;
        }
        Err(error) if is_skippable_chunk_error(&error) => {
            record_skipped_chunk(job, &error);
            *preview = skipped_chunk_preview(&state.chunks()[index], &error);
            let mut snapshot = job.snapshot.lock().unwrap();
            snapshot
                .default_selection
                .as_mut()
                .unwrap()
                .extraction_failed_chunks += 1;
        }
        Err(error) => return Err(error.into()),
    }
    let mut snapshot = job.snapshot.lock().unwrap();
    snapshot.completed_chunks += 1;
    let completed = snapshot.completed_chunks;
    update_phase_progress(
        &mut snapshot,
        completed,
        total_chunks,
        0,
        EXTRACTION_END,
        format!("Processed {completed} of {total_chunks} source chunks"),
    );
    Ok(())
}

fn record_assessment_completed(
    job: &PreviewJob,
    state: &DefaultGenerationJob,
    candidate: &KnowledgeCandidate,
    outcome: CandidateAssessment,
    total_candidates: usize,
) -> Result<(), PipelineError> {
    if let Err(reason) = &outcome.result {
        let mut snapshot = job.snapshot.lock().unwrap();
        snapshot.warnings.push(LlmFailure {
            code: LlmFailureCode::InvalidResponse,
            message: format!(
                "A concept in '{}' was excluded from selection: {reason}",
                candidate.heading
            ),
            retryable: false,
            retry_after_secs: None,
        });
    }
    let mut snapshot = job.snapshot.lock().unwrap();
    let report = snapshot.default_selection.as_mut().unwrap();
    report.assessed_count += 1;
    report.source_context_reassessments += usize::from(outcome.used_source_context);
    if outcome.result.is_err() {
        report.unresolved_assessments += 1;
    }
    let done = report.assessed_count;
    state
        .attach_assessment(candidate, outcome)
        .map_err(PipelineError::State)?;
    update_phase_progress(
        &mut snapshot,
        done,
        total_candidates,
        EXTRACTION_END,
        ASSESSMENT_END,
        format!("Assessed {done} of {total_candidates} concepts"),
    );
    Ok(())
}

fn record_selection_completed(job: &PreviewJob, mut report: DefaultSelectionReport) {
    let mut snapshot = job.snapshot.lock().unwrap();
    let previous = snapshot.default_selection.as_ref().unwrap();
    report.extracted_chunks = previous.extracted_chunks;
    report.extraction_failed_chunks = previous.extraction_failed_chunks;
    snapshot.default_selection = Some(report);
    snapshot.progress_percent = snapshot.progress_percent.max(SELECTION_END);
}

fn record_generation_completed(
    job: &PreviewJob,
    preview: &mut ChunkPreview,
    allocated: usize,
    result: Result<Vec<LearningItemDraft>, LlmServiceError>,
) -> Result<(), PipelineError> {
    match result {
        Ok(items) => {
            let count = items.len();
            let mcqs = items
                .iter()
                .filter(|item| item.content.mcq.is_some())
                .count();
            preview.llm_result.status = if count == 0 { "omitted" } else { "ok" }.into();
            preview.llm_result.items = items;
            let mut snapshot = job.snapshot.lock().unwrap();
            snapshot.mcq_generated += mcqs;
            if chunk_preview_is_ready(preview) {
                snapshot.ready_previews.push(preview.clone());
            }
            let report = snapshot.default_selection.as_mut().unwrap();
            report.generated_count += count;
            report.generation_omitted_count += allocated.saturating_sub(count);
        }
        Err(error) if is_skippable_chunk_error(&error) => {
            record_skipped_chunk(job, &error);
            preview.llm_result.status = "skipped".into();
            preview.llm_result.error = Some(error.to_failure().message);
            let mut snapshot = job.snapshot.lock().unwrap();
            let report = snapshot.default_selection.as_mut().unwrap();
            report.generation_failed_chunks += 1;
            report.generation_failed_targets += allocated;
        }
        Err(error) => return Err(error.into()),
    }
    let mut snapshot = job.snapshot.lock().unwrap();
    let report = snapshot.default_selection.as_mut().unwrap();
    report.generation_completed_chunks += 1;
    let done = report.generation_completed_chunks;
    let total = report.generation_total_chunks;
    let generated = report.generated_count;
    update_phase_progress(
        &mut snapshot,
        done,
        total,
        SELECTION_END,
        99,
        format!(
            "Generated {generated} learning items · processed {done} of {total} selected chunks"
        ),
    );
    Ok(())
}

pub(super) async fn run(
    job: Arc<PreviewJob>,
    llm: Arc<LlmService>,
    concurrency: usize,
    note_reports: Vec<NoteGenerationReport>,
) {
    assert!(concurrency > 0);
    let state = job
        .default_generation
        .as_ref()
        .expect("Default job retains state");
    let mut previews = state.chunks().iter().map(preview).collect::<Vec<_>>();
    let result = run_stages(&job, &llm, concurrency, &mut previews).await;
    if matches!(result, Err(PipelineError::Cancelled)) || job.cancelled.load(Ordering::Relaxed) {
        finish_cancelled_job(&job);
        return;
    }
    let summary = {
        let mut snapshot = job.snapshot.lock().unwrap();
        let report = snapshot.default_selection.as_mut().unwrap();
        report.explain_shortfall();
        GenerationSummary {
            total_notes: snapshot.total_notes,
            total_chunks: snapshot.total_chunks,
            notes_with_chunks: snapshot.notes_with_chunks,
            note_reports,
            chunk_previews: previews,
            default_selection: snapshot.default_selection.clone(),
        }
    };
    match result {
        Err(PipelineError::Llm(error)) => finish_job_with_llm_error(&job, &error, Some(summary)),
        Err(PipelineError::State(message)) => finish_job_with_failure(
            &job,
            LlmFailure {
                code: LlmFailureCode::Unknown,
                message,
                retryable: false,
                retry_after_secs: None,
            },
            Some(summary),
        ),
        Err(PipelineError::Cancelled) => unreachable!(),
        Ok(()) => {
            let mut snapshot = job.snapshot.lock().unwrap();
            snapshot.summary = Some(summary);
            snapshot.is_finished = true;
            snapshot.is_paused = false;
            snapshot.current_chunk = None;
            snapshot.activity = None;
            snapshot.progress_percent = 100;
        }
    }
}

#[cfg(test)]
mod tests;
