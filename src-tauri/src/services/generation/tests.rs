use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio::time::{sleep, Duration};

use super::job::{run_with_job_control, PreviewJob};
use super::progress::{
    chunk_preview_is_ready, finish_job_with_llm_error, is_skippable_chunk_error, phase_percent,
    record_skipped_chunk, retry_activity,
};
use super::*;
use crate::models::learning_item::{
    GeneratedFlashcard, GeneratedItem, GenerationMetadata, GenerationPipeline, LearningItemDraft,
    SourceReference,
};
use crate::models::note::Note;

use crate::services::llm::{
    LlmFailure, LlmFailureCode, LlmRetryEvent, LlmRetryState, LlmServiceError,
};

pub(super) fn test_generation() -> GenerationMetadata {
    GenerationMetadata {
        model: Some(String::from("test-model")),
        provider: Some(String::from("ollama")),
        pipeline: Some(GenerationPipeline::Chunk),
        generated_at: Some(String::from("2026-09-09T12:00:00Z")),
    }
}

#[test]
fn flashcard_only_chunk_preview_is_ready() {
    let preview = ChunkPreview {
        note_path: String::from("notes/search.md"),
        note_title: String::from("Search"),
        heading: String::from("Binary search"),
        section_index: 0,
        chunk_index: 0,
        start_line: 1,
        end_line: 5,
        char_count: 120,
        preview_text: String::from("Binary search halves the search interval each step."),
        llm_result: ChunkLlmResult {
            status: String::from("ok"),
            key_points: vec![String::from("Binary search time complexity")],
            items: vec![LearningItemDraft {
                draft_id: String::from("draft_test_1"),
                generation: test_generation(),
                source: SourceReference {
                    note_path: String::from("notes/search.md"),
                    start_line: 1,
                    end_line: 5,
                    knowledge_point: String::from("Binary search time complexity"),
                },
                content: GeneratedItem {
                    knowledge_point_id: String::from("kp_1"),
                    target: String::from("Binary search time complexity"),
                    answer: String::from("O(log n)"),
                    explanation: Some(String::from(
                        "Each step halves the remaining search interval.",
                    )),
                    flashcard: GeneratedFlashcard {
                        prompt: String::from(
                            "What is the time complexity of binary search on a sorted array?",
                        ),
                    },
                    mcq: None,
                    mcq_omission_reason: Some(String::from(
                        "A direct recall flashcard is sufficient for this target.",
                    )),
                },
            }],
            questions: Vec::new(),
            error: None,
        },
    };

    assert!(chunk_preview_is_ready(&preview));
}

fn controllable_test_job() -> Arc<PreviewJob> {
    Arc::new(PreviewJob {
        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        control_changed: Notify::new(),
        snapshot: Mutex::new(GenerationProgressSnapshot {
            default_selection: None,
            job_id: String::from("preview-control-test"),
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
            is_finished: false,
            error: None,
            warnings: Vec::new(),
            ready_previews: Vec::new(),
            summary: None,
            phase_label: None,
            current_chunk: None,
            activity: None,
        }),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation: test_generation(),
        default_generation: None,
    })
}

#[tokio::test]
async fn paused_job_holds_ready_work_until_resumed() {
    let job = controllable_test_job();
    let (complete_work, work_completed) = tokio::sync::oneshot::channel();
    let worker_job = Arc::clone(&job);
    let worker = tokio::spawn(async move {
        run_with_job_control(worker_job, async move {
            work_completed.await.expect("work signal should be sent");
            42
        })
        .await
    });

    job.paused.store(true, Ordering::Relaxed);
    job.control_changed.notify_waiters();
    complete_work
        .send(())
        .expect("work should still be waiting");
    sleep(Duration::from_millis(20)).await;

    assert!(!worker.is_finished());

    job.paused.store(false, Ordering::Relaxed);
    job.control_changed.notify_waiters();
    assert_eq!(worker.await.expect("worker should finish"), Some(42));
}

#[tokio::test]
async fn cancelled_job_drops_in_flight_work_promptly() {
    let job = controllable_test_job();
    let worker_job = Arc::clone(&job);
    let worker = tokio::spawn(async move {
        run_with_job_control(worker_job, std::future::pending::<()>()).await
    });

    tokio::task::yield_now().await;
    job.cancelled.store(true, Ordering::Relaxed);
    job.control_changed.notify_waiters();

    let result = tokio::time::timeout(Duration::from_millis(100), worker)
        .await
        .expect("cancelled worker should stop promptly")
        .expect("worker should not panic");
    assert!(result.is_none());
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
        default_selection: None,
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
        ready_previews: Vec::new(),
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
fn graph_progress_and_summary_omit_default_selection_configuration() {
    let mut job = controllable_test_job();
    Arc::get_mut(&mut job).unwrap().generation.pipeline = Some(GenerationPipeline::Graph);
    assert!(job.default_generation.is_none());
    let serialized = serde_json::to_value(job.snapshot.lock().unwrap().clone()).unwrap();
    assert!(serialized.get("default_selection").is_none());
    let summary = GenerationSummary {
        default_selection: None,
        total_notes: 0,
        total_chunks: 0,
        notes_with_chunks: 0,
        note_reports: Vec::new(),
        chunk_previews: Vec::new(),
    };
    assert!(serde_json::to_value(summary)
        .unwrap()
        .get("default_selection")
        .is_none());
}

#[test]
fn terminal_llm_error_finishes_job_and_records_failure() {
    let job = PreviewJob {
        paused: AtomicBool::new(true),
        cancelled: AtomicBool::new(false),
        control_changed: Notify::new(),
        snapshot: Mutex::new(GenerationProgressSnapshot {
            default_selection: None,
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
            ready_previews: Vec::new(),
            summary: None,
            phase_label: Some(String::from("Generating questions")),
            current_chunk: Some(2),
            activity: Some(String::from("Generating question 2 of 2")),
        }),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation: test_generation(),
        default_generation: None,
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
        control_changed: Notify::new(),
        snapshot: Mutex::new(GenerationProgressSnapshot {
            default_selection: None,
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
            ready_previews: Vec::new(),
            summary: None,
            phase_label: None,
            current_chunk: Some(1),
            activity: Some(String::from("Generating questions for chunk 1 of 2")),
        }),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation: test_generation(),
        default_generation: None,
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
