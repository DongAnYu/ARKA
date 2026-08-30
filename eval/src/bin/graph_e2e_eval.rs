#![allow(dead_code)]

#[path = "../../../src-tauri/src/models/mod.rs"]
mod models;
#[path = "../../../src-tauri/src/services/mod.rs"]
mod services;

use std::collections::HashMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use rust_xlsxwriter::Workbook;
use serde::Serialize;

use services::embedding::{EmbeddingConfig, EmbeddingProvider, EmbeddingService};
use services::graph_generation::entity_resolution::candidate_generator::CandidateConfig;
use services::graph_generation::entity_resolution::pipeline::{
    resolve_graph_entities_with_progress, EntityResolutionConfig, EntityResolutionProgress,
};
use services::graph_generation::entity_resolution::semantic_verifier::{
    EntityMatchDecision, VerifierConfig,
};
use services::graph_generation::pipeline::{
    run_graph_stage_a_with_progress, run_graph_stage_b_for_graph, GraphStageAProgress,
    GraphStageAResult, GraphStageBResult,
};
use services::graph_generation::types::PropositionGraph;
use services::llm::{LlmConfig, LlmProvider, LlmRetryState, LlmService};

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
struct CliArgs {
    note_path: PathBuf,
    output_dir: PathBuf,
    minimum_similarity: f32,
    max_candidates_per_entity: usize,
    max_points_per_entity: usize,
    max_concurrency: usize,
}

#[derive(Debug, Serialize)]
struct GraphE2eEvalRun {
    generated_at_utc: String,
    provider: String,
    base_url: String,
    model: String,
    note_path: String,
    json_path: String,
    xlsx_path: String,
    embedding: ProviderSummary,
    timings: EvalTimings,
    stage_a_summary: StageASummary,
    entity_resolution: EntityResolutionReport,
    stage_b_summary: StageBSummary,
    stage_a: GraphStageAResult,
    stage_b: GraphStageBResult,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderSummary {
    provider: String,
    base_url: String,
    model: String,
    timeout_secs: u64,
}

#[derive(Debug, Serialize)]
struct EvalTimings {
    total_ms: u128,
    stage_a_ms: u128,
    entity_resolution_ms: u128,
    embedding_and_candidate_ms: u128,
    verification_ms: u128,
    finalization_ms: u128,
    stage_b_ms: u128,
}

#[derive(Debug, Serialize)]
struct EntityResolutionReport {
    config: EntityResolutionConfigReport,
    summary: EntityResolutionSummary,
    candidates: Vec<EntityCandidateReport>,
    verification_progress: Vec<VerificationProgressReport>,
    merge_groups: Vec<MergeGroupReport>,
    resolved_graph: PropositionGraph,
}

#[derive(Debug, Serialize)]
struct EntityResolutionConfigReport {
    minimum_similarity: f32,
    max_candidates_per_entity: usize,
    max_points_per_entity: usize,
    max_reason_chars: usize,
    max_concurrency: usize,
}

#[derive(Debug, Serialize)]
struct EntityResolutionSummary {
    entities_before: usize,
    entities_after: usize,
    candidate_pairs: usize,
    same_entity: usize,
    different_entity: usize,
    uncertain: usize,
    merge_groups: usize,
}

#[derive(Debug, Serialize)]
struct EntityCandidateReport {
    position: usize,
    entity_id: String,
    entity_name: String,
    candidate_entity_id: String,
    candidate_entity_name: String,
    similarity: f32,
    decision: String,
    reason: String,
}

#[derive(Debug, Serialize)]
struct VerificationProgressReport {
    completed_pairs: usize,
    total_pairs: usize,
    in_flight_pairs: usize,
    entity_id: String,
    candidate_entity_id: String,
    similarity: f32,
    decision: String,
    elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
struct MergeGroupReport {
    canonical_entity_id: String,
    merged_entity_ids: Vec<String>,
}

#[derive(Default)]
struct ResolutionTrace {
    embedding_and_candidate_ms: u128,
    verification_ms: u128,
    finalization_started_ms: u128,
    verification_progress: Vec<VerificationProgressReport>,
}

#[derive(Debug, Serialize)]
struct StageASummary {
    total_chunks: usize,
    successful_chunks: usize,
    failed_chunks: usize,
    total_raw_mentions: usize,
    unique_entities: usize,
    total_knowledge_points: usize,
    total_relations: usize,
    validation_violation_count: usize,
}

#[derive(Debug, Serialize)]
struct StageBSummary {
    total_bundles: usize,
    successful_mcqs: usize,
    failed_mcqs: usize,
    recall_mcqs: usize,
    relational_mcqs: usize,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_dotenv_if_present();

