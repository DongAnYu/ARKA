#![allow(dead_code)]

#[path = "../../../src-tauri/src/models/mod.rs"]
mod models;
#[path = "../../../src-tauri/src/services/mod.rs"]
mod services;

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rust_xlsxwriter::Workbook;
use serde::Serialize;

use services::graph_generation::pipeline::{run_graph_stage_a, GraphStageAChunkResult};
use services::llm::LlmService;

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

// =====================================================================
// Data structures
// =====================================================================

#[derive(Debug, Clone)]
struct CliArgs {
    note_path: PathBuf,
    output_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct StageAEvalRun {
    generated_at_utc: String,
    provider: String,
    base_url: String,
    model: String,
    note_path: String,
    json_path: String,
    xlsx_path: String,
    total_chunks: usize,
    successful_parses: usize,
    failed_parses: usize,
    aggregate: AggregateStats,
    consolidation: ConsolidationStats,
    chunks: Vec<GraphStageAChunkResult>,
}

#[derive(Debug, Serialize)]
struct ConsolidationStats {
    total_raw_mentions: usize,
    unique_entities: usize,
    /// unique_entities / total_raw_mentions — lower means more dedup happened
    dedup_ratio: f64,
    kp_with_entity_ids: usize,
    total_knowledge_points: usize,
    total_relations: usize,
    validation_violations: Vec<String>,
    entities: Vec<EntitySummary>,
}

#[derive(Debug, Serialize)]
struct EntitySummary {
    id: String,
    canonical_name: String,
    aliases: Vec<String>,
    chunk_count: usize,
    chunk_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AggregateStats {
    avg_entities_per_chunk: f64,
    avg_points_per_chunk: f64,
    avg_relations_per_point: f64,
    knowledge_type_distribution: KnowledgeTypeDistribution,
    validation_error_breakdown: Vec<String>,
}

#[derive(Debug, Default, Serialize)]
struct KnowledgeTypeDistribution {
    definition: usize,
    fact: usize,
    procedural: usize,
    conceptual: usize,
}

// =====================================================================
// Main
// =====================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_dotenv_if_present();

    let args = parse_args()?;
    fs::create_dir_all(&args.output_dir)?;

