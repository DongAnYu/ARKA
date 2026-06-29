// Pulls in your two code folders
mod models;
mod services;

use models::note::Note;
use models::question::Question;
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
fn preview_generation(vault_path: String) -> Result<GenerationSummary, String> {
  services::generation::orchestrate_vault(&vault_path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .setup(|app| {
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
      preview_generation
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