    let args = parse_args()?;
    fs::create_dir_all(&args.output_dir)?;

    let (llm, verifier_summary) = llm_service_from_env()?;
    let llm = llm.with_retry_observer(|event| match event.state {
        LlmRetryState::Waiting => println!(
            "[llm-retry] {} | attempt {}/{} starts in {}s",
            event.failure.message,
            event.next_attempt,
            event.max_attempts,
            event.delay.as_secs()
        ),
        LlmRetryState::Retrying => println!(
            "[llm-retry] Starting attempt {}/{}",
            event.next_attempt, event.max_attempts
        ),
    });
    let (embedding_service, embedding_summary) = embedding_service_from_env()?;
    let resolution_config = EntityResolutionConfig {
        max_points_per_entity: args.max_points_per_entity,
        candidates: CandidateConfig {
            minimum_similarity: args.minimum_similarity,
            max_candidates_per_entity: args.max_candidates_per_entity,
        },
        verifier: VerifierConfig {
            max_concurrency: args.max_concurrency,
            ..VerifierConfig::default()
        },
    };
    let note_path = args
        .note_path
        .to_str()
        .ok_or("Note path contains invalid UTF-8")?;

    println!("Graph E2E Eval");
    println!("Note: {}", args.note_path.display());
    println!(
        "Verifier: {}/{} (timeout {}s per attempt)",
        verifier_summary.provider, verifier_summary.model, verifier_summary.timeout_secs
    );
    println!(
        "Embedding: {}/{}",
        embedding_summary.provider, embedding_summary.model
    );
    println!(
        "Entity resolution: similarity >= {:.3}, top {}, concurrency {}",
        resolution_config.candidates.minimum_similarity,
        resolution_config.candidates.max_candidates_per_entity,
        resolution_config.verifier.max_concurrency
    );
    println!("---");

    let total_started = Instant::now();
    println!("Running Stage A...");
    let stage_a_started = Instant::now();
    let stage_a = run_graph_stage_a_with_progress(note_path, &llm, |progress| match progress {
        GraphStageAProgress::ChunksPrepared { total_chunks } => println!(
            "[stage-a] Prepared {total_chunks} chunks; LLM extraction runs sequentially"
        ),
        GraphStageAProgress::ChunkStarted {
            chunk_number,
            total_chunks,
            heading,
        } => println!(
            "[stage-a {chunk_number}/{total_chunks}] Sending chunk to LLM | heading={heading:?}"
        ),
        GraphStageAProgress::ChunkCompleted {
            chunk_number,
            total_chunks,
            heading,
            status,
            attempts,
            elapsed_ms,
        } => println!(
            "[stage-a {chunk_number}/{total_chunks}] Completed | heading={heading:?} status={status} attempts={} elapsed={:.1}s",
            attempts
                .map(|value| value.to_string())
                .unwrap_or_else(|| String::from("n/a")),
            elapsed_ms as f64 / 1_000.0
        ),
        GraphStageAProgress::Consolidating {
            successful_chunks,
            failed_chunks,
        } => println!(
            "[stage-a] Consolidating chunks | successful={successful_chunks} failed={failed_chunks}"
        ),
    })
    .await?;
    let stage_a_ms = stage_a_started.elapsed().as_millis();
    let stage_a_summary = summarize_stage_a(&stage_a);
    print_stage_a_summary(&stage_a_summary);
    print_stage_timing("Stage A", stage_a_ms);