    let base_url = env::var("OPENROUTER_BASE_URL")
        .or_else(|_| env::var("LLM_BASE_URL"))
        .unwrap_or_else(|_| String::from(DEFAULT_OPENROUTER_BASE_URL));
    let model = required_env("LLM_MODEL")?;
    let api_key = required_env("OPENROUTER_API_KEY")?;
    let timeout_secs = env::var("LLM_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TIMEOUT_SECS);

    services::llm::set_runtime_llm_config(
        "openrouter",
        &base_url,
        &model,
        timeout_secs,
        Some(&api_key),
    )?;

    let llm = LlmService::from_runtime_or_env()?;
    let note_path = args
        .note_path
        .to_str()
        .ok_or("Note path contains invalid UTF-8")?;
    let stage_a = run_graph_stage_a(note_path, &llm).await?;

    println!(
        "Stage A Eval: {} chunks from '{}'",
        stage_a.total_chunks, stage_a.note_title
    );
    println!("Model: {model}");
    println!("---");

    for (idx, chunk) in stage_a.chunks.iter().enumerate() {
        match chunk.status.as_str() {
            "success" => println!(
                "  Chunk {}/{} [{}]... OK ({} entities, {} points, {} relations, {} attempt{})",
                idx + 1,
                stage_a.total_chunks,
                chunk.heading,
                chunk.entity_count.unwrap_or(0),
                chunk.point_count.unwrap_or(0),
                chunk.relation_count.unwrap_or(0),
                chunk.attempts.unwrap_or(1),
                if chunk.attempts == Some(1) { "" } else { "s" }
            ),
            _ => println!(
                "  Chunk {}/{} [{}]... LLM ERROR: {}",
                idx + 1,
                stage_a.total_chunks,
                chunk.heading,
                chunk.error.as_deref().unwrap_or("unknown error")
            ),
        }
    }

    // Aggregate stats
    let chunk_results = stage_a.chunks.clone();
    let successful: Vec<&GraphStageAChunkResult> = chunk_results
        .iter()
        .filter(|c| c.status == "success")
        .collect();
    let successful_count = stage_a.successful_chunks;
    let failed_count = stage_a.failed_chunks;

    let total_entities: usize = successful.iter().filter_map(|c| c.entity_count).sum();
    let total_points: usize = successful.iter().filter_map(|c| c.point_count).sum();
    let total_relations: usize = successful.iter().filter_map(|c| c.relation_count).sum();

    let avg_entities = if successful_count > 0 {
        total_entities as f64 / successful_count as f64
    } else {
        0.0
    };
    let avg_points = if successful_count > 0 {
        total_points as f64 / successful_count as f64
    } else {
        0.0
    };
    let avg_relations = if total_points > 0 {
        total_relations as f64 / total_points as f64
    } else {
        0.0
    };

    // Knowledge type distribution
    let mut kt_dist = KnowledgeTypeDistribution::default();
    for chunk_r in &successful {
        if let Some(points) = &chunk_r.knowledge_points {
            for p in points {
                match p.knowledge_type.as_str() {
                    "Definition" => kt_dist.definition += 1,
                    "Fact" => kt_dist.fact += 1,
                    "Procedural" => kt_dist.procedural += 1,
                    "Conceptual" => kt_dist.conceptual += 1,
                    _ => {}
                }
            }
        }
    }

    // Validation error breakdown
    let validation_errors: Vec<String> = chunk_results
        .iter()
        .filter(|c| c.status != "success")
        .filter_map(|c| c.error.clone())
        .collect();

    let aggregate = AggregateStats {
        avg_entities_per_chunk: avg_entities,
        avg_points_per_chunk: avg_points,
        avg_relations_per_point: avg_relations,
        knowledge_type_distribution: kt_dist,
        validation_error_breakdown: validation_errors,
    };

    // ── Consolidation pass ────────────────────────────────────────────────
    let total_raw_mentions: usize = chunk_results
        .iter()
        .filter_map(|chunk| chunk.entity_count)
        .sum();

    let kp_with_ids = stage_a
        .graph
        .knowledge_points
        .iter()
        .filter(|kp| !kp.entity_ids.is_empty())
        .count();

    let dedup_ratio = if total_raw_mentions > 0 {
        stage_a.graph.entities.len() as f64 / total_raw_mentions as f64
    } else {
        1.0
    };

    let entity_summaries: Vec<EntitySummary> = stage_a
        .graph
        .entities
        .iter()
        .map(|e| EntitySummary {
            id: e.id.clone(),
            canonical_name: e.canonical_name.clone(),
            aliases: e.aliases.clone(),
            chunk_count: e.chunk_ids.len(),
            chunk_ids: e.chunk_ids.clone(),
        })
        .collect();

    let consolidation = ConsolidationStats {
        total_raw_mentions,
        unique_entities: stage_a.graph.entities.len(),
        dedup_ratio,
        kp_with_entity_ids: kp_with_ids,
        total_knowledge_points: stage_a.graph.knowledge_points.len(),
        total_relations: stage_a.graph.relations.len(),
        validation_violations: stage_a.validation_violations.clone(),
        entities: entity_summaries,
    };

    // Output
    let timestamp = Utc::now();
    let stem = args
        .note_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("evaluation")
        .to_ascii_lowercase();
    let file_prefix = format!("stage-a-{}-{}", stem, timestamp.format("%Y%m%d-%H%M%S"));

    let json_path = args.output_dir.join(format!("{file_prefix}.json"));
    let xlsx_path = args.output_dir.join(format!("{file_prefix}.xlsx"));

    let run = StageAEvalRun {
        generated_at_utc: timestamp.to_rfc3339(),
        provider: String::from("openrouter"),
        base_url,
        model,
        note_path: args.note_path.display().to_string(),
        json_path: json_path.display().to_string(),
        xlsx_path: xlsx_path.display().to_string(),
        total_chunks: stage_a.total_chunks,
        successful_parses: successful_count,
        failed_parses: failed_count,
        aggregate,
        consolidation,
        chunks: chunk_results.clone(),
    };

    fs::write(&json_path, serde_json::to_string_pretty(&run)?)?;
    write_xlsx(&xlsx_path, &chunk_results, &run.consolidation)?;

    println!("---");
    println!("Stage A evaluation completed.");
    println!("Note: {}", args.note_path.display());
    println!("JSON: {}", json_path.display());
    println!("XLSX: {}", xlsx_path.display());
    println!(
        "Results: {}/{} chunks parsed successfully",
        successful_count, stage_a.total_chunks
    );
    println!(
        "Avg: {:.1} entities/chunk, {:.1} points/chunk, {:.2} relations/point",
        avg_entities, avg_points, avg_relations
    );
    println!("---");
    println!("Consolidation:");
    println!(
        "  Raw mentions: {}  →  Unique entities: {}  (dedup ratio: {:.2})",
        run.consolidation.total_raw_mentions,
        run.consolidation.unique_entities,
        run.consolidation.dedup_ratio
    );
    println!(
        "  KPs with entity_ids: {}/{}  |  Relations after dedup: {}",
        run.consolidation.kp_with_entity_ids,
        run.consolidation.total_knowledge_points,
        run.consolidation.total_relations
    );
    if stage_a.validation_violations.is_empty() {
        println!("  validate_graph: OK");
    } else {
        println!(
            "  validate_graph: {} VIOLATIONS",
            stage_a.validation_violations.len()
        );
        for v in &stage_a.validation_violations {
            println!("    - {v}");
        }
    }

    Ok(())
}

// =====================================================================
// Helpers
// =====================================================================

fn parse_args() -> Result<CliArgs, Box<dyn Error>> {
    let repo_root = repo_root();
    let default_note_path = repo_root.join(DEFAULT_NOTE_RELATIVE_PATH);
    let default_output_dir = repo_root.join(DEFAULT_OUTPUT_DIR_NAME);

    let mut note_path = default_note_path;
    let mut output_dir = default_output_dir;
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
    })
}

