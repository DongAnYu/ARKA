#![allow(dead_code)]

#[path = "../../../src-tauri/src/models/mod.rs"]
mod models;
#[path = "../../../src-tauri/src/services/mod.rs"]
mod services;

use std::collections::{HashMap, HashSet};
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use services::embedding::{EmbeddingConfig, EmbeddingProvider, EmbeddingService};
use services::graph_generation::entity_resolution::candidate_generator::CandidateConfig;
use services::graph_generation::entity_resolution::merge_planner::EntityMergePlan;
use services::graph_generation::entity_resolution::pipeline::{
    resolve_graph_entities, EntityResolutionConfig, EntityResolutionMetrics,
};
use services::graph_generation::entity_resolution::semantic_verifier::{
    EntityMatchDecision, EntityVerificationSource, VerifierConfig,
};
use services::graph_generation::types::{
    EntityNode, KnowledgePoint, KnowledgeType, PropositionGraph,
};
use services::llm::{LlmConfig, LlmProvider, LlmService};

const DEFAULT_FIXTURE_RELATIVE_PATH: &str = "eval/fixtures/entity_resolution/photosynthesis.json";
const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
struct CliArgs {
    fixture_path: PathBuf,
    output_dir: PathBuf,
    minimum_similarity: f32,
    max_candidates_per_entity: usize,
    max_points_per_entity: usize,
}

#[derive(Debug, Deserialize)]
struct EntityResolutionFixture {
    fixture_version: u32,
    name: String,
    description: String,
    entities: Vec<FixtureEntity>,
    labeled_pairs: Vec<LabeledPair>,
    expected_clusters: Vec<ExpectedCluster>,
    expected_entity_count_before: usize,
    expected_entity_count_after: usize,
}