    println!("---");
    println!("Running entity resolution...");
    let entity_names = stage_a
        .graph
        .entities
        .iter()
        .map(|entity| (entity.id.clone(), entity.canonical_name.clone()))
        .collect::<HashMap<_, _>>();
    let resolution_started = Instant::now();
    let mut resolution_trace = ResolutionTrace::default();
    let resolution = resolve_graph_entities_with_progress(
        &stage_a.graph,
        &embedding_service,
        &llm,
        &resolution_config,
        |progress| match progress {
            EntityResolutionProgress::GeneratingEmbeddings { entity_count } => {
                println!("[embed] Generating embeddings for {entity_count} entities");
            }
            EntityResolutionProgress::CandidatesGenerated { candidate_count } => {
                resolution_trace.embedding_and_candidate_ms = resolution_started.elapsed().as_millis();
                println!("[candidate] Generated {candidate_count} candidate pairs");
            }
            EntityResolutionProgress::CandidateSelected {
                position,
                total_pairs,
                entity_id,
                candidate_entity_id,
                similarity,
            } => {
                println!(
                    "[candidate {position}/{total_pairs}] {} ({entity_id}) ↔ {} ({candidate_entity_id}) | similarity={similarity:.4}",
                    entity_name(&entity_names, &entity_id),
                    entity_name(&entity_names, &candidate_entity_id),
                );
            }
            EntityResolutionProgress::VerifyingCandidates {
                completed_pairs,
                total_pairs,
                in_flight_pairs,
                entity_id,
                candidate_entity_id,
                similarity,
                decision,
            } => {
                let elapsed_ms = resolution_started.elapsed().as_millis();
                println!(
                    "[verify {completed_pairs}/{total_pairs}] {} ↔ {} | similarity={similarity:.4} decision={} active={in_flight_pairs}",
                    entity_name(&entity_names, &entity_id),
                    entity_name(&entity_names, &candidate_entity_id),
                    decision_label(decision),
                );
                resolution_trace.verification_progress.push(VerificationProgressReport {
                    completed_pairs,
                    total_pairs,
                    in_flight_pairs,
                    entity_id,
                    candidate_entity_id,
                    similarity,
                    decision: decision_label(decision).to_string(),
                    elapsed_ms,
                });
            }
            EntityResolutionProgress::Finalizing { verified_pair_count } => {
                let elapsed_ms = resolution_started.elapsed().as_millis();
                resolution_trace.verification_ms = elapsed_ms
                    .saturating_sub(resolution_trace.embedding_and_candidate_ms);
                resolution_trace.finalization_started_ms = elapsed_ms;
                println!("[merge] Applying decisions from {verified_pair_count} verified pairs");
            }
        },
    )
    .await?;
    let entity_resolution_ms = resolution_started.elapsed().as_millis();
    let finalization_ms =
        entity_resolution_ms.saturating_sub(resolution_trace.finalization_started_ms);
    print_entity_resolution_summary(&resolution.metrics);
    print_stage_timing("Entity resolution", entity_resolution_ms);
    print_stage_timing(
        "  Embedding + candidate generation",
        resolution_trace.embedding_and_candidate_ms,
    );
    print_stage_timing("  LLM verification", resolution_trace.verification_ms);
    print_stage_timing("  Merge + graph rewrite", finalization_ms);

    println!("---");
    println!("Running Stage B from the resolved graph...");
    let stage_b_started = Instant::now();
    let stage_b = run_graph_stage_b_for_graph(&resolution.graph, &llm).await?;
    let stage_b_ms = stage_b_started.elapsed().as_millis();
    let stage_b_summary = summarize_stage_b(&stage_b);
    print_stage_b_summary(&stage_b_summary);
    let total_ms = total_started.elapsed().as_millis();
    print_stage_timing("Stage B", stage_b_ms);
    print_stage_timing("Pipeline total", total_ms);

    let candidates = resolution
        .candidates
        .iter()
        .zip(&resolution.verified_candidates)
        .enumerate()
        .map(|(index, (candidate, verified))| EntityCandidateReport {
            position: index + 1,
            entity_id: candidate.entity_id.clone(),
            entity_name: entity_name(&entity_names, &candidate.entity_id).to_string(),
            candidate_entity_id: candidate.candidate_entity_id.clone(),
            candidate_entity_name: entity_name(&entity_names, &candidate.candidate_entity_id)
                .to_string(),
            similarity: candidate.similarity,
            decision: decision_label(verified.decision).to_string(),
            reason: verified.reason.clone(),
        })
        .collect();
    let merge_groups = resolution
        .merge_plan
        .merges
        .iter()
        .map(|group| MergeGroupReport {
            canonical_entity_id: group.canonical_entity_id.clone(),
            merged_entity_ids: group.merged_entity_ids.clone(),
        })
        .collect();
    let resolution_metrics = resolution.metrics;
    let entity_resolution = EntityResolutionReport {
        config: EntityResolutionConfigReport {
            minimum_similarity: resolution_config.candidates.minimum_similarity,
            max_candidates_per_entity: resolution_config.candidates.max_candidates_per_entity,
            max_points_per_entity: resolution_config.max_points_per_entity,
            max_reason_chars: resolution_config.verifier.max_reason_chars,
            max_concurrency: resolution_config.verifier.max_concurrency,
        },
        summary: EntityResolutionSummary {
            entities_before: resolution_metrics.entity_count_before,
            entities_after: resolution_metrics.entity_count_after,
            candidate_pairs: resolution_metrics.candidate_pair_count,
            same_entity: resolution_metrics.same_entity_count,
            different_entity: resolution_metrics.different_entity_count,
            uncertain: resolution_metrics.unresolved_pair_count,
            merge_groups: resolution_metrics.merge_group_count,
        },
        candidates,
        verification_progress: std::mem::take(&mut resolution_trace.verification_progress),
        merge_groups,
        resolved_graph: resolution.graph,
    };

