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
use services::generation::ChunkPreview;

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";

#[derive(Debug, Clone)]
struct CliArgs {
    note_path: PathBuf,
    output_dir: PathBuf,
}

#[derive(Debug, Serialize)]
struct EvaluationRun {
    generated_at_utc: String,
    provider: String,
    base_url: String,
    model: String,
    note_path: String,
    json_path: String,
    xlsx_path: String,
    total_chunks: usize,
    total_learning_items: usize,
    flashcard_only_items: usize,
    items_with_mcq: usize,
    failed_chunks: usize,
    chunk_previews: Vec<ChunkPreview>,
    rows: Vec<EvaluationRow>,
}

#[derive(Debug, Clone, Serialize)]
struct EvaluationRow {
    note_title: String,
    note_path: String,
    heading: String,
    section_index: usize,
    chunk_index: usize,
    stage_status: String,
    key_points: Vec<String>,
    item_index: Option<usize>,
    knowledge_point_id: Option<String>,
    source_knowledge_point: Option<String>,
    target: Option<String>,
    answer: Option<String>,
    explanation: Option<String>,
    flashcard_prompt: Option<String>,
    has_mcq: bool,
    mcq_prompt: Option<String>,
    mcq_prompt_reuses_flashcard: bool,
    distractor_1: Option<String>,
    distractor_2: Option<String>,
    distractor_3: Option<String>,
    mcq_omission_reason: Option<String>,
    generation_provider: Option<String>,
    generation_model: Option<String>,
    stage_error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    load_dotenv()?;

    let args = parse_args()?;
    fs::create_dir_all(&args.output_dir)?;

    let llm_config = services::llm::LlmConfig::from_env()?;
    let provider = match llm_config.provider {
        services::llm::LlmProvider::Ollama => "ollama",
        services::llm::LlmProvider::OpenAi => "openai",
        services::llm::LlmProvider::OpenRouter => "openrouter",
    };
    let base_url = llm_config.base_url.clone();
    let model = llm_config.model.clone();

    let timestamp = Utc::now();
    let stem = args
        .note_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("evaluation")
        .to_ascii_lowercase();
    let file_prefix = format!("{}-{}", stem, timestamp.format("%Y%m%d-%H%M%S"));

    let note_path = args
        .note_path
        .to_str()
        .ok_or("Note path contains invalid UTF-8")?;
    let notes = services::filesystem::load_vault_notes(note_path)
        .map_err(|err| format!("AQG pipeline failed to load note: {err}"))?;
    let summary = services::generation::orchestrate_notes_for_evaluation(&notes).await;

    let rows = flatten_rows(&summary.chunk_previews);
    let total_learning_items = rows
        .iter()
        .filter(|row| row.knowledge_point_id.is_some())
        .count();
    let items_with_mcq = rows.iter().filter(|row| row.has_mcq).count();
    let flashcard_only_items = total_learning_items.saturating_sub(items_with_mcq);
    let failed_chunks = summary
        .chunk_previews
        .iter()
        .filter(|chunk| chunk.llm_result.status == "error")
        .count();

    let json_path = args.output_dir.join(format!("{file_prefix}.json"));
    let xlsx_path = args.output_dir.join(format!("{file_prefix}.xlsx"));

    let run = EvaluationRun {
        generated_at_utc: timestamp.to_rfc3339(),
        provider: provider.to_string(),
        base_url,
        model,
        note_path: args.note_path.display().to_string(),
        json_path: json_path.display().to_string(),
        xlsx_path: xlsx_path.display().to_string(),
        total_chunks: summary.total_chunks,
        total_learning_items,
        flashcard_only_items,
        items_with_mcq,
        failed_chunks,
        chunk_previews: summary.chunk_previews,
        rows: rows.clone(),
    };

    fs::write(&json_path, serde_json::to_string_pretty(&run)?)?;
    write_xlsx(&xlsx_path, &rows)?;

    println!("AQG evaluation completed.");
    println!("Note: {}", args.note_path.display());
    println!("JSON: {}", json_path.display());
    println!("XLSX: {}", xlsx_path.display());
    println!("Learning items recorded: {}", total_learning_items);
    println!("Items with MCQ: {}", items_with_mcq);
    println!("Flashcard-only items: {}", flashcard_only_items);
    println!("Failed chunks: {}", failed_chunks);