#[derive(Debug, Deserialize)]
struct FixtureEntity {
    id: String,
    name: String,
    aliases: Vec<String>,
    context: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LabeledPair {
    id: String,
    left: String,
    right: String,
    expected: ExpectedDecision,
    rationale: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum ExpectedDecision {
    SameEntity,
    DifferentEntity,
}

#[derive(Debug, Deserialize)]
struct ExpectedCluster {
    canonical_name: String,
    members: Vec<String>,
}

#[derive(Debug, Serialize)]
struct EntityResolutionEvalRun {
    generated_at_utc: String,
    fixture_path: String,
    output_path: String,
    fixture_version: u32,
    fixture_name: String,
    fixture_description: String,
    embedding: ProviderSummary,
    verifier: ProviderSummary,
    config: EvalConfigSummary,
    summary: EvalSummary,
    pair_results: Vec<PairEvaluation>,
    expected_clusters: Vec<ExpectedClusterEvaluation>,
    actual_merge_groups: Vec<MergeGroupSummary>,
    candidates: Vec<CandidateSummary>,
    verifier_decisions: Vec<VerifierDecisionSummary>,
    entities_after: Vec<EntitySummary>,
}

#[derive(Debug, Serialize)]
struct ProviderSummary {
    provider: String,
    base_url: String,
    model: String,
}

#[derive(Debug, Serialize)]
struct EvalConfigSummary {
    minimum_similarity: f32,
    max_candidates_per_entity: usize,
    max_points_per_entity: usize,
    max_reason_chars: usize,
}

#[derive(Debug, Serialize)]
struct EvalSummary {
    /// Whether the rewritten graph exactly matches the expected resolution.
    passed: bool,
    /// Whether every labeled pair was directly retrieved and classified.
    ///
    /// This can fail while `passed` remains true when a multi-member entity
    /// cluster is correctly connected and merged through transitive edges.
    strict_pairwise_passed: bool,
    positive_pair_count: usize,
    negative_pair_count: usize,
    expected_positive_candidates_found: usize,
    expected_positive_merges_found: usize,
    false_positive_merge_count: usize,
    unresolved_positive_pair_count: usize,
    candidate_recall: f64,
    positive_merge_recall: f64,
    false_positive_rate: f64,
    expected_entity_count_before: usize,
    actual_entity_count_before: usize,
    expected_entity_count_after: usize,
    actual_entity_count_after: usize,
    expected_cluster_count: usize,
    matched_cluster_count: usize,
    pipeline_metrics: PipelineMetricsSummary,
}

#[derive(Debug, Serialize)]
struct PipelineMetricsSummary {
    entity_count_before: usize,
    entity_count_after: usize,
    candidate_pair_count: usize,
    same_entity_count: usize,
    different_entity_count: usize,
    unresolved_pair_count: usize,
    merge_group_count: usize,
}

#[derive(Debug, Serialize)]
struct PairEvaluation {
    id: String,
    left: String,
    right: String,
    expected: ExpectedDecision,
    rationale: String,
    emitted_as_candidate: bool,
    similarity: Option<f32>,
    actual_decision: Option<String>,
    verifier_reason: Option<String>,
    outcome: String,
    correct: bool,
}

#[derive(Debug, Serialize)]
struct ExpectedClusterEvaluation {
    expected_canonical_name: String,
    expected_members: Vec<String>,
    canonical_name_matched: bool,
    matched: bool,
    actual_canonical_entity_id: Option<String>,
    actual_canonical_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct MergeGroupSummary {
    canonical_entity_id: String,
    canonical_name: String,
    members: Vec<String>,
}

#[derive(Debug, Serialize)]
struct CandidateSummary {
    entity_id: String,
    candidate_entity_id: String,
    similarity: f32,
}

#[derive(Debug, Serialize)]
struct VerifierDecisionSummary {
    entity_id: String,
    candidate_entity_id: String,
    similarity: f32,
    decision: String,
    reason: String,
    source: String,
}

#[derive(Debug, Serialize)]
struct EntitySummary {
    id: String,
    canonical_name: String,
    aliases: Vec<String>,
    chunk_ids: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_dotenv_if_present();
    let args = parse_args()?;
    fs::create_dir_all(&args.output_dir)?;

    let fixture: EntityResolutionFixture =
        serde_json::from_str(&fs::read_to_string(&args.fixture_path)?)?;
    validate_fixture(&fixture)?;
    let graph = fixture_graph(&fixture);

    let (embedding_service, embedding_summary) = embedding_service_from_env()?;
    let (llm_service, verifier_summary) = llm_service_from_env()?;
    let config = EntityResolutionConfig {
        max_points_per_entity: args.max_points_per_entity,
        candidates: CandidateConfig {
            minimum_similarity: args.minimum_similarity,
            max_candidates_per_entity: args.max_candidates_per_entity,
        },
        verifier: VerifierConfig::default(),
    };

    println!("Entity Resolution Eval: {}", fixture.name);
    println!("Fixture: {}", args.fixture_path.display());
    println!(
        "Embedding: {}/{}",
        embedding_summary.provider, embedding_summary.model
    );
    println!(
        "Verifier: {}/{}",
        verifier_summary.provider, verifier_summary.model
    );
    println!(
        "Candidate settings: similarity >= {:.3}, top {} per entity",
        config.candidates.minimum_similarity, config.candidates.max_candidates_per_entity
    );
    println!("---");

    let result = resolve_graph_entities(&graph, &embedding_service, &llm_service, &config).await?;
    let pair_results = evaluate_pairs(&fixture, &result.candidates, &result.verified_candidates);
    let actual_merge_groups = summarize_merge_groups(&graph, &result.merge_plan);
    let expected_clusters = evaluate_clusters(&fixture, &actual_merge_groups);
    let summary = summarize_run(&fixture, &pair_results, &expected_clusters, result.metrics);

    print_pair_results(&pair_results);
    print_summary(&summary);

    let timestamp = Utc::now();
    let file_prefix = format!(
        "entity-resolution-{}-{}",
        safe_file_stem(&fixture.name),
        timestamp.format("%Y%m%d-%H%M%S")
    );
    let output_path = args.output_dir.join(format!("{file_prefix}.json"));

    let run = EntityResolutionEvalRun {
        generated_at_utc: timestamp.to_rfc3339(),
        fixture_path: args.fixture_path.display().to_string(),
        output_path: output_path.display().to_string(),
        fixture_version: fixture.fixture_version,
        fixture_name: fixture.name,
        fixture_description: fixture.description,
        embedding: embedding_summary,
        verifier: verifier_summary,
        config: EvalConfigSummary {
            minimum_similarity: config.candidates.minimum_similarity,
            max_candidates_per_entity: config.candidates.max_candidates_per_entity,
            max_points_per_entity: config.max_points_per_entity,
            max_reason_chars: config.verifier.max_reason_chars,
        },
        summary,
        pair_results,
        expected_clusters,
        actual_merge_groups,
        candidates: result
            .candidates
            .into_iter()
            .map(|candidate| CandidateSummary {
                entity_id: candidate.entity_id,
                candidate_entity_id: candidate.candidate_entity_id,
                similarity: candidate.similarity,
            })
            .collect(),
        verifier_decisions: result
            .verified_candidates
            .into_iter()
            .map(|decision| VerifierDecisionSummary {
                entity_id: decision.entity_id,
                candidate_entity_id: decision.candidate_entity_id,
                similarity: decision.similarity,
                decision: decision_label(decision.decision).to_string(),
                reason: decision.reason,
                source: verification_source_label(decision.source).to_string(),
            })
            .collect(),
        entities_after: result
            .graph
            .entities
            .into_iter()
            .map(|entity| EntitySummary {
                id: entity.id,
                canonical_name: entity.canonical_name,
                aliases: entity.aliases,
                chunk_ids: entity.chunk_ids,
            })
            .collect(),
    };

    fs::write(&output_path, serde_json::to_string_pretty(&run)?)?;
    println!("Report: {}", output_path.display());

    Ok(())
}

fn validate_fixture(fixture: &EntityResolutionFixture) -> Result<(), Box<dyn Error>> {
    if fixture.entities.len() != fixture.expected_entity_count_before {
        return Err(format!(
            "Fixture has {} entities but expected_entity_count_before is {}",
            fixture.entities.len(),
            fixture.expected_entity_count_before
        )
        .into());
    }

    let mut entity_ids = HashSet::new();
    let mut entity_index_by_id = HashMap::new();
    for (index, entity) in fixture.entities.iter().enumerate() {
        if !entity_ids.insert(entity.id.as_str()) {
            return Err(format!("Fixture contains duplicate entity ID '{}'", entity.id).into());
        }
        entity_index_by_id.insert(entity.id.as_str(), index);
    }

    let mut pair_ids = HashSet::new();
    let mut unordered_pairs = HashSet::new();
    for pair in &fixture.labeled_pairs {
        if !pair_ids.insert(pair.id.as_str()) {
            return Err(format!("Fixture contains duplicate pair ID '{}'", pair.id).into());
        }
        if pair.left == pair.right {
            return Err(format!("Fixture pair '{}' compares an entity to itself", pair.id).into());
        }
        for entity_id in [&pair.left, &pair.right] {
            if !entity_ids.contains(entity_id.as_str()) {
                return Err(format!(
                    "Fixture pair '{}' references unknown entity '{}'",
                    pair.id, entity_id
                )
                .into());
            }
        }
        if !unordered_pairs.insert(pair_key(&pair.left, &pair.right)) {
            return Err(format!("Fixture contains duplicate unordered pair '{}'", pair.id).into());
        }
    }

    let mut clustered_entity_ids = HashSet::new();
    let mut cluster_by_entity_id = HashMap::new();
    let mut merged_entity_count = 0;

    for (cluster_index, cluster) in fixture.expected_clusters.iter().enumerate() {
        if cluster.members.len() < 2 {
            return Err(format!(
                "Expected cluster '{}' must contain at least two entities",
                cluster.canonical_name
            )
            .into());
        }

        let mut members_in_cluster = HashSet::new();
        for member_id in &cluster.members {
            if !entity_ids.contains(member_id.as_str()) {
                return Err(format!(
                    "Expected cluster '{}' references unknown entity '{}'",
                    cluster.canonical_name, member_id
                )
                .into());
            }
            if !members_in_cluster.insert(member_id.as_str()) {
                return Err(format!(
                    "Expected cluster '{}' contains duplicate entity '{}'",
                    cluster.canonical_name, member_id
                )
                .into());
            }
            if !clustered_entity_ids.insert(member_id.as_str()) {
                return Err(format!(
                    "Entity '{}' appears in more than one expected cluster",
                    member_id
                )
                .into());
            }
            cluster_by_entity_id.insert(member_id.as_str(), cluster_index);
        }

        let earliest_member_index = cluster
            .members
            .iter()
            .filter_map(|member_id| entity_index_by_id.get(member_id.as_str()).copied())
            .min()
            .expect("validated expected cluster should contain known members");
        let earliest_name = &fixture.entities[earliest_member_index].name;
        if cluster.canonical_name != *earliest_name {
            return Err(format!(
                "Expected cluster canonical name '{}' does not match earliest graph entity name '{}'",
                cluster.canonical_name, earliest_name
            )
            .into());
        }

        merged_entity_count += cluster.members.len() - 1;
    }

    let calculated_entity_count_after = fixture.entities.len() - merged_entity_count;
    if calculated_entity_count_after != fixture.expected_entity_count_after {
        return Err(format!(
            "Expected clusters imply {} entities after resolution, but expected_entity_count_after is {}",
            calculated_entity_count_after, fixture.expected_entity_count_after
        )
        .into());
    }

    for pair in &fixture.labeled_pairs {
        let left_cluster = cluster_by_entity_id.get(pair.left.as_str());
        let right_cluster = cluster_by_entity_id.get(pair.right.as_str());
        match pair.expected {
            ExpectedDecision::SameEntity
                if left_cluster.is_none() || left_cluster != right_cluster =>
            {
                return Err(format!(
                    "Positive pair '{}' is not represented by one expected cluster",
                    pair.id
                )
                .into());
            }
            ExpectedDecision::DifferentEntity
                if left_cluster.is_some() && left_cluster == right_cluster =>
            {
                return Err(format!(
                    "Negative pair '{}' places both entities in the same expected cluster",
                    pair.id
                )
                .into());
            }
            _ => {}
        }
    }

    Ok(())
}

fn fixture_graph(fixture: &EntityResolutionFixture) -> PropositionGraph {
    let entities = fixture
        .entities
        .iter()
        .map(|entity| EntityNode {
            id: entity.id.clone(),
            canonical_name: entity.name.clone(),
            aliases: entity.aliases.clone(),
            chunk_ids: vec![format!("fixture-{}", entity.id)],
        })
        .collect();

    let knowledge_points = fixture
        .entities
        .iter()
        .flat_map(|entity| {
            entity
                .context
                .iter()
                .enumerate()
                .map(|(index, context)| KnowledgePoint {
                    id: format!("fixture-kp-{}-{index}", entity.id),
                    point: context.clone(),
                    knowledge_type: KnowledgeType::Fact,
                    chunk_id: format!("fixture-{}", entity.id),
                    raw_entity_names: vec![entity.name.clone()],
                    entity_ids: vec![entity.id.clone()],
                    raw_relations: Vec::new(),
                })
        })
        .collect();

    PropositionGraph {
        entities,
        knowledge_points,
        relations: Vec::new(),
    }
}

fn evaluate_pairs(
    fixture: &EntityResolutionFixture,
    candidates: &[services::graph_generation::entity_resolution::candidate_generator::EntityCandidate],
    verified: &[services::graph_generation::entity_resolution::semantic_verifier::VerifiedEntityCandidate],
) -> Vec<PairEvaluation> {
    let candidates_by_pair = candidates
        .iter()
        .map(|candidate| {
            (
                pair_key(&candidate.entity_id, &candidate.candidate_entity_id),
                candidate.similarity,
            )
        })
        .collect::<HashMap<_, _>>();
    let verified_by_pair = verified
        .iter()
        .map(|decision| {
            (
                pair_key(&decision.entity_id, &decision.candidate_entity_id),
                decision,
            )
        })
        .collect::<HashMap<_, _>>();

    fixture
        .labeled_pairs
        .iter()
        .map(|pair| {
            let key = pair_key(&pair.left, &pair.right);
            let similarity = candidates_by_pair.get(&key).copied();
            let decision = verified_by_pair.get(&key).copied();
            let (outcome, correct) =
                pair_outcome(pair.expected, decision.map(|item| item.decision));

            PairEvaluation {
                id: pair.id.clone(),
                left: pair.left.clone(),
                right: pair.right.clone(),
                expected: pair.expected,
                rationale: pair.rationale.clone(),
                emitted_as_candidate: similarity.is_some(),
                similarity,
                actual_decision: decision.map(|item| decision_label(item.decision).to_string()),
                verifier_reason: decision.map(|item| item.reason.clone()),
                outcome: outcome.to_string(),
                correct,
            }
        })
        .collect()
}

fn pair_outcome(
    expected: ExpectedDecision,
    actual: Option<EntityMatchDecision>,
) -> (&'static str, bool) {
    match (expected, actual) {
        (ExpectedDecision::SameEntity, Some(EntityMatchDecision::SameEntity)) => {
            ("expected_merge_found", true)
        }
        (ExpectedDecision::SameEntity, Some(EntityMatchDecision::DifferentEntity)) => {
            ("false_negative", false)
        }
        (ExpectedDecision::SameEntity, Some(EntityMatchDecision::Uncertain)) => {
            ("unresolved", false)
        }
        (ExpectedDecision::SameEntity, None) => ("missed_candidate", false),
        (ExpectedDecision::DifferentEntity, Some(EntityMatchDecision::SameEntity)) => {
            ("false_positive", false)
        }
        (ExpectedDecision::DifferentEntity, Some(EntityMatchDecision::DifferentEntity)) => {
            ("expected_non_merge_found", true)
        }
        (ExpectedDecision::DifferentEntity, Some(EntityMatchDecision::Uncertain)) => {
            ("unresolved", false)
        }
        // A negative pair filtered by retrieval is a correct non-merge. The
        // verifier need not spend a request proving every dissimilar pair.
        (ExpectedDecision::DifferentEntity, None) => ("correctly_filtered", true),
    }
}

fn summarize_merge_groups(
    source_graph: &PropositionGraph,
    merge_plan: &EntityMergePlan,
) -> Vec<MergeGroupSummary> {
    let entity_name_by_id = source_graph
        .entities
        .iter()
        .map(|entity| (entity.id.as_str(), entity.canonical_name.as_str()))
        .collect::<HashMap<_, _>>();

    merge_plan
        .merges
        .iter()
        .map(|entity_merge| {
            let mut members = vec![entity_merge.canonical_entity_id.clone()];
            members.extend(entity_merge.merged_entity_ids.iter().cloned());
            MergeGroupSummary {
                canonical_entity_id: entity_merge.canonical_entity_id.clone(),
                canonical_name: entity_name_by_id
                    .get(entity_merge.canonical_entity_id.as_str())
                    .copied()
                    .unwrap_or("<unknown>")
                    .to_string(),
                members,
            }
        })
        .collect()
}

fn evaluate_clusters(
    fixture: &EntityResolutionFixture,
    actual: &[MergeGroupSummary],
) -> Vec<ExpectedClusterEvaluation> {
    fixture
        .expected_clusters
        .iter()
        .map(|expected| {
            let mut expected_members = expected.members.clone();
            expected_members.sort();
            let matching_group = actual.iter().find(|group| {
                let mut actual_members = group.members.clone();
                actual_members.sort();
                actual_members == expected_members
            });
            let canonical_name_matched = matching_group
                .map(|group| group.canonical_name == expected.canonical_name)
                .unwrap_or(false);

            ExpectedClusterEvaluation {
                expected_canonical_name: expected.canonical_name.clone(),
                expected_members: expected.members.clone(),
                canonical_name_matched,
                // A cluster only passes when both its membership and the
                // deterministic earliest-graph canonical survivor are right.
                matched: matching_group.is_some() && canonical_name_matched,
                actual_canonical_entity_id: matching_group
                    .map(|group| group.canonical_entity_id.clone()),
                actual_canonical_name: matching_group.map(|group| group.canonical_name.clone()),
            }
        })
        .collect()
}

fn summarize_run(
    fixture: &EntityResolutionFixture,
    pairs: &[PairEvaluation],
    clusters: &[ExpectedClusterEvaluation],
    pipeline_metrics: EntityResolutionMetrics,
) -> EvalSummary {
    let positive_pair_count = pairs
        .iter()
        .filter(|pair| pair.expected == ExpectedDecision::SameEntity)
        .count();
    let negative_pair_count = pairs.len().saturating_sub(positive_pair_count);
    let expected_positive_candidates_found = pairs
        .iter()
        .filter(|pair| pair.expected == ExpectedDecision::SameEntity && pair.emitted_as_candidate)
        .count();
    let expected_positive_merges_found = pairs
        .iter()
        .filter(|pair| pair.outcome == "expected_merge_found")
        .count();
    let false_positive_merge_count = pairs
        .iter()
        .filter(|pair| pair.outcome == "false_positive")
        .count();
    let unresolved_positive_pair_count = pairs
        .iter()
        .filter(|pair| {
            pair.expected == ExpectedDecision::SameEntity
                && matches!(pair.outcome.as_str(), "unresolved" | "missed_candidate")
        })
        .count();
    let matched_cluster_count = clusters.iter().filter(|cluster| cluster.matched).count();

    let candidate_recall = ratio(expected_positive_candidates_found, positive_pair_count);
    let positive_merge_recall = ratio(expected_positive_merges_found, positive_pair_count);
    let false_positive_rate = ratio(false_positive_merge_count, negative_pair_count);
    let strict_pairwise_passed = pairs.iter().all(|pair| pair.correct);
    let passed = false_positive_merge_count == 0
        && pipeline_metrics.entity_count_before == fixture.expected_entity_count_before
        && pipeline_metrics.entity_count_after == fixture.expected_entity_count_after
        && matched_cluster_count == clusters.len();

    EvalSummary {
        passed,
        strict_pairwise_passed,
        positive_pair_count,
        negative_pair_count,
        expected_positive_candidates_found,
        expected_positive_merges_found,
        false_positive_merge_count,
        unresolved_positive_pair_count,
        candidate_recall,
        positive_merge_recall,
        false_positive_rate,
        expected_entity_count_before: fixture.expected_entity_count_before,
        actual_entity_count_before: pipeline_metrics.entity_count_before,
        expected_entity_count_after: fixture.expected_entity_count_after,
        actual_entity_count_after: pipeline_metrics.entity_count_after,
        expected_cluster_count: clusters.len(),
        matched_cluster_count,
        pipeline_metrics: PipelineMetricsSummary::from(pipeline_metrics),
    }
}

impl From<EntityResolutionMetrics> for PipelineMetricsSummary {
    fn from(value: EntityResolutionMetrics) -> Self {
        Self {
            entity_count_before: value.entity_count_before,
            entity_count_after: value.entity_count_after,
            candidate_pair_count: value.candidate_pair_count,
            same_entity_count: value.same_entity_count,
            different_entity_count: value.different_entity_count,
            unresolved_pair_count: value.unresolved_pair_count,
            merge_group_count: value.merge_group_count,
        }
    }
}

fn print_pair_results(pairs: &[PairEvaluation]) {
    for pair in pairs {
        println!(
            "{} {} ↔ {} | expected={:?} candidate={} similarity={} actual={} outcome={}",
            if pair.correct { "PASS" } else { "FAIL" },
            pair.left,
            pair.right,
            pair.expected,
            pair.emitted_as_candidate,
            pair.similarity
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| String::from("n/a")),
            pair.actual_decision.as_deref().unwrap_or("not_verified"),
            pair.outcome,
        );
    }
}

fn print_summary(summary: &EvalSummary) {
    println!("---");
    println!(
        "Final resolution: {}",
        if summary.passed { "PASS" } else { "FAIL" }
    );
    println!(
        "Strict pairwise evaluation: {}",
        if summary.strict_pairwise_passed {
            "PASS"
        } else {
            "FAIL"
        }
    );
    println!(
        "Candidate recall: {}/{} ({:.1}%)",
        summary.expected_positive_candidates_found,
        summary.positive_pair_count,
        summary.candidate_recall * 100.0
    );
    println!(
        "Direct verified same-entity edges: {}/{} ({:.1}%)",
        summary.expected_positive_merges_found,
        summary.positive_pair_count,
        summary.positive_merge_recall * 100.0
    );
    println!(
        "False positives: {}/{} ({:.1}%)",
        summary.false_positive_merge_count,
        summary.negative_pair_count,
        summary.false_positive_rate * 100.0
    );
    println!(
        "Positive pairs not directly resolved: {}",
        summary.unresolved_positive_pair_count
    );
    println!(
        "Entities: {} → {} (expected {} → {})",
        summary.actual_entity_count_before,
        summary.actual_entity_count_after,
        summary.expected_entity_count_before,
        summary.expected_entity_count_after
    );
    println!(
        "Expected clusters matched: {}/{}",
        summary.matched_cluster_count, summary.expected_cluster_count
    );
}

fn embedding_service_from_env() -> Result<(EmbeddingService, ProviderSummary), Box<dyn Error>> {
    let provider_name = env::var("EMBEDDING_PROVIDER").unwrap_or_else(|_| String::from("ollama"));
    let provider = EmbeddingProvider::from_config_value(&provider_name)
        .ok_or_else(|| format!("Unsupported EMBEDDING_PROVIDER '{provider_name}'"))?;
    let base_url = env::var("EMBEDDING_BASE_URL")
        .unwrap_or_else(|_| default_provider_base_url(provider.as_str()).to_string());
    let model = required_env("EMBEDDING_MODEL")?;
    let timeout_secs = optional_u64_env("EMBEDDING_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS)?;
    let api_key = provider_api_key(provider.as_str(), "EMBEDDING_API_KEY")?;
    let config = EmbeddingConfig::new(provider, &base_url, &model, timeout_secs, api_key)?;
    let service = EmbeddingService::new(config)?;

    Ok((
        service,
        ProviderSummary {
            provider: provider.as_str().to_string(),
            base_url,
            model,
        },
    ))
}

fn llm_service_from_env() -> Result<(LlmService, ProviderSummary), Box<dyn Error>> {
    let provider_name = env::var("LLM_PROVIDER").unwrap_or_else(|_| String::from("openrouter"));
    let provider = parse_llm_provider(&provider_name)?;
    let base_url = match provider {
        LlmProvider::Ollama => env::var("LLM_BASE_URL")
            .unwrap_or_else(|_| default_provider_base_url("ollama").to_string()),
        LlmProvider::OpenAi => env::var("OPENAI_BASE_URL")
            .or_else(|_| env::var("LLM_BASE_URL"))
            .unwrap_or_else(|_| default_provider_base_url("openai").to_string()),
        LlmProvider::OpenRouter => env::var("OPENROUTER_BASE_URL")
            .or_else(|_| env::var("LLM_BASE_URL"))
            .unwrap_or_else(|_| default_provider_base_url("openrouter").to_string()),
    };
    let model = required_env("LLM_MODEL")?;
    let timeout_secs = optional_u64_env("LLM_TIMEOUT_SECS", DEFAULT_TIMEOUT_SECS)?;
    let provider_label = llm_provider_label(provider);
    let api_key = provider_api_key(provider_label, "LLM_API_KEY")?;
    let service = LlmService::new(LlmConfig {
        provider,
        base_url: base_url.clone(),
        model: model.clone(),
        timeout_secs,
        api_key,
    })?;

    Ok((
        service,
        ProviderSummary {
            provider: provider_label.to_string(),
            base_url,
            model,
        },
    ))
}

fn provider_api_key(
    provider: &str,
    generic_env_name: &str,
) -> Result<Option<String>, Box<dyn Error>> {
    if provider == "ollama" {
        return Ok(None);
    }

    let provider_env_name = match provider {
        "openai" => "OPENAI_API_KEY",
        "openrouter" => "OPENROUTER_API_KEY",
        _ => return Err(format!("Unsupported provider '{provider}'").into()),
    };
    let key = env::var(generic_env_name)
        .or_else(|_| env::var(provider_env_name))
        .map_err(|_| {
            format!(
                "Required environment variable {generic_env_name} or {provider_env_name} is not set"
            )
        })?;
    let key = key.trim();
    if key.is_empty() {
        return Err(format!("API key for provider '{provider}' is empty").into());
    }

    Ok(Some(key.to_string()))
}

fn parse_llm_provider(value: &str) -> Result<LlmProvider, Box<dyn Error>> {
    match value.trim().to_ascii_lowercase().as_str() {
        "ollama" => Ok(LlmProvider::Ollama),
        "openai" => Ok(LlmProvider::OpenAi),
        "openrouter" => Ok(LlmProvider::OpenRouter),
        _ => Err(format!("Unsupported LLM_PROVIDER '{value}'").into()),
    }
}

fn llm_provider_label(provider: LlmProvider) -> &'static str {
    match provider {
        LlmProvider::Ollama => "ollama",
        LlmProvider::OpenAi => "openai",
        LlmProvider::OpenRouter => "openrouter",
    }
}

fn default_provider_base_url(provider: &str) -> &'static str {
    match provider {
        "ollama" => "http://127.0.0.1:11434",
        "openai" => "https://api.openai.com/v1",
        "openrouter" => "https://openrouter.ai/api/v1",
        _ => "",
    }
}

fn parse_args() -> Result<CliArgs, Box<dyn Error>> {
    let repo_root = repo_root();
    let mut fixture_path = repo_root.join(DEFAULT_FIXTURE_RELATIVE_PATH);
    let mut output_dir = repo_root.join(DEFAULT_OUTPUT_DIR_NAME);
    let mut minimum_similarity = 0.5;
    let mut max_candidates_per_entity = 3;
    let mut max_points_per_entity = 3;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--fixture" => fixture_path = PathBuf::from(next_arg(&mut args, "--fixture")?),
            "--output-dir" => output_dir = PathBuf::from(next_arg(&mut args, "--output-dir")?),
            "--minimum-similarity" => {
                minimum_similarity = next_arg(&mut args, "--minimum-similarity")?.parse()?
            }
            "--max-candidates" => {
                max_candidates_per_entity = next_arg(&mut args, "--max-candidates")?.parse()?
            }
            "--max-context-points" => {
                max_points_per_entity = next_arg(&mut args, "--max-context-points")?.parse()?
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            unexpected => return Err(format!("Unsupported argument: {unexpected}").into()),
        }
    }