    let timestamp = Utc::now();
    let stem = args
        .note_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("evaluation")
        .to_ascii_lowercase();
    let file_prefix = format!("graph-e2e-{}-{}", stem, timestamp.format("%Y%m%d-%H%M%S"));

    let json_path = args.output_dir.join(format!("{file_prefix}.json"));
    let xlsx_path = args.output_dir.join(format!("{file_prefix}.xlsx"));

    let run = GraphE2eEvalRun {
        generated_at_utc: timestamp.to_rfc3339(),
        provider: verifier_summary.provider,
        base_url: verifier_summary.base_url,
        model: verifier_summary.model,
        note_path: args.note_path.display().to_string(),
        json_path: json_path.display().to_string(),
        xlsx_path: xlsx_path.display().to_string(),
        embedding: embedding_summary,
        timings: EvalTimings {
            total_ms,
            stage_a_ms,
            entity_resolution_ms,
            embedding_and_candidate_ms: resolution_trace.embedding_and_candidate_ms,
            verification_ms: resolution_trace.verification_ms,
            finalization_ms,
            stage_b_ms,
        },
        stage_a_summary,
        entity_resolution,
        stage_b_summary,
        stage_a,
        stage_b,
    };

    let output_started = Instant::now();
    fs::write(&json_path, serde_json::to_string_pretty(&run)?)?;
    write_xlsx(&xlsx_path, &run)?;
    let output_ms = output_started.elapsed().as_millis();

    println!("---");
    println!("Graph E2E evaluation completed.");
    print_stage_timing("Report writing", output_ms);
    print_stage_timing("Wall-clock total", total_started.elapsed().as_millis());
    println!("JSON: {}", json_path.display());
    println!("XLSX: {}", xlsx_path.display());

