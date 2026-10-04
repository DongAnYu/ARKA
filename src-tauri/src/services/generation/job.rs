//! Shared generation job registry, captured metadata, and pause/cancel controls.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::sync::Notify;

use crate::models::learning_item::{GenerationMetadata, GenerationPipeline};
use crate::services::database;
use crate::services::llm::LlmService;

use super::default::candidates::DefaultGenerationJob;
use super::progress::GenerationProgressSnapshot;

pub(super) fn configured_llm_service() -> Result<Arc<LlmService>, String> {
    LlmService::from_runtime_or_env()
        .map(Arc::new)
        .map_err(|err| {
            log::warn!("Generation cannot start because LLM configuration is unavailable: {err}");
            err.to_failure().message
        })
}

/// Loads the single concurrency limit shared by every application LLM stage.
pub(super) async fn configured_llm_concurrency() -> Result<usize, String> {
    let model_config = database::load_model_config()
        .await
        .map_err(|error| format!("Failed to load LLM concurrency setting: {error}"))?;
    let concurrency = model_config.validated_llm_concurrency()?;
    log::info!("Generation LLM concurrency resolved (limit={concurrency})");
    Ok(concurrency)
}

static PREVIEW_JOBS: OnceLock<Mutex<HashMap<String, Arc<PreviewJob>>>> = OnceLock::new();
static NEXT_PREVIEW_JOB_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_LEARNING_ITEM_DRAFT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(super) struct PreviewJob {
    pub(super) paused: AtomicBool,
    pub(super) cancelled: AtomicBool,
    pub(super) control_changed: Notify,
    pub(super) snapshot: Mutex<GenerationProgressSnapshot>,
    pub(super) saved_draft_ids: Mutex<std::collections::HashSet<String>>,
    pub(super) generation: GenerationMetadata,
    /// Default-only options and authoritative Stage A sources, retained for
    /// the job lifetime. Graph jobs never construct this state.
    pub(super) default_generation: Option<Arc<DefaultGenerationJob>>,
}

pub(super) fn preview_jobs() -> &'static Mutex<HashMap<String, Arc<PreviewJob>>> {
    PREVIEW_JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(super) fn next_preview_job_id() -> String {
    format!(
        "preview-{}",
        NEXT_PREVIEW_JOB_ID.fetch_add(1, Ordering::Relaxed)
    )
}

/// Polls one unit of generation work only while its job is running.
///
/// Pausing keeps the operation future alive so it can resume without losing
/// local state. Cancelling drops the operation immediately, which closes any
/// in-flight HTTP response future owned by this process.
pub(super) async fn run_with_job_control<T>(
    job: Arc<PreviewJob>,
    operation: impl Future<Output = T>,
) -> Option<T> {
    tokio::pin!(operation);

    loop {
        let control_changed = job.control_changed.notified();
        tokio::pin!(control_changed);
        control_changed.as_mut().enable();

        if job.cancelled.load(Ordering::Relaxed) {
            return None;
        }

        if job.paused.load(Ordering::Relaxed) {
            control_changed.await;
            continue;
        }

        tokio::select! {
            biased;
            _ = &mut control_changed => continue,
            result = &mut operation => return Some(result),
        }
    }
}

pub(super) fn preview_job(job_id: &str) -> Result<Arc<PreviewJob>, String> {
    preview_jobs()
        .lock()
        .map_err(|_| String::from("Preview job state is unavailable."))?
        .get(job_id)
        .cloned()
        .ok_or_else(|| format!("Preview job '{job_id}' was not found."))
}

pub fn generation_metadata_for_job(job_id: &str) -> Result<GenerationMetadata, String> {
    Ok(preview_job(job_id)?.generation.clone())
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
    drop(snapshot);
    job.control_changed.notify_waiters();

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
    let mut snapshot = job
        .snapshot
        .lock()
        .map_err(|_| String::from("Preview job snapshot is unavailable."))?;
    snapshot.is_cancelled = true;
    snapshot.is_paused = false;
    snapshot.activity = Some(String::from("Cancelling generation"));
    drop(snapshot);
    job.control_changed.notify_waiters();

    Ok(())
}

pub(super) fn next_learning_item_draft_id() -> String {
    format!(
        "draft-{}",
        NEXT_LEARNING_ITEM_DRAFT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

pub(super) fn generation_metadata(
    llm_service: &LlmService,
    pipeline: GenerationPipeline,
) -> GenerationMetadata {
    GenerationMetadata {
        model: Some(llm_service.model().to_string()),
        provider: Some(llm_service.provider_name().to_string()),
        pipeline: Some(pipeline),
        generated_at: Some(chrono::Utc::now().to_rfc3339()),
    }
}
