// Pulls in your two code folders
mod models;
mod services;

use std::path::PathBuf;

use models::note::Note;
use models::question::Question;
use models::model_settings::ModelConfig;
use services::generation::GenerationSummary;

#[tauri::command]
async fn get_questions() -> Result<Vec<Question>, String> {
  services::database::get_questions()
    .await
    .map_err(|err| format!("Failed to load questions: {err}"))
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
) -> Result<(), String> {
  if provider.trim().to_lowercase() != "ollama" {
    return Err(String::from("Only Ollama runtime settings are currently supported."));
  }

  let timeout = timeout_secs.unwrap_or(60);
  if timeout == 0 {
    return Err(String::from("Timeout must be greater than 0 seconds."));
  }

  services::llm::set_runtime_llm_config(&base_url, &model, timeout)
    .map_err(|err| format!("Failed to apply runtime LLM settings: {err}"))
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

      tauri::async_runtime::block_on(async {
        services::database::run_smoke_test().await
      })
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
      get_notes,
      preview_generation,
      fetch_ollama_models,
      set_runtime_llm_settings,
      load_model_config,
      save_model_config
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