    Ok(())
}

fn summarize_stage_a(stage_a: &GraphStageAResult) -> StageASummary {
    StageASummary {
        total_chunks: stage_a.total_chunks,
        successful_chunks: stage_a.successful_chunks,
        failed_chunks: stage_a.failed_chunks,
        total_raw_mentions: stage_a
            .chunks
            .iter()
            .filter_map(|chunk| chunk.entity_count)
            .sum(),
        unique_entities: stage_a.graph.entities.len(),
        total_knowledge_points: stage_a.graph.knowledge_points.len(),
        total_relations: stage_a.graph.relations.len(),
        validation_violation_count: stage_a.validation_violations.len(),
    }
}

fn summarize_stage_b(stage_b: &GraphStageBResult) -> StageBSummary {
    let mut recall_mcqs = 0;
    let mut relational_mcqs = 0;

    for item in &stage_b.items {
        let Some(mcq) = &item.mcq else {
            continue;
        };
        match mcq.question_type {
            services::graph_generation::types::QuestionType::Recall => recall_mcqs += 1,
            services::graph_generation::types::QuestionType::Relational => relational_mcqs += 1,
        }
    }

    StageBSummary {
        total_bundles: stage_b.total_bundles,
        successful_mcqs: stage_b.successful_mcqs,
        failed_mcqs: stage_b.failed_mcqs,
        recall_mcqs,
        relational_mcqs,
    }
}

fn print_stage_a_summary(summary: &StageASummary) {
    println!(
        "Stage A: {}/{} chunks parsed successfully",
        summary.successful_chunks, summary.total_chunks
    );
    println!(
        "Stage A graph: {} raw mentions -> {} unique entities, {} knowledge points, {} relations",
        summary.total_raw_mentions,
        summary.unique_entities,
        summary.total_knowledge_points,
        summary.total_relations
    );
    println!(
        "Stage A validation violations: {}",
        summary.validation_violation_count
    );
}

fn print_stage_timing(stage: &str, elapsed_ms: u128) {
    println!("[timing] {stage}: {}", format_duration(elapsed_ms));
}

fn format_duration(elapsed_ms: u128) -> String {
    if elapsed_ms < 1_000 {
        return format!("{elapsed_ms}ms");
    }

    let elapsed_seconds = elapsed_ms as f64 / 1_000.0;
    if elapsed_seconds < 60.0 {
        return format!("{elapsed_seconds:.1}s");
    }

    let minutes = (elapsed_seconds / 60.0).floor() as u64;
    let seconds = elapsed_seconds - minutes as f64 * 60.0;
    format!("{minutes}m {seconds:.1}s")
}

fn print_entity_resolution_summary(
    metrics: &services::graph_generation::entity_resolution::pipeline::EntityResolutionMetrics,
) {
    println!(
        "Entity resolution: {} → {} entities, {} candidates, {} same, {} different, {} uncertain, {} merge groups",
        metrics.entity_count_before,
        metrics.entity_count_after,
        metrics.candidate_pair_count,
        metrics.same_entity_count,
        metrics.different_entity_count,
        metrics.unresolved_pair_count,
        metrics.merge_group_count
    );
}

fn print_stage_b_summary(summary: &StageBSummary) {
    println!(
        "Stage B: {}/{} MCQs generated successfully",
        summary.successful_mcqs, summary.total_bundles
    );
    println!(
        "Stage B question types: {} recall, {} relational",
        summary.recall_mcqs, summary.relational_mcqs
    );
}

fn entity_name<'a>(entity_names: &'a HashMap<String, String>, entity_id: &'a str) -> &'a str {
    entity_names
        .get(entity_id)
        .map(String::as_str)
        .unwrap_or(entity_id)
}

fn decision_label(decision: EntityMatchDecision) -> &'static str {
    match decision {
        EntityMatchDecision::SameEntity => "same_entity",
        EntityMatchDecision::DifferentEntity => "different_entity",
        EntityMatchDecision::Uncertain => "uncertain",
    }
}

fn write_xlsx(path: &Path, run: &GraphE2eEvalRun) -> Result<(), Box<dyn Error>> {
    let mut workbook = Workbook::new();
    write_summary_sheet(&mut workbook, run)?;
    write_stage_a_chunks_sheet(&mut workbook, &run.stage_a)?;
    write_entities_sheet(&mut workbook, &run.stage_a)?;
    write_entity_resolution_sheet(&mut workbook, &run.entity_resolution)?;
    write_verification_progress_sheet(&mut workbook, &run.entity_resolution)?;
    write_resolved_entities_sheet(&mut workbook, &run.entity_resolution)?;
    write_stage_b_mcqs_sheet(&mut workbook, &run.stage_b)?;
    workbook.save(path)?;
    Ok(())
}

fn write_summary_sheet(
    workbook: &mut Workbook,
    run: &GraphE2eEvalRun,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Summary")?;
    let rows = [
        ("generated_at_utc", run.generated_at_utc.as_str()),
        ("verifier_provider", run.provider.as_str()),
        ("verifier_base_url", run.base_url.as_str()),
        ("verifier_model", run.model.as_str()),
        ("embedding_provider", run.embedding.provider.as_str()),
        ("embedding_base_url", run.embedding.base_url.as_str()),
        ("embedding_model", run.embedding.model.as_str()),
        ("note_path", run.note_path.as_str()),
        ("json_path", run.json_path.as_str()),
        ("xlsx_path", run.xlsx_path.as_str()),
    ];

    for (row, (key, value)) in rows.iter().enumerate() {
        sheet.write_string(row as u32, 0, *key)?;
        sheet.write_string(row as u32, 1, *value)?;
    }

    let start = rows.len() as u32 + 2;
    sheet.write_string(start, 0, "stage_a_total_chunks")?;
    sheet.write_number(start, 1, run.stage_a_summary.total_chunks as f64)?;
    sheet.write_string(start + 1, 0, "stage_a_successful_chunks")?;
    sheet.write_number(start + 1, 1, run.stage_a_summary.successful_chunks as f64)?;
    sheet.write_string(start + 2, 0, "stage_a_failed_chunks")?;
    sheet.write_number(start + 2, 1, run.stage_a_summary.failed_chunks as f64)?;
    sheet.write_string(start + 3, 0, "stage_a_unique_entities")?;
    sheet.write_number(start + 3, 1, run.stage_a_summary.unique_entities as f64)?;
    sheet.write_string(start + 4, 0, "stage_a_knowledge_points")?;
    sheet.write_number(
        start + 4,
        1,
        run.stage_a_summary.total_knowledge_points as f64,
    )?;
    sheet.write_string(start + 5, 0, "stage_a_relations")?;
    sheet.write_number(start + 5, 1, run.stage_a_summary.total_relations as f64)?;

    let start_resolution = start + 8;
    sheet.write_string(start_resolution, 0, "resolution_entities_before")?;
    sheet.write_number(
        start_resolution,
        1,
        run.entity_resolution.summary.entities_before as f64,
    )?;
    sheet.write_string(start_resolution + 1, 0, "resolution_entities_after")?;
    sheet.write_number(
        start_resolution + 1,
        1,
        run.entity_resolution.summary.entities_after as f64,
    )?;
    sheet.write_string(start_resolution + 2, 0, "resolution_candidate_pairs")?;
    sheet.write_number(
        start_resolution + 2,
        1,
        run.entity_resolution.summary.candidate_pairs as f64,
    )?;
    sheet.write_string(start_resolution + 3, 0, "resolution_same_entity")?;
    sheet.write_number(
        start_resolution + 3,
        1,
        run.entity_resolution.summary.same_entity as f64,
    )?;
    sheet.write_string(start_resolution + 4, 0, "resolution_different_entity")?;
    sheet.write_number(
        start_resolution + 4,
        1,
        run.entity_resolution.summary.different_entity as f64,
    )?;
    sheet.write_string(start_resolution + 5, 0, "resolution_uncertain")?;
    sheet.write_number(
        start_resolution + 5,
        1,
        run.entity_resolution.summary.uncertain as f64,
    )?;
    sheet.write_string(start_resolution + 6, 0, "resolution_total_ms")?;
    sheet.write_number(
        start_resolution + 6,
        1,
        run.timings.entity_resolution_ms as f64,
    )?;

    let start_b = start_resolution + 9;
    sheet.write_string(start_b, 0, "stage_b_total_bundles")?;
    sheet.write_number(start_b, 1, run.stage_b_summary.total_bundles as f64)?;
    sheet.write_string(start_b + 1, 0, "stage_b_successful_mcqs")?;
    sheet.write_number(start_b + 1, 1, run.stage_b_summary.successful_mcqs as f64)?;
    sheet.write_string(start_b + 2, 0, "stage_b_failed_mcqs")?;
    sheet.write_number(start_b + 2, 1, run.stage_b_summary.failed_mcqs as f64)?;
    sheet.write_string(start_b + 3, 0, "stage_b_recall_mcqs")?;
    sheet.write_number(start_b + 3, 1, run.stage_b_summary.recall_mcqs as f64)?;
    sheet.write_string(start_b + 4, 0, "stage_b_relational_mcqs")?;
    sheet.write_number(start_b + 4, 1, run.stage_b_summary.relational_mcqs as f64)?;

    Ok(())
}

fn write_stage_a_chunks_sheet(
    workbook: &mut Workbook,
    stage_a: &GraphStageAResult,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Stage A Chunks")?;
    let headers = [
        "chunk_index",
        "heading",
        "status",
        "attempts",
        "entity_count",
        "point_count",
        "relation_count",
        "entities",
        "knowledge_points",
        "error",
        "content_preview",
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string(0, col as u16, *header)?;
    }

    for (idx, chunk) in stage_a.chunks.iter().enumerate() {
        let row = (idx + 1) as u32;
        sheet.write_number(row, 0, chunk.chunk_index as f64)?;
        sheet.write_string(row, 1, &chunk.heading)?;
        sheet.write_string(row, 2, &chunk.status)?;
        if let Some(attempts) = chunk.attempts {
            sheet.write_number(row, 3, attempts as f64)?;
        }
        if let Some(count) = chunk.entity_count {
            sheet.write_number(row, 4, count as f64)?;
        }
        if let Some(count) = chunk.point_count {
            sheet.write_number(row, 5, count as f64)?;
        }
        if let Some(count) = chunk.relation_count {
            sheet.write_number(row, 6, count as f64)?;
        }
        if let Some(entities) = &chunk.entities {
            sheet.write_string(row, 7, entities.join(", "))?;
        }
        if let Some(points) = &chunk.knowledge_points {
            let summary = points
                .iter()
                .map(|point| format!("[{}] {}", point.knowledge_type, truncate(&point.point, 80)))
                .collect::<Vec<_>>()
                .join(" | ");
            sheet.write_string(row, 8, summary)?;
        }
        sheet.write_string(row, 9, chunk.error.as_deref().unwrap_or(""))?;
        sheet.write_string(row, 10, &chunk.content_preview)?;
    }

    Ok(())
}

fn write_entities_sheet(
    workbook: &mut Workbook,
    stage_a: &GraphStageAResult,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Entities Before")?;
    let headers = [
        "id",
        "canonical_name",
        "aliases",
        "chunk_count",
        "chunk_ids",
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string(0, col as u16, *header)?;
    }

    for (idx, entity) in stage_a.graph.entities.iter().enumerate() {
        let row = (idx + 1) as u32;
        sheet.write_string(row, 0, &entity.id)?;
        sheet.write_string(row, 1, &entity.canonical_name)?;
        sheet.write_string(row, 2, entity.aliases.join(", "))?;
        sheet.write_number(row, 3, entity.chunk_ids.len() as f64)?;
        sheet.write_string(row, 4, entity.chunk_ids.join(", "))?;
    }

    Ok(())
}

fn write_entity_resolution_sheet(
    workbook: &mut Workbook,
    resolution: &EntityResolutionReport,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Entity Resolution")?;
    let headers = [
        "position",
        "entity_id",
        "entity_name",
        "candidate_entity_id",
        "candidate_entity_name",
        "embedding_similarity",
        "llm_decision",
        "llm_reason",
    ];
    for (column, header) in headers.iter().enumerate() {
        sheet.write_string(0, column as u16, *header)?;
    }

    for candidate in &resolution.candidates {
        let row = candidate.position as u32;
        sheet.write_number(row, 0, candidate.position as f64)?;
        sheet.write_string(row, 1, &candidate.entity_id)?;
        sheet.write_string(row, 2, &candidate.entity_name)?;
        sheet.write_string(row, 3, &candidate.candidate_entity_id)?;
        sheet.write_string(row, 4, &candidate.candidate_entity_name)?;
        sheet.write_number(row, 5, candidate.similarity as f64)?;
        sheet.write_string(row, 6, &candidate.decision)?;
        sheet.write_string(row, 7, &candidate.reason)?;
    }

    Ok(())
}

fn write_verification_progress_sheet(
    workbook: &mut Workbook,
    resolution: &EntityResolutionReport,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Verification Progress")?;
    let headers = [
        "completion_order",
        "total_pairs",
        "requests_active",
        "entity_id",
        "candidate_entity_id",
        "embedding_similarity",
        "llm_decision",
        "elapsed_ms",
    ];
    for (column, header) in headers.iter().enumerate() {
        sheet.write_string(0, column as u16, *header)?;
    }

    for progress in &resolution.verification_progress {
        let row = progress.completed_pairs as u32;
        sheet.write_number(row, 0, progress.completed_pairs as f64)?;
        sheet.write_number(row, 1, progress.total_pairs as f64)?;
        sheet.write_number(row, 2, progress.in_flight_pairs as f64)?;
        sheet.write_string(row, 3, &progress.entity_id)?;
        sheet.write_string(row, 4, &progress.candidate_entity_id)?;
        sheet.write_number(row, 5, progress.similarity as f64)?;
        sheet.write_string(row, 6, &progress.decision)?;
        sheet.write_number(row, 7, progress.elapsed_ms as f64)?;
    }

    Ok(())
}

fn write_resolved_entities_sheet(
    workbook: &mut Workbook,
    resolution: &EntityResolutionReport,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Entities After")?;
    let headers = [
        "id",
        "canonical_name",
        "aliases",
        "chunk_count",
        "chunk_ids",
    ];
    for (column, header) in headers.iter().enumerate() {
        sheet.write_string(0, column as u16, *header)?;
    }

    for (index, entity) in resolution.resolved_graph.entities.iter().enumerate() {
        let row = (index + 1) as u32;
        sheet.write_string(row, 0, &entity.id)?;
        sheet.write_string(row, 1, &entity.canonical_name)?;
        sheet.write_string(row, 2, entity.aliases.join(", "))?;
        sheet.write_number(row, 3, entity.chunk_ids.len() as f64)?;
        sheet.write_string(row, 4, entity.chunk_ids.join(", "))?;
    }

    Ok(())
}

fn write_stage_b_mcqs_sheet(
    workbook: &mut Workbook,
    stage_b: &GraphStageBResult,
) -> Result<(), Box<dyn Error>> {
    let sheet = workbook.add_worksheet().set_name("Stage B MCQs")?;
    let headers = [
        "bundle_index",
        "status",
        "question_type",
        "root_point",
        "question",
        "option_a",
        "option_b",
        "option_c",
        "option_d",
        "correct_answer",
        "explanation",
        "error",
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string(0, col as u16, *header)?;
    }

    for (idx, item) in stage_b.items.iter().enumerate() {
        let row = (idx + 1) as u32;
        sheet.write_number(row, 0, item.bundle_index as f64)?;
        sheet.write_string(row, 1, &item.status)?;
        sheet.write_string(row, 3, &item.bundle.root_point.point)?;
        if let Some(mcq) = &item.mcq {
            sheet.write_string(row, 2, format!("{:?}", mcq.question_type))?;
            sheet.write_string(row, 4, &mcq.question)?;
            sheet.write_string(
                row,
                5,
                mcq.options.first().map(String::as_str).unwrap_or(""),
            )?;
            sheet.write_string(row, 6, mcq.options.get(1).map(String::as_str).unwrap_or(""))?;
            sheet.write_string(row, 7, mcq.options.get(2).map(String::as_str).unwrap_or(""))?;
            sheet.write_string(row, 8, mcq.options.get(3).map(String::as_str).unwrap_or(""))?;
            sheet.write_string(row, 9, answer_label(mcq.correct_index))?;
            sheet.write_string(row, 10, &mcq.explanation)?;
        }
        sheet.write_string(row, 11, item.error.as_deref().unwrap_or(""))?;
    }

    Ok(())
}

fn answer_label(index: usize) -> &'static str {
    match index {
        0 => "A",
        1 => "B",
        2 => "C",
        3 => "D",
        _ => "",
    }
}

