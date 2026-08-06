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
use services::generation::{ChunkLlmQuestionPreview, ChunkPreview};

const DEFAULT_OUTPUT_DIR_NAME: &str = "eval/output";
const DEFAULT_NOTE_RELATIVE_PATH: &str = "docs/evaluation_notes/Photosynthesis.md";
const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_TIMEOUT_SECS: u64 = 60;

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
    total_questions: usize,
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
    question_index: Option<usize>,
    question: Option<String>,
    option_a: Option<String>,
    option_b: Option<String>,
    option_c: Option<String>,
    option_d: Option<String>,
    correct_answer: Option<String>,
    explanation: Option<String>,
    stage_error: Option<String>,
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

    let timestamp = Utc::now();
    let stem = args
        .note_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("evaluation")
        .to_ascii_lowercase();
    let file_prefix = format!("{}-{}", stem, timestamp.format("%Y%m%d-%H%M%S"));

    let summary = services::generation::orchestrate_vault(
        args.note_path
            .to_str()
            .ok_or("Note path contains invalid UTF-8")?,
    )
    .await
    .map_err(|err| format!("AQG pipeline failed: {err}"))?;

    let rows = flatten_rows(&summary.chunk_previews);
    let total_questions = rows.iter().filter(|row| row.question.is_some()).count();

    let json_path = args.output_dir.join(format!("{file_prefix}.json"));
    let xlsx_path = args.output_dir.join(format!("{file_prefix}.xlsx"));

    let run = EvaluationRun {
        generated_at_utc: timestamp.to_rfc3339(),
        provider: String::from("openrouter"),
        base_url,
        model,
        note_path: args.note_path.display().to_string(),
        json_path: json_path.display().to_string(),
        xlsx_path: xlsx_path.display().to_string(),
        total_chunks: summary.total_chunks,
        total_questions,
        chunk_previews: summary.chunk_previews,
        rows: rows.clone(),
    };

    fs::write(&json_path, serde_json::to_string_pretty(&run)?)?;
    write_xlsx(&xlsx_path, &rows)?;

    println!("AQG evaluation completed.");
    println!("Note: {}", args.note_path.display());
    println!("JSON: {}", json_path.display());
    println!("XLSX: {}", xlsx_path.display());
    println!("Questions recorded: {}", total_questions);

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

fn flatten_rows(chunks: &[ChunkPreview]) -> Vec<EvaluationRow> {
    let mut rows = Vec::new();

    for chunk in chunks {
        if chunk.llm_result.questions.is_empty() {
            rows.push(EvaluationRow {
                note_title: chunk.note_title.clone(),
                note_path: chunk.note_path.clone(),
                heading: chunk.heading.clone(),
                section_index: chunk.section_index,
                chunk_index: chunk.chunk_index,
                stage_status: chunk.llm_result.status.clone(),
                key_points: chunk.llm_result.key_points.clone(),
                question_index: None,
                question: None,
                option_a: None,
                option_b: None,
                option_c: None,
                option_d: None,
                correct_answer: None,
                explanation: None,
                stage_error: chunk.llm_result.error.clone(),
            });
            continue;
        }

        for (question_index, question) in chunk.llm_result.questions.iter().enumerate() {
            rows.push(build_question_row(chunk, question_index, question));
        }
    }

    rows
}

fn build_question_row(
    chunk: &ChunkPreview,
    question_index: usize,
    question: &ChunkLlmQuestionPreview,
) -> EvaluationRow {
    EvaluationRow {
        note_title: chunk.note_title.clone(),
        note_path: chunk.note_path.clone(),
        heading: chunk.heading.clone(),
        section_index: chunk.section_index,
        chunk_index: chunk.chunk_index,
        stage_status: chunk.llm_result.status.clone(),
        key_points: chunk.llm_result.key_points.clone(),
        question_index: Some(question_index + 1),
        question: Some(question.question.clone()),
        option_a: Some(question.option_a.clone()),
        option_b: Some(question.option_b.clone()),
        option_c: Some(question.option_c.clone()),
        option_d: Some(question.option_d.clone()),
        correct_answer: Some(question.correct_answer.clone()),
        explanation: Some(question.explanation.clone()),
        stage_error: chunk.llm_result.error.clone(),
    }
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
        "question_index",
        "question",
        "option_a",
        "option_b",
        "option_c",
        "option_d",
        "correct_answer",
        "explanation",
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

        if let Some(question_index) = row.question_index {
            worksheet.write_number(line, 7, question_index as f64)?;
        }

        worksheet.write_string(line, 8, row.question.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 9, row.option_a.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 10, row.option_b.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 11, row.option_c.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 12, row.option_d.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 13, row.correct_answer.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 14, row.explanation.as_deref().unwrap_or(""))?;
        worksheet.write_string(line, 15, row.stage_error.as_deref().unwrap_or(""))?;
    }

    workbook.save(path)?;
    Ok(())
}
