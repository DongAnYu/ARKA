//! End-to-end orchestration for semantic entity resolution.
//!
//! This module connects the otherwise independent, testable stages in their
//! required order. It owns no provider-specific logic; embeddings and semantic
//! verification continue to use ARKA's configured provider-neutral services.
//!
//! ```text
//! PropositionGraph
//!       ↓ build_entity_contexts
//! EntityContext
//!       ↓ generate_entity_context_embeddings
//! EntityEmbedding
//!       ↓ generate_entity_candidates
//! EntityCandidate
//!       ↓ verify_entity_candidates
//! VerifiedEntityCandidate
//!       ↓ build_entity_merge_plan
//! EntityMergePlan
//!       ↓ rewrite_graph_with_entity_merges
//! PropositionGraph
//!       ↓ build_index
//! GraphIndex
//! ```

use std::error::Error;
use std::fmt;
use std::time::Instant;

use crate::services::embedding::{EmbeddingService, EmbeddingServiceError};
use crate::services::graph_generation::graph_index::{build_index, GraphIndex};
use crate::services::graph_generation::types::PropositionGraph;
use crate::services::llm::LlmService;

use super::candidate_generator::{
    generate_entity_candidates, CandidateConfig, CandidateGenerationError, EntityCandidate,
};
use super::context_builder::build_entity_contexts;
use super::embedding_generator::generate_entity_context_embeddings;
use super::graph_rewriter::{rewrite_graph_with_entity_merges, GraphRewriteError};
use super::merge_planner::{build_entity_merge_plan, EntityMergePlan, MergePlanningError};
use super::semantic_verifier::{
    verify_entity_candidates_with_progress, EntityMatchDecision, EntityVerificationError,
    EntityVerificationSource, VerifiedEntityCandidate, VerifierConfig,
};

/// Default loose retrieval threshold shared by the application and eval tools.
/// Similarity selects verifier candidates only; it never authorizes a merge.
pub const DEFAULT_ENTITY_CANDIDATE_SIMILARITY: f32 = 0.9;

/// Settings for one complete entity-resolution run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityResolutionConfig {
    /// Maximum graph knowledge points included in each entity's evidence.
    pub max_points_per_entity: usize,
    /// Loose embedding retrieval settings; these never authorize a merge.
    pub candidates: CandidateConfig,
    /// Structured semantic-verifier output settings.
    pub verifier: VerifierConfig,
}

impl Default for EntityResolutionConfig {
    fn default() -> Self {
        Self {
            max_points_per_entity: 3,
            candidates: CandidateConfig {
                minimum_similarity: DEFAULT_ENTITY_CANDIDATE_SIMILARITY,
                max_candidates_per_entity: 3,
            },
            verifier: VerifierConfig::default(),
        }
    }
}

/// Counts recorded for evaluation, tuning, and pipeline diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EntityResolutionMetrics {
    pub entity_count_before: usize,
    pub entity_count_after: usize,
    pub candidate_pair_count: usize,
    pub same_entity_count: usize,
    pub different_entity_count: usize,
    pub unresolved_pair_count: usize,
    pub merge_group_count: usize,
}

/// Completed entity-resolution output plus inspectable intermediate decisions.
#[derive(Debug)]
pub struct EntityResolutionResult {
    /// Rewritten graph used by downstream bundle and question generation.
    pub graph: PropositionGraph,
    /// Fresh index built only after every stable-ID rewrite is complete.
    pub index: GraphIndex,
    /// Loose embedding shortlist, retained for threshold evaluation.
    pub candidates: Vec<EntityCandidate>,
    /// Semantic decisions and explanations, retained for auditing and evals.
    pub verified_candidates: Vec<VerifiedEntityCandidate>,
    /// Deterministic plan that produced the rewritten graph.
    pub merge_plan: EntityMergePlan,
    /// Aggregate run counts without embedding-vector storage.
    pub metrics: EntityResolutionMetrics,
}