fn parse_args() -> Result<CliArgs, Box<dyn Error>> {
    let repo_root = repo_root();
    let default_note_path = repo_root.join(DEFAULT_NOTE_RELATIVE_PATH);
    let default_output_dir = repo_root.join(DEFAULT_OUTPUT_DIR_NAME);

    let mut note_path = default_note_path;
    let mut output_dir = default_output_dir;
    let mut minimum_similarity = 0.5;
    let mut max_candidates_per_entity = 3;
    let mut max_points_per_entity = 3;
    let mut max_concurrency = 5;
    let mut args = env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--note" => {
                let value = args.next().ok_or("Missing value for --note")?;
                note_path = PathBuf::from(value);
            }
            "--output-dir" => {
                let value = args.next().ok_or("Missing value for --output-dir")?;
                output_dir = PathBuf::from(value);
            }
            "--minimum-similarity" => {
                minimum_similarity = next_arg(&mut args, "--minimum-similarity")?.parse()?
            }
            "--max-candidates" => {
                max_candidates_per_entity = next_arg(&mut args, "--max-candidates")?.parse()?
            }
            "--max-context-points" => {
                max_points_per_entity = next_arg(&mut args, "--max-context-points")?.parse()?
            }
            "--max-concurrency" => {
                max_concurrency = next_arg(&mut args, "--max-concurrency")?.parse()?
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            unexpected => {
                return Err(format!("Unsupported argument: {unexpected}").into());
            }
        }
    }

    Ok(CliArgs {
        note_path,
        output_dir,
        minimum_similarity,
        max_candidates_per_entity,
        max_points_per_entity,
        max_concurrency,
    })
}

