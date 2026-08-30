// Pulls in your two code folders
mod models;
mod services;

#[cfg(debug_assertions)]
use std::path::PathBuf;

use models::model_settings::{EmbeddingConnectionResult, EmbeddingModelConfig, ModelConfig};
use models::note::Note;
use models::question::{Question, QuestionInput};
use models::recall_dashboard::RecallDashboard;
use models::recall_space::RecallSpace;
use services::generation::{GenerationProgressSnapshot, GenerationSummary};
use services::scheduler::Rating;
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

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
async fn get_due_questions(space_id: Option<i64>) -> Result<Vec<Question>, String> {
    services::database::get_due_questions(space_id)
        .await
        .map_err(|err| format!("Failed to load due questions: {err}"))
}

#[tauri::command]
async fn get_recall_dashboard() -> Result<RecallDashboard, String> {
    services::database::get_recall_dashboard()
        .await
        .map_err(|err| format!("Failed to load recall dashboard: {err}"))
}

#[tauri::command]
async fn review_question(
    question_id: i64,
    rating: String,
    is_correct: bool,
) -> Result<Question, String> {
    let parsed_rating = rating
        .parse::<Rating>()
        .map_err(|err| format!("Failed to parse review rating: {err}"))?;

    services::database::review_question_with_outcome(question_id, parsed_rating, is_correct)
        .await
        .map_err(|err| format!("Failed to review question {question_id}: {err}"))
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
    let embedding_config = config.embedding_config();
    if !embedding_config.selected_model.trim().is_empty() {
        services::embedding::prepare_embedding_service(&embedding_config)
            .map_err(|err| err.to_string())?;
    }

    services::database::save_model_config(config)
        .await
        .map_err(|err| format!("Failed to save model config: {err}"))
}

#[tauri::command]
async fn test_embedding_config(
    config: EmbeddingModelConfig,
) -> Result<EmbeddingConnectionResult, String> {
    let (provider, service) =
        services::embedding::prepare_embedding_service(&config).map_err(|err| err.to_string())?;
    let batch = service
        .embed_batch(&[String::from("ARKA embedding connection test")])
        .await
        .map_err(|err| format!("Embedding connection test failed: {err}"))?;

    Ok(EmbeddingConnectionResult {
        provider: provider.as_str().to_string(),
        model: config.selected_model.trim().to_string(),
        dimensions: batch.dimensions(),
    })
}

fn apply_persisted_llm_config(config: &ModelConfig) -> Result<(), String> {
    let timeout_secs = u64::try_from(config.timeout_secs)
        .map_err(|_| "Persisted LLM timeout must be greater than 0 seconds.".to_string())?;

    services::llm::set_runtime_llm_config(
        &config.provider,
        &config.base_url,
        &config.selected_model,
        timeout_secs,
        config.api_key.as_deref(),
    )
    .map_err(|err| format!("Failed to apply persisted LLM settings: {err}"))
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

#[tauri::command]
async fn start_graph_generation_job(vault_path: String) -> Result<String, String> {
    services::generation::start_graph_generation_job(&vault_path).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir {
                        file_name: Some("arka".into()),
                    }),
                ])
                .level(log::LevelFilter::Info)
                .max_file_size(1_000_000)
                .rotation_strategy(RotationStrategy::KeepSome(5))
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            log::info!(
                "Starting ARKA v{} on {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS
            );

            let log_path = app
                .path()
                .app_log_dir()
                .map(|path| path.join("arka.log"));

            match &log_path {
                Ok(path) => log::info!("Persistent log file: {}", path.display()),
                Err(err) => log::warn!("Could not resolve persistent log file: {err}"),
            }

            #[cfg(debug_assertions)]
            {
                let env_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env");
                match dotenvy::from_path(&env_path) {
                    Ok(_) => log::info!("Loaded environment from {}", env_path.display()),
                    Err(err) => log::warn!(
                        "Could not load .env from {}: {} (falling back to process environment)",
                        env_path.display(),
                        err
                    ),
                }
            }

            log::info!("Running database startup checks");
            if let Err(err) =
                tauri::async_runtime::block_on(services::database::run_smoke_test())
            {
                log::error!("Database startup failed: {err}");

                let log_location = log_path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| "the ARKA application log directory".to_string());
                let message = format!(
                    "ARKA could not start because its database could not be prepared.\n\n\
                     Error: {err}\n\n\
                     Diagnostic log:\n{log_location}\n\n\
                     If you need help, please report this bug and include the log file:\n\
                     https://github.com/DongAnYu/ARKA/issues"
                );

                app.dialog()
                    .message(message)
                    .title("ARKA could not start")
                    .kind(MessageDialogKind::Error)
                    .blocking_show();

                return Err(Box::new(err));
            }

            log::info!("Database startup checks completed successfully");

            match tauri::async_runtime::block_on(services::database::load_model_config()) {
                Ok(config) if config.selected_model.trim().is_empty() => {
                    log::info!(
                        "No persisted LLM model is configured; generation requires explicit model settings or environment configuration"
                    );
                }
                Ok(config) => match apply_persisted_llm_config(&config) {
                    Ok(()) => log::info!(
                        "Restored persisted LLM config (provider={}, base_url={}, model={}, timeout_secs={})",
                        config.provider,
                        config.base_url,
                        config.selected_model,
                        config.timeout_secs
                    ),
                    Err(err) => log::warn!(
                        "Could not restore persisted LLM config: {err}; generation requires valid model settings or environment configuration"
                    ),
                },
                Err(err) => log::warn!(
                    "Could not load persisted LLM config: {err}; generation requires valid model settings or environment configuration"
                ),
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_questions,
            get_questions_by_space,
            get_due_questions,
            get_recall_dashboard,
            review_question,
            delete_question,
            delete_questions,
            modify_question,
            get_notes,
            preview_generation,
            start_preview_generation,
            get_preview_generation_progress,
            set_preview_generation_paused,
            cancel_preview_generation,
            start_graph_generation_job,
            fetch_ollama_models,
            set_runtime_llm_settings,
            load_model_config,
            save_model_config,
            test_embedding_config,
            save_generated_questions,
            get_spaces,
            create_space,
            modify_space,
            delete_space
        ])
        .run(tauri::generate_context!())
        .unwrap_or_else(|err| {
            log::error!("Application terminated with a fatal startup/runtime error: {err}");
            eprintln!("Application terminated with a fatal startup/runtime error: {err}");
            std::process::exit(1);
        });
}