/// Live milestones from one entity-resolution run.
///
/// These events expose expensive provider-backed work without coupling the
/// resolver to the application's progress-snapshot implementation.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityResolutionProgress {
    /// Entity contexts are about to be embedded in one provider batch.
    GeneratingEmbeddings { entity_count: usize },
    /// Embedding retrieval has produced the semantic-verification shortlist.
    CandidatesGenerated { candidate_count: usize },
    /// One embedding candidate selected for semantic verification.
    CandidateSelected {
        position: usize,
        total_pairs: usize,
        entity_id: String,
        candidate_entity_id: String,
        similarity: f32,
    },
    /// A verifier request completed and the bounded worker pool was refilled.
    VerifyingCandidates {
        completed_pairs: usize,
        total_pairs: usize,
        in_flight_pairs: usize,
        entity_id: String,
        candidate_entity_id: String,
        similarity: f32,
        decision: EntityMatchDecision,
        /// Whether this completion used the provider or transitive equality.
        source: EntityVerificationSource,
    },
    /// Provider work is complete and graph merges are being applied locally.
    Finalizing { verified_pair_count: usize },
}

/// Stage-specific failure from a complete entity-resolution run.
#[derive(Debug)]
pub enum EntityResolutionPipelineError {
    CandidateGeneration(CandidateGenerationError),
    Embedding(EmbeddingServiceError),
    Verification(EntityVerificationError),
    MergePlanning(MergePlanningError),
    GraphRewrite(GraphRewriteError),
}

impl fmt::Display for EntityResolutionPipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CandidateGeneration(source) => {
                write!(formatter, "Entity candidate generation failed: {source}")
            }
            Self::Embedding(source) => write!(formatter, "Entity embedding failed: {source}"),
            Self::Verification(source) => write!(formatter, "Entity verification failed: {source}"),
            Self::MergePlanning(source) => {
                write!(formatter, "Entity merge planning failed: {source}")
            }
            Self::GraphRewrite(source) => {
                write!(formatter, "Entity graph rewrite failed: {source}")
            }
        }
    }
}

impl Error for EntityResolutionPipelineError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CandidateGeneration(source) => Some(source),
            Self::Embedding(source) => Some(source),
            Self::Verification(source) => Some(source),
            Self::MergePlanning(source) => Some(source),
            Self::GraphRewrite(source) => Some(source),
        }
    }
}

impl From<CandidateGenerationError> for EntityResolutionPipelineError {
    fn from(value: CandidateGenerationError) -> Self {
        Self::CandidateGeneration(value)
    }
}

impl From<EmbeddingServiceError> for EntityResolutionPipelineError {
    fn from(value: EmbeddingServiceError) -> Self {
        Self::Embedding(value)
    }
}

impl From<EntityVerificationError> for EntityResolutionPipelineError {
    fn from(value: EntityVerificationError) -> Self {
        Self::Verification(value)
    }
}

impl From<MergePlanningError> for EntityResolutionPipelineError {
    fn from(value: MergePlanningError) -> Self {
        Self::MergePlanning(value)
    }
}

impl From<GraphRewriteError> for EntityResolutionPipelineError {
    fn from(value: GraphRewriteError) -> Self {
        Self::GraphRewrite(value)
    }
}

/// Resolves semantically equivalent graph entities and rebuilds the graph index.
///
/// Local configuration is validated before either provider is contacted. The
/// function stops at the first operational error and never returns a partially
/// rewritten graph. Empty graphs and zero-candidate runs complete without an
/// LLM verification request.
pub async fn resolve_graph_entities(
    graph: &PropositionGraph,
    embedding_service: &EmbeddingService,
    llm_service: &LlmService,
    config: &EntityResolutionConfig,
) -> Result<EntityResolutionResult, EntityResolutionPipelineError> {
    resolve_graph_entities_with_progress(graph, embedding_service, llm_service, config, |_| {})
        .await
}