fn print_usage() {
    println!(
        "Usage: cargo run --manifest-path eval/Cargo.toml --bin graph_e2e_eval -- [--note <markdown-file>] [--output-dir <dir>] [--minimum-similarity <f32>] [--max-candidates <usize>] [--max-context-points <usize>] [--max-concurrency <usize>]"
    );
}

fn next_arg(
    args: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, Box<dyn Error>> {
    args.next()
        .ok_or_else(|| format!("Missing value for {option}").into())
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_dotenv_if_present() {
    let env_path = repo_root().join("src-tauri/.env");
    let _ = dotenvy::from_path(env_path);
}

fn required_env(name: &str) -> Result<String, Box<dyn Error>> {
    let value =
        env::var(name).map_err(|_| format!("Required environment variable {name} is not set"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("Required environment variable {name} is empty").into());
    }
    Ok(trimmed.to_string())
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
            timeout_secs,
        },
    ))
}

fn llm_service_from_env() -> Result<(LlmService, ProviderSummary), Box<dyn Error>> {
    let provider_name = env::var("LLM_PROVIDER").unwrap_or_else(|_| String::from("openrouter"));
    let provider = parse_llm_provider(&provider_name)?;
    let provider_label = llm_provider_label(provider);
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
            timeout_secs,
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

fn optional_u64_env(name: &str, default: u64) -> Result<u64, Box<dyn Error>> {
    let Some(value) = env::var(name).ok() else {
        return Ok(default);
    };
    let parsed = value
        .trim()
        .parse::<u64>()
        .map_err(|error| format!("{name} must be a positive integer: {error}"))?;
    if parsed == 0 {
        return Err(format!("{name} must be greater than zero").into());
    }
    Ok(parsed)
}

fn truncate(s: &str, max_chars: usize) -> String {
    let chars_count = s.chars().count();
    if chars_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}
