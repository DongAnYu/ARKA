// Pulls in your two code folders
mod models;
mod services;

use std::path::PathBuf;

use models::model_settings::ModelConfig;
use models::note::Note;
use models::question::{Question, QuestionInput};
use models::recall_space::RecallSpace;
use services::generation::{GenerationProgressSnapshot, GenerationSummary};

#[tauri::command]
async fn get_questions() -> Result<Vec<Question>, String> {
    services::database::get_questions()
        .await
        .map_err(|err| format!("Failed to load questions: {err}"))
}

#[tauri::command]
async fn get_questions_by_space(space_id: i64) -> Result<Vec<Question>, String> {
    services::database::get_questions_by_space(space_id)
        .await
        .map_err(|err| format!("Failed to load questions for space {space_id}: {err}"))
}

#[tauri::command]
async fn delete_question(id: i64) -> Result<(), String> {
    services::database::delete_question(id)
        .await
        .map_err(|err| format!("Failed to delete question {id}: {err}"))
}

#[tauri::command]
async fn delete_questions(ids: Vec<i64>) -> Result<(), String> {
    services::database::delete_questions(ids)
        .await
        .map_err(|err| format!("Failed to delete selected questions: {err}"))
}

#[tauri::command]
async fn modify_question(id: i64, question_input: QuestionInput) -> Result<Question, String> {
    services::database::modify_question(id, question_input)
        .await
        .map_err(|err| format!("Failed to modify question {id}: {err}"))
}

#[tauri::command]
fn get_notes(vault_path: String) -> Result<Vec<Note>, String> {
    services::filesystem::load_vault_notes(&vault_path)
}

#[tauri::command]
async fn preview_generation(vault_path: String) -> Result<GenerationSummary, String> {
    services::generation::orchestrate_vault(&vault_path).await
}

#[tauri::command]
async fn start_preview_generation(vault_path: String) -> Result<String, String> {
    services::generation::start_preview_generation_job(&vault_path).await
}

#[tauri::command]
fn get_preview_generation_progress(job_id: String) -> Result<GenerationProgressSnapshot, String> {
    services::generation::get_preview_generation_progress(&job_id)
}

#[tauri::command]
fn set_preview_generation_paused(job_id: String, paused: bool) -> Result<(), String> {
    services::generation::set_preview_generation_paused(&job_id, paused)
}

#[tauri::command]
fn cancel_preview_generation(job_id: String) -> Result<(), String> {
    services::generation::cancel_preview_generation(&job_id)
}

#[tauri::command]
async fn fetch_ollama_models(
    base_url: String,
    model_name: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<Vec<String>, String> {
    let timeout = timeout_secs.unwrap_or(60);
    if timeout == 0 {
        return Err(String::from("Timeout must be greater than 0 seconds."));
    }

    services::llm::fetch_ollama_models(&base_url, model_name.as_deref(), timeout)
        .await
        .map_err(|err| format!("Failed to fetch Ollama models: {err}"))
}

#[tauri::command]
async fn load_model_config() -> Result<ModelConfig, String> {
    services::database::load_model_config()
        .await
        .map_err(|err| format!("Failed to load model config: {err}"))
}

#[tauri::command]
async fn save_model_config(config: ModelConfig) -> Result<(), String> {
    services::database::save_model_config(config)
        .await
        .map_err(|err| format!("Failed to save model config: {err}"))
}

#[tauri::command]
fn set_runtime_llm_settings(
    provider: String,
    base_url: String,
    model: String,
    timeout_secs: Option<u64>,
    api_key: Option<String>,
) -> Result<(), String> {
    let timeout = timeout_secs.unwrap_or(60);
    if timeout == 0 {
        return Err(String::from("Timeout must be greater than 0 seconds."));
    }

    services::llm::set_runtime_llm_config(&provider, &base_url, &model, timeout, api_key.as_deref())
        .map_err(|err| format!("Failed to apply runtime LLM settings: {err}"))
}

#[tauri::command]
async fn save_generated_questions(
    questions: Vec<QuestionInput>,
    model: String,
) -> Result<(), String> {
    services::database::save_questions(questions, model)
        .await
        .map_err(|err| format!("Failed to save questions: {err}"))
}

#[tauri::command]
async fn get_spaces() -> Result<Vec<RecallSpace>, String> {
    services::database::get_spaces()
        .await
        .map_err(|err| format!("Failed to load recall spaces: {err}"))
}

#[tauri::command]
async fn create_space(name: String, description: Option<String>) -> Result<RecallSpace, String> {
    services::database::create_space(&name, description.as_deref())
        .await
        .map_err(|err| format!("Failed to create recall space: {err}"))
}

#[tauri::command]
async fn modify_space(
    id: i64,
    name: String,
    description: Option<String>,
) -> Result<RecallSpace, String> {
    services::database::modify_space(id, &name, description.as_deref())
        .await
        .map_err(|err| format!("Failed to modify recall space: {err}"))
}

#[tauri::command]
async fn delete_space(id: i64) -> Result<(), String> {
    services::database::delete_space(id)
        .await
        .map_err(|err| format!("Failed to delete recall space: {err}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
            match dotenvy::from_path(&env_path) {
                Ok(_) => log::info!("Loaded environment from {}", env_path.display()),
                Err(err) => log::warn!(
          "Could not load .env from {}: {} (falling back to process environment/defaults)",
          env_path.display(),
          err
        ),
            }

            tauri::async_runtime::block_on(async { services::database::run_smoke_test().await })
                .map_err(|err| -> Box<dyn std::error::Error> { Box::new(err) })?;

            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_questions,
            get_questions_by_space,
            delete_question,
            delete_questions,
            modify_question,
            get_notes,
            preview_generation,
            start_preview_generation,
            get_preview_generation_progress,
            set_preview_generation_paused,
            cancel_preview_generation,
            fetch_ollama_models,
            set_runtime_llm_settings,
            load_model_config,
            save_model_config,
            save_generated_questions,
            get_spaces,
            create_space,
            modify_space,
            delete_space
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