/// Resolves graph entities while reporting provider-backed stage progress.
///
/// The callback is synchronous and lightweight by design: callers should update
/// in-memory status only, never perform blocking I/O from it.
pub async fn resolve_graph_entities_with_progress<F>(
    graph: &PropositionGraph,
    embedding_service: &EmbeddingService,
    llm_service: &LlmService,
    config: &EntityResolutionConfig,
    mut on_progress: F,
) -> Result<EntityResolutionResult, EntityResolutionPipelineError>
where
    F: FnMut(EntityResolutionProgress) + Send,
{
    // Fail invalid local configuration before incurring provider work.
    config.candidates.validate()?;
    config.verifier.validate()?;
    let resolution_started = Instant::now();
    log::info!(
        "Starting entity resolution (entities={})",
        graph.entities.len()
    );

    // Contexts are built once and reused for both semantic stages so the LLM
    // verifies the exact evidence that shaped candidate retrieval.
    let contexts = build_entity_contexts(graph, config.max_points_per_entity);
    on_progress(EntityResolutionProgress::GeneratingEmbeddings {
        entity_count: contexts.len(),
    });
    let embedding_started = Instant::now();
    let embeddings = generate_entity_context_embeddings(&contexts, embedding_service).await?;
    log::info!(
        "Generated {} entity embeddings in {:.2?}",
        embeddings.len(),
        embedding_started.elapsed()
    );
    let candidates = generate_entity_candidates(&embeddings, &config.candidates)?;
    on_progress(EntityResolutionProgress::CandidatesGenerated {
        candidate_count: candidates.len(),
    });
    for (index, candidate) in candidates.iter().enumerate() {
        on_progress(EntityResolutionProgress::CandidateSelected {
            position: index + 1,
            total_pairs: candidates.len(),
            entity_id: candidate.entity_id.clone(),
            candidate_entity_id: candidate.candidate_entity_id.clone(),
            similarity: candidate.similarity,
        });
    }
    log::info!(
        "Generated {} entity verification candidates (minimum_similarity={:.3}, max_per_entity={})",
        candidates.len(),
        config.candidates.minimum_similarity,
        config.candidates.max_candidates_per_entity
    );

    let verification_started = Instant::now();
    let verified_candidates = verify_entity_candidates_with_progress(
        &candidates,
        &contexts,
        llm_service,
        &config.verifier,
        |progress| {
            on_progress(EntityResolutionProgress::VerifyingCandidates {
                completed_pairs: progress.completed_pairs,
                total_pairs: progress.total_pairs,
                in_flight_pairs: progress.in_flight_pairs,
                entity_id: progress.entity_id,
                candidate_entity_id: progress.candidate_entity_id,
                similarity: progress.similarity,
                decision: progress.decision,
                source: progress.source,
            });
        },
    )
    .await?;
    let transitive_inference_count = verified_candidates
        .iter()
        .filter(|result| result.source == EntityVerificationSource::TransitiveInference)
        .count();
    log::info!(
        "Resolved {} entity candidate pairs in {:.2?} (llm_requests={}, transitive_skips={})",
        verified_candidates.len(),
        verification_started.elapsed(),
        verified_candidates.len() - transitive_inference_count,
        transitive_inference_count
    );
    on_progress(EntityResolutionProgress::Finalizing {
        verified_pair_count: verified_candidates.len(),
    });
    let merge_plan = build_entity_merge_plan(graph, &verified_candidates)?;
    let rewritten_graph = rewrite_graph_with_entity_merges(graph, &merge_plan)?;
    let index = build_index(&rewritten_graph);
    let metrics = build_metrics(
        graph,
        &rewritten_graph,
        &candidates,
        &verified_candidates,
        &merge_plan,
    );
    log::info!(
        "Finished entity resolution in {:.2?} (entities_before={}, entities_after={}, candidates={})",
        resolution_started.elapsed(),
        metrics.entity_count_before,
        metrics.entity_count_after,
        metrics.candidate_pair_count
    );

    Ok(EntityResolutionResult {
        graph: rewritten_graph,
        index,
        candidates,
        verified_candidates,
        merge_plan,
        metrics,
    })
}

