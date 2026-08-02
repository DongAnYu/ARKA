#![allow(dead_code)]

#[path = "../models/mod.rs"]
mod models;
#[path = "../services/mod.rs"]
mod services;

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use rust_xlsxwriter::Workbook;
use serde::Serialize;

use services::chunker::chunk_markdown;
use services::graph_generation::consolidator::{consolidate, validate_graph};
use services::graph_generation::stage_a_prompt::format_stage_a_graph_user_prompt;
use services::graph_generation::stage_a_schema::{parse_stage_a_output, stage_a_format_schema};
use services::graph_generation::types::ExtractedKnowledge;
use services::llm::{JsonGenerationRequest, LlmService, LlmServiceError};

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

const STAGE_A_SYSTEM_PROMPT: &str =
    "You are a knowledge graph extraction specialist. Output only valid JSON.";

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
    chunks: Vec<ChunkResult>,
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

#[derive(Debug, Clone, Serialize)]
struct ChunkResult {
    chunk_index: usize,
    heading: String,
    content_preview: String,
    status: String,
    raw_llm_response: Option<String>,
    entity_count: Option<usize>,
    point_count: Option<usize>,
    relation_count: Option<usize>,
    entities: Option<Vec<String>>,
    knowledge_points: Option<Vec<PointSummary>>,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct PointSummary {
    point: String,
    knowledge_type: String,
    raw_entity_names: Vec<String>,
    relation_count: usize,
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

    // Read and chunk the note
    let note_content = fs::read_to_string(&args.note_path)?;
    let note_title = args
        .note_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled");
    let note_path_str = args.note_path.to_str().unwrap_or("");

    let chunks = chunk_markdown(note_path_str, note_title, &note_content);

    println!(
        "Stage A Eval: {} chunks from '{}'",
        chunks.len(),
        note_title
    );
    println!("Model: {model}");
    println!("---");

    let format_schema = stage_a_format_schema();
    let mut chunk_results: Vec<ChunkResult> = Vec::new();
    let mut extracted_chunks: Vec<ExtractedKnowledge> = Vec::new();

    for (idx, chunk) in chunks.iter().enumerate() {
        print!(
            "  Chunk {}/{} [{}]... ",
            idx + 1,
            chunks.len(),
            &chunk.heading
        );

        let user_prompt =
            format_stage_a_graph_user_prompt(&chunk.content, "(no index context in eval mode)");

        let chunk_id = format!("chunk-{}", idx);
        let request = JsonGenerationRequest {
            stage_label: "Graph Stage A Eval",
            system_prompt: STAGE_A_SYSTEM_PROMPT,
            user_prompt: &user_prompt,
            format_schema: format_schema.clone(),
            payload_preview_chars: 800,
        };

        let llm_result = llm
            .generate_json_with_retries(request, |raw_json| {
                parse_stage_a_output(raw_json, chunk_id.clone())
                    .map_err(|err| LlmServiceError::InvalidOutput(err.to_string()))
            })
            .await;

        let chunk_result = match llm_result {
            Ok((extracted, raw_json, attempts)) => {
                let point_summaries: Vec<PointSummary> = extracted
                    .knowledge_points
                    .iter()
                    .map(|kp| PointSummary {
                        point: kp.point.clone(),
                        knowledge_type: format!("{:?}", kp.knowledge_type),
                        raw_entity_names: kp.raw_entity_names.clone(),
                        relation_count: kp.raw_relations.len(),
                    })
                    .collect();

                let total_relations: usize = extracted
                    .knowledge_points
                    .iter()
                    .map(|kp| kp.raw_relations.len())
                    .sum();

                println!(
                    "OK ({} entities, {} points, {} relations, {} attempt{})",
                    extracted.raw_entities.len(),
                    extracted.knowledge_points.len(),
                    total_relations,
                    attempts,
                    if attempts == 1 { "" } else { "s" }
                );

                let result = ChunkResult {
                    chunk_index: idx,
                    heading: chunk.heading.clone(),
                    content_preview: truncate(&chunk.content, 200),
                    status: "success".to_string(),
                    raw_llm_response: Some(raw_json),
                    entity_count: Some(extracted.raw_entities.len()),
                    point_count: Some(extracted.knowledge_points.len()),
                    relation_count: Some(total_relations),
                    entities: Some(
                        extracted
                            .raw_entities
                            .iter()
                            .map(|e| e.name.clone())
                            .collect(),
                    ),
                    knowledge_points: Some(point_summaries),
                    error: None,
                };
                extracted_chunks.push(extracted);
                result
            }
            Err(llm_err) => {
                let err_str = format!("{}", llm_err);
                println!("LLM ERROR: {}", err_str);

                ChunkResult {
                    chunk_index: idx,
                    heading: chunk.heading.clone(),
                    content_preview: truncate(&chunk.content, 200),
                    status: "llm_error".to_string(),
                    raw_llm_response: None,
                    entity_count: None,
                    point_count: None,
                    relation_count: None,
                    entities: None,
                    knowledge_points: None,
                    error: Some(err_str),
                }
            }
        };

        chunk_results.push(chunk_result);
    }

    // Aggregate stats
    let successful: Vec<&ChunkResult> = chunk_results
        .iter()
        .filter(|c| c.status == "success")
        .collect();
    let successful_count = successful.len();
    let failed_count = chunk_results.len() - successful_count;

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
        .filter(|c| c.status == "validation_error")
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
    let total_raw_mentions: usize = extracted_chunks.iter().map(|c| c.raw_entities.len()).sum();

    let graph = consolidate(extracted_chunks);
    let violations = validate_graph(&graph);

    let kp_with_ids = graph
        .knowledge_points
        .iter()
        .filter(|kp| !kp.entity_ids.is_empty())
        .count();

    let dedup_ratio = if total_raw_mentions > 0 {
        graph.entities.len() as f64 / total_raw_mentions as f64
    } else {
        1.0
    };

    let entity_summaries: Vec<EntitySummary> = graph
        .entities
        .iter()
        .map(|e| EntitySummary {
            id: e.id.clone(),
            canonical_name: e.canonical_name.clone(),
            aliases: e.aliases.clone(),
            chunk_count: e.chunk_ids.len(),
        })
        .collect();

    let consolidation = ConsolidationStats {
        total_raw_mentions,
        unique_entities: graph.entities.len(),
        dedup_ratio,
        kp_with_entity_ids: kp_with_ids,
        total_knowledge_points: graph.knowledge_points.len(),
        total_relations: graph.relations.len(),
        validation_violations: violations.clone(),
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
        total_chunks: chunks.len(),
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
        successful_count,
        chunks.len()
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
    if violations.is_empty() {
        println!("  validate_graph: OK");
    } else {
        println!("  validate_graph: {} VIOLATIONS", violations.len());
        for v in &violations {
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
        "Usage: cargo run --bin stage_a_eval -- [--note <markdown-file>] [--output-dir <dir>]"
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_dotenv_if_present() {
    let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
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
    chunks: &[ChunkResult],
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
    }

    workbook.save(path)?;
    Ok(())
}
