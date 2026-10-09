use super::*;
use crate::services::generation::tests::test_generation;

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
    let prepared =
        prepare_embedding_service_for_generation(&embedding_settings("nomic-embed-text:latest"))
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
fn entity_verification_advances_progress_without_regressing() {
    use crate::services::generation::graph::entity_resolution::semantic_verifier::{
        EntityMatchDecision, EntityVerificationSource,
    };

    let job = PreviewJob {
        paused: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        control_changed: Notify::new(),
        snapshot: Mutex::new(GenerationProgressSnapshot {
            default_selection: None,
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
            ready_previews: Vec::new(),
            summary: None,
            phase_label: Some(String::from("Resolving entities")),
            current_chunk: None,
            activity: None,
        }),
        saved_draft_ids: Mutex::new(std::collections::HashSet::new()),
        generation: test_generation(),
        default_generation: None,
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
    let rate_limit = EntityResolutionPipelineError::Embedding(EmbeddingServiceError::HttpStatus {
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