fn build_metrics(
    source_graph: &PropositionGraph,
    rewritten_graph: &PropositionGraph,
    candidates: &[EntityCandidate],
    verified: &[VerifiedEntityCandidate],
    merge_plan: &EntityMergePlan,
) -> EntityResolutionMetrics {
    let mut metrics = EntityResolutionMetrics {
        entity_count_before: source_graph.entities.len(),
        entity_count_after: rewritten_graph.entities.len(),
        candidate_pair_count: candidates.len(),
        merge_group_count: merge_plan.merges.len(),
        ..EntityResolutionMetrics::default()
    };

    for result in verified {
        match result.decision {
            EntityMatchDecision::SameEntity => metrics.same_entity_count += 1,
            EntityMatchDecision::DifferentEntity => metrics.different_entity_count += 1,
            EntityMatchDecision::Uncertain => metrics.unresolved_pair_count += 1,
        }
    }

    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::embedding::{EmbeddingConfig, EmbeddingProvider};
    use crate::services::graph_generation::types::{
        EntityNode, KnowledgePoint, KnowledgeType, Relation, RelationType,
    };
    use crate::services::llm::{LlmConfig, LlmProvider};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    fn entity(id: &str, name: &str, aliases: &[&str]) -> EntityNode {
        EntityNode {
            id: id.to_string(),
            canonical_name: name.to_string(),
            aliases: aliases.iter().map(|value| value.to_string()).collect(),
            chunk_ids: vec![format!("chunk-{id}")],
        }
    }

    fn point(id: &str, text: &str, entity_ids: &[&str]) -> KnowledgePoint {
        KnowledgePoint {
            id: id.to_string(),
            point: text.to_string(),
            knowledge_type: KnowledgeType::Fact,
            chunk_id: String::from("chunk-1"),
            raw_entity_names: Vec::new(),
            entity_ids: entity_ids.iter().map(|value| value.to_string()).collect(),
            raw_relations: Vec::new(),
        }
    }

    fn graph() -> PropositionGraph {
        PropositionGraph {
            entities: vec![
                entity("co2-symbol", "CO₂", &["CO₂", "CO2"]),
                entity("carbon-dioxide", "carbon dioxide", &["carbon dioxide"]),
                entity("oxygen", "oxygen", &["oxygen", "O₂"]),
            ],
            knowledge_points: vec![
                point("kp-1", "CO₂ is attached to RuBP.", &["co2-symbol"]),
                point(
                    "kp-2",
                    "Carbon dioxide participates in fixation.",
                    &["carbon-dioxide"],
                ),
                point("kp-3", "Oxygen is released.", &["oxygen"]),
            ],
            relations: vec![Relation {
                source_id: String::from("carbon-dioxide"),
                target_id: String::from("oxygen"),
                relation_type: RelationType::RelatedTo,
            }],
        }
    }

    fn embedding_service(base_url: &str) -> EmbeddingService {
        let config = EmbeddingConfig::new(
            EmbeddingProvider::Ollama,
            base_url,
            "test-embedding-model",
            5,
            None,
        )
        .expect("embedding config should be valid");
        EmbeddingService::new(config).expect("embedding service should build")
    }

    fn llm_service(base_url: &str) -> LlmService {
        LlmService::new(LlmConfig {
            provider: LlmProvider::Ollama,
            base_url: base_url.to_string(),
            model: String::from("test-verifier-model"),
            timeout_secs: 5,
            api_key: None,
        })
        .expect("LLM service should build")
    }

    async fn one_request_server(response_body: String) -> (String, oneshot::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock listener should bind");
        let address = listener.local_addr().expect("listener should have address");
        let (request_sender, request_receiver) = oneshot::channel();

        tokio::spawn(async move {
            let (mut socket, _) = listener
                .accept()
                .await
                .expect("server should accept request");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];

            loop {
                let bytes_read = socket.read(&mut chunk).await.expect("request should read");
                if bytes_read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..bytes_read]);

                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }

            let _ = request_sender.send(String::from_utf8_lossy(&request).into_owned());
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(), response_body
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response should write");
        });

        (format!("http://{address}"), request_receiver)
    }

    #[tokio::test]
    async fn resolves_graph_end_to_end_and_rebuilds_the_index() {
        let (embedding_url, embedding_request) = one_request_server(
            json!({"embeddings": [[1.0, 0.0], [0.99, 0.01], [0.0, 1.0]]}).to_string(),
        )
        .await;
        let verification_content = json!({
            "decision": "same_entity",
            "reason": "CO₂ and carbon dioxide are interchangeable names for the same substance."
        })
        .to_string();
        let (llm_url, llm_request) = one_request_server(
            json!({
                "message": {
                    "role": "assistant",
                    "content": verification_content
                }
            })
            .to_string(),
        )
        .await;
        let config = EntityResolutionConfig {
            max_points_per_entity: 2,
            candidates: CandidateConfig {
                minimum_similarity: 0.9,
                max_candidates_per_entity: 3,
            },
            verifier: VerifierConfig::default(),
        };

        let mut progress_events = Vec::new();
        let result = resolve_graph_entities_with_progress(
            &graph(),
            &embedding_service(&embedding_url),
            &llm_service(&llm_url),
            &config,
            |progress| progress_events.push(progress),
        )
        .await
        .expect("complete resolution pipeline should succeed");

        assert_eq!(result.graph.entities.len(), 2);
        assert_eq!(result.graph.entities[0].id, "co2-symbol");
        assert_eq!(
            result.graph.entities[0].aliases,
            vec!["CO₂", "CO2", "carbon dioxide"]
        );
        assert_eq!(result.merge_plan.merges.len(), 1);
        assert_eq!(result.candidates.len(), 1);
        assert_eq!(
            result.verified_candidates[0].decision,
            EntityMatchDecision::SameEntity
        );
        assert_eq!(
            result.index.points_for_entity("co2-symbol"),
            vec!["kp-1", "kp-2"]
        );
        assert_eq!(
            result.index.edges_from_entity("co2-symbol"),
            vec![(RelationType::RelatedTo, String::from("oxygen"))]
        );
        assert_eq!(
            result.metrics,
            EntityResolutionMetrics {
                entity_count_before: 3,
                entity_count_after: 2,
                candidate_pair_count: 1,
                same_entity_count: 1,
                different_entity_count: 0,
                unresolved_pair_count: 0,
                merge_group_count: 1,
            }
        );
        let candidate_similarity = result.candidates[0].similarity;
        assert_eq!(
            progress_events,
            vec![
                EntityResolutionProgress::GeneratingEmbeddings { entity_count: 3 },
                EntityResolutionProgress::CandidatesGenerated { candidate_count: 1 },
                EntityResolutionProgress::CandidateSelected {
                    position: 1,
                    total_pairs: 1,
                    entity_id: String::from("carbon-dioxide"),
                    candidate_entity_id: String::from("co2-symbol"),
                    similarity: candidate_similarity,
                },
                EntityResolutionProgress::VerifyingCandidates {
                    completed_pairs: 1,
                    total_pairs: 1,
                    in_flight_pairs: 0,
                    entity_id: String::from("carbon-dioxide"),
                    candidate_entity_id: String::from("co2-symbol"),
                    similarity: candidate_similarity,
                    decision: EntityMatchDecision::SameEntity,
                    source: EntityVerificationSource::Llm,
                },
                EntityResolutionProgress::Finalizing {
                    verified_pair_count: 1,
                },
            ]
        );

        let embedding_request = embedding_request.await.expect("embedding request captured");
        assert!(embedding_request.contains("Entity: CO₂"));
        let llm_request = llm_request.await.expect("LLM request captured");
        assert!(llm_request.contains("carbon dioxide"));
        assert!(!llm_request.contains("retrieval_similarity"));
    }

    #[tokio::test]
    async fn invalid_config_fails_before_contacting_providers() {
        let mut config = EntityResolutionConfig::default();
        config.candidates.minimum_similarity = f32::NAN;

        let error = resolve_graph_entities(
            &graph(),
            &embedding_service("http://127.0.0.1:1"),
            &llm_service("http://127.0.0.1:1"),
            &config,
        )
        .await
        .expect_err("invalid local config must fail immediately");

        assert!(matches!(
            error,
            EntityResolutionPipelineError::CandidateGeneration(
                CandidateGenerationError::InvalidMinimumSimilarity { value }
            ) if value.is_nan()
        ));
    }

    #[tokio::test]
    async fn empty_graph_completes_without_provider_requests() {
        let empty_graph = PropositionGraph {
            entities: Vec::new(),
            knowledge_points: Vec::new(),
            relations: Vec::new(),
        };
        let result = resolve_graph_entities(
            &empty_graph,
            &embedding_service("http://127.0.0.1:1"),
            &llm_service("http://127.0.0.1:1"),
            &EntityResolutionConfig::default(),
        )
        .await
        .expect("empty graph should not require providers");

        assert!(result.graph.entities.is_empty());
        assert!(result.candidates.is_empty());
        assert!(result.verified_candidates.is_empty());
        assert_eq!(result.metrics, EntityResolutionMetrics::default());
    }
}
