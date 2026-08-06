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

use services::graph_generation::pipeline::{
    run_graph_stage_a, run_graph_stage_b, GraphStageAResult, GraphStageBResult,
};
use services::llm::LlmService;

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone)]
struct CliArgs {
    note_path: PathBuf,
    output_dir: PathBuf,
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
    stage_a_summary: StageASummary,
    stage_b_summary: StageBSummary,
    stage_a: GraphStageAResult,
    stage_b: GraphStageBResult,
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

    println!("Graph E2E Eval");
    println!("Note: {}", args.note_path.display());
    println!("Model: {model}");
    println!("---");

    println!("Running Stage A...");
    let stage_a = run_graph_stage_a(note_path, &llm).await?;
    let stage_a_summary = summarize_stage_a(&stage_a);
    print_stage_a_summary(&stage_a_summary);

    println!("---");
    println!("Running Stage B...");
    let stage_b = run_graph_stage_b(&stage_a, &llm).await?;
    let stage_b_summary = summarize_stage_b(&stage_b);
    print_stage_b_summary(&stage_b_summary);

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
        provider: String::from("openrouter"),
        base_url,
        model,
        note_path: args.note_path.display().to_string(),
        json_path: json_path.display().to_string(),
        xlsx_path: xlsx_path.display().to_string(),
        stage_a_summary,
        stage_b_summary,
        stage_a,
        stage_b,
    };

    fs::write(&json_path, serde_json::to_string_pretty(&run)?)?;
    write_xlsx(&xlsx_path, &run)?;

    println!("---");
    println!("Graph E2E evaluation completed.");
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

fn write_xlsx(path: &Path, run: &GraphE2eEvalRun) -> Result<(), Box<dyn Error>> {
    let mut workbook = Workbook::new();
    write_summary_sheet(&mut workbook, run)?;
    write_stage_a_chunks_sheet(&mut workbook, &run.stage_a)?;
    write_entities_sheet(&mut workbook, &run.stage_a)?;
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
        ("provider", run.provider.as_str()),
        ("model", run.model.as_str()),
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

    let start_b = start + 8;
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
    let sheet = workbook.add_worksheet().set_name("Entities")?;
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
        "Usage: cargo run --manifest-path eval/Cargo.toml --bin graph_e2e_eval -- [--note <markdown-file>] [--output-dir <dir>]"
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