fn print_usage() {
    println!(
        "Usage: cargo run --manifest-path eval/Cargo.toml --bin graph_stage_a_eval -- [--note <markdown-file>] [--output-dir <dir>]"
    );
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

fn truncate(s: &str, max_chars: usize) -> String {
    let chars_count = s.chars().count();
    if chars_count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

fn write_xlsx(
    path: &Path,
    chunks: &[GraphStageAChunkResult],
    consolidation: &ConsolidationStats,
) -> Result<(), Box<dyn Error>> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet().set_name("Chunks")?;

    let headers = [
        "chunk_index",
        "heading",
        "status",
        "entity_count",
        "point_count",
        "relation_count",
        "entities",
        "knowledge_points",
        "error",
        "content_preview",
    ];

    for (col, header) in headers.iter().enumerate() {
        worksheet.write_string(0, col as u16, *header)?;
    }

    for (idx, chunk) in chunks.iter().enumerate() {
        let row = (idx + 1) as u32;
        worksheet.write_number(row, 0, chunk.chunk_index as f64)?;
        worksheet.write_string(row, 1, &chunk.heading)?;
        worksheet.write_string(row, 2, &chunk.status)?;

        if let Some(ec) = chunk.entity_count {
            worksheet.write_number(row, 3, ec as f64)?;
        }
        if let Some(pc) = chunk.point_count {
            worksheet.write_number(row, 4, pc as f64)?;
        }
        if let Some(rc) = chunk.relation_count {
            worksheet.write_number(row, 5, rc as f64)?;
        }

        if let Some(entities) = &chunk.entities {
            worksheet.write_string(row, 6, entities.join(", "))?;
        }

        if let Some(points) = &chunk.knowledge_points {
            let summary: Vec<String> = points
                .iter()
                .map(|p| format!("[{}] {}", p.knowledge_type, truncate(&p.point, 80)))
                .collect();
            worksheet.write_string(row, 7, summary.join(" | "))?;
        }

        worksheet.write_string(row, 8, chunk.error.as_deref().unwrap_or(""))?;
        worksheet.write_string(row, 9, &chunk.content_preview)?;
    }

    // ── Entities sheet ───────────────────────────────────────────────────
    let ent_sheet = workbook.add_worksheet().set_name("Entities")?;
    let ent_headers = [
        "id",
        "canonical_name",
        "aliases",
        "chunk_count",
        "chunk_ids",
    ];
    for (col, header) in ent_headers.iter().enumerate() {
        ent_sheet.write_string(0, col as u16, *header)?;
    }
    for (idx, entity) in consolidation.entities.iter().enumerate() {
        let row = (idx + 1) as u32;
        ent_sheet.write_string(row, 0, &entity.id)?;
        ent_sheet.write_string(row, 1, &entity.canonical_name)?;
        ent_sheet.write_string(row, 2, entity.aliases.join(", "))?;
        ent_sheet.write_number(row, 3, entity.chunk_count as f64)?;
        ent_sheet.write_string(row, 4, entity.chunk_ids.join(", "))?;
    }

    workbook.save(path)?;
    Ok(())
}