    Ok(())
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
    println!("Usage: cargo run --manifest-path eval/Cargo.toml --bin aqg_eval -- [--note <markdown-file>] [--output-dir <dir>]");
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_dotenv() -> Result<(), Box<dyn Error>> {
    let env_path = repo_root().join("src-tauri/.env");
    dotenvy::from_path_override(&env_path)
        .map_err(|error| format!("Failed to load {}: {error}", env_path.display()))?;
    Ok(())
}

fn flatten_rows(chunks: &[ChunkPreview]) -> Vec<EvaluationRow> {
    let mut rows = Vec::new();

    for chunk in chunks {
        if chunk.llm_result.items.is_empty() {
            rows.push(EvaluationRow {
                note_title: chunk.note_title.clone(),
                note_path: chunk.note_path.clone(),
                heading: chunk.heading.clone(),
                section_index: chunk.section_index,
                chunk_index: chunk.chunk_index,
                stage_status: chunk.llm_result.status.clone(),
                key_points: chunk.llm_result.key_points.clone(),
                item_index: None,
                knowledge_point_id: None,
                source_knowledge_point: None,
                target: None,
                answer: None,
                explanation: None,
                flashcard_prompt: None,
                has_mcq: false,
                mcq_prompt: None,
                mcq_prompt_reuses_flashcard: false,
                distractor_1: None,
                distractor_2: None,
                distractor_3: None,
                mcq_omission_reason: None,
                generation_provider: None,
                generation_model: None,
                stage_error: chunk.llm_result.error.clone(),
            });
            continue;
        }

        for (item_index, item) in chunk.llm_result.items.iter().enumerate() {
            let mcq_prompt_reuses_flashcard = item
                .content
                .mcq
                .as_ref()
                .is_some_and(|mcq| mcq.prompt.is_none());
            let mcq_prompt = item.content.mcq.as_ref().map(|mcq| {
                mcq.prompt
                    .clone()
                    .unwrap_or_else(|| item.content.flashcard.prompt.clone())
            });
            let distractors = item
                .content
                .mcq
                .as_ref()
                .map(|mcq| mcq.distractors.as_slice())
                .unwrap_or(&[]);

            rows.push(EvaluationRow {
                note_title: chunk.note_title.clone(),
                note_path: chunk.note_path.clone(),
                heading: chunk.heading.clone(),
                section_index: chunk.section_index,
                chunk_index: chunk.chunk_index,
                stage_status: chunk.llm_result.status.clone(),
                key_points: chunk.llm_result.key_points.clone(),
                item_index: Some(item_index + 1),
                knowledge_point_id: Some(item.content.knowledge_point_id.clone()),
                source_knowledge_point: Some(item.source.knowledge_point.clone()),
                target: Some(item.content.target.clone()),
                answer: Some(item.content.answer.clone()),
                explanation: item.content.explanation.clone(),
                flashcard_prompt: Some(item.content.flashcard.prompt.clone()),
                has_mcq: item.content.mcq.is_some(),
                mcq_prompt,
                mcq_prompt_reuses_flashcard,
                distractor_1: distractors.first().cloned(),
                distractor_2: distractors.get(1).cloned(),
                distractor_3: distractors.get(2).cloned(),
                mcq_omission_reason: item.content.mcq_omission_reason.clone(),
                generation_provider: item.generation.provider.clone(),
                generation_model: item.generation.model.clone(),
                stage_error: chunk.llm_result.error.clone(),
            });
        }
    }

    rows
}

fn write_xlsx(path: &Path, rows: &[EvaluationRow]) -> Result<(), Box<dyn Error>> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();

    let headers = [
        "note_title",
        "note_path",
        "heading",
        "section_index",
        "chunk_index",
        "stage_status",
        "key_points",
        "item_index",
        "knowledge_point_id",
        "source_knowledge_point",
        "target",
        "answer",
        "explanation",
        "flashcard_prompt",
        "has_mcq",
        "mcq_prompt",
        "mcq_prompt_reuses_flashcard",
        "distractor_1",
        "distractor_2",
        "distractor_3",
        "mcq_omission_reason",
        "generation_provider",
        "generation_model",
        "stage_error",
    ];

    for (column, header) in headers.iter().enumerate() {
        worksheet.write_string(0, column as u16, *header)?;
    }

    for (index, row) in rows.iter().enumerate() {
        let line = (index + 1) as u32;
        worksheet.write_string(line, 0, &row.note_title)?;
        worksheet.write_string(line, 1, &row.note_path)?;
        worksheet.write_string(line, 2, &row.heading)?;
        worksheet.write_number(line, 3, row.section_index as f64)?;
        worksheet.write_number(line, 4, row.chunk_index as f64)?;
        worksheet.write_string(line, 5, &row.stage_status)?;
        worksheet.write_string(line, 6, row.key_points.join(" | "))?;

        if let Some(item_index) = row.item_index {
            worksheet.write_number(line, 7, item_index as f64)?;
        }

        worksheet.write_string(line, 8, row.knowledge_point_id.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 9, row.source_knowledge_point.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 10, row.target.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 11, row.answer.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 12, row.explanation.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 13, row.flashcard_prompt.as_deref().unwrap_or(""))?;
        worksheet.write_boolean(line, 14, row.has_mcq)?;
        worksheet.write_string(line, 15, row.mcq_prompt.as_deref().unwrap_or(""))?;
        worksheet.write_boolean(line, 16, row.mcq_prompt_reuses_flashcard)?;
        worksheet.write_string(line, 17, row.distractor_1.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 18, row.distractor_2.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 19, row.distractor_3.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 20, row.mcq_omission_reason.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 21, row.generation_provider.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 22, row.generation_model.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 23, row.stage_error.as_deref().unwrap_or(""))?;
    }

    workbook.save(path)?;
    Ok(())
}