    Ok(CliArgs {
        fixture_path,
        output_dir,
        minimum_similarity,
        max_candidates_per_entity,
        max_points_per_entity,
    })
}

fn next_arg(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("Missing value for {option}").into())
}

fn print_usage() {
    println!(
        "Usage: cargo run --manifest-path eval/Cargo.toml --bin entity_resolution_eval -- [--fixture <json>] [--output-dir <dir>] [--minimum-similarity <f32>] [--max-candidates <usize>] [--max-context-points <usize>]"
    );
}

fn pair_key(left: &str, right: &str) -> (String, String) {
    if left < right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn decision_label(decision: EntityMatchDecision) -> &'static str {
    match decision {
        EntityMatchDecision::SameEntity => "same_entity",
        EntityMatchDecision::DifferentEntity => "different_entity",
        EntityMatchDecision::Uncertain => "uncertain",
    }
}

fn verification_source_label(source: EntityVerificationSource) -> &'static str {
    match source {
        EntityVerificationSource::Llm => "llm",
        EntityVerificationSource::TransitiveInference => "transitive_inference",
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn safe_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn optional_u64_env(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    match env::var(name) {
        Ok(value) => {
            let parsed = value
                .parse::<u64>()
                .map_err(|_| format!("{name} must be a positive integer"))?;
            if parsed == 0 {
                return Err(format!("{name} must be greater than zero").into());
            }
            Ok(parsed)
        }
        Err(_) => Ok(default),
    }
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    let value =
        env::var(name).map_err(|_| format!("Required environment variable {name} is not set"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("Required environment variable {name} is empty").into());
    }
    Ok(value.to_string())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_dotenv_if_present() {
    let _ = dotenvy::from_path(repo_root().join("src-tauri/.env"));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(source: &str) -> EntityResolutionFixture {
        serde_json::from_str(source).expect("checked-in fixture should parse")
    }

    #[test]
    fn checked_in_fixtures_are_structurally_valid() {
        let smoke = fixture(include_str!(
            "../../fixtures/entity_resolution/photosynthesis.json"
        ));
        let hard = fixture(include_str!(
            "../../fixtures/entity_resolution/photosynthesis-hard.json"
        ));
        assert!(hard.entities.len() > smoke.entities.len());
        assert!(hard.labeled_pairs.len() > smoke.labeled_pairs.len());

        for fixture in [smoke, hard] {
            validate_fixture(&fixture).expect("checked-in fixture should be valid");
            assert!(!fixture.entities.is_empty());
            assert!(!fixture.labeled_pairs.is_empty());
            assert!(fixture
                .labeled_pairs
                .iter()
                .any(|pair| pair.expected == ExpectedDecision::SameEntity));
            assert!(fixture
                .labeled_pairs
                .iter()
                .any(|pair| pair.expected == ExpectedDecision::DifferentEntity));
        }
    }

    #[test]
    fn negative_pair_filtered_before_verification_is_correct() {
        assert_eq!(
            pair_outcome(ExpectedDecision::DifferentEntity, None),
            ("correctly_filtered", true)
        );
    }

    #[test]
    fn positive_pair_missing_from_candidates_is_unresolved() {
        assert_eq!(
            pair_outcome(ExpectedDecision::SameEntity, None),
            ("missed_candidate", false)
        );
    }

    #[test]
    fn transitive_cluster_success_is_separate_from_pairwise_retrieval() {
        let fixture = fixture(include_str!(
            "../../fixtures/entity_resolution/photosynthesis.json"
        ));
        let pairs = vec![PairEvaluation {
            id: String::from("indirect-positive"),
            left: String::from("co2"),
            right: String::from("carbon-dioxide"),
            expected: ExpectedDecision::SameEntity,
            rationale: String::from("Merged through another member"),
            emitted_as_candidate: false,
            similarity: None,
            actual_decision: None,
            verifier_reason: None,
            outcome: String::from("missed_candidate"),
            correct: false,
        }];
        let clusters = fixture
            .expected_clusters
            .iter()
            .map(|cluster| ExpectedClusterEvaluation {
                expected_canonical_name: cluster.canonical_name.clone(),
                expected_members: cluster.members.clone(),
                canonical_name_matched: true,
                matched: true,
                actual_canonical_entity_id: cluster.members.first().cloned(),
                actual_canonical_name: Some(cluster.canonical_name.clone()),
            })
            .collect::<Vec<_>>();
        let metrics = EntityResolutionMetrics {
            entity_count_before: fixture.expected_entity_count_before,
            entity_count_after: fixture.expected_entity_count_after,
            merge_group_count: fixture.expected_clusters.len(),
            ..EntityResolutionMetrics::default()
        };

        let summary = summarize_run(&fixture, &pairs, &clusters, metrics);

        assert!(summary.passed);
        assert!(!summary.strict_pairwise_passed);
    }
}
