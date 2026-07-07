use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::models::question::{Question, QuestionInput};
use crate::models::recall_space::RecallSpace;
use crate::models::model_settings::ModelConfig;

fn resolve_database_url() -> String {
    // Honor an explicit DATABASE_URL first (dev overrides).
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return normalize_sqlite_url(&url);
    }

    // Otherwise, use the OS-specific application data directory to store the sqlite DB.
    use directories::ProjectDirs;

    let proj = ProjectDirs::from("com", "yudongan", "active-recall-knowledge-assistance")
        .expect("Unable to determine platform-specific data directory");

    let data_dir = proj.data_dir();
    if let Err(err) = std::fs::create_dir_all(data_dir) {
        eprintln!("Failed to create data dir {}: {}", data_dir.display(), err);
    }

    let db_path = data_dir.join("review.db");
    // Ensure parent exists
    if let Some(parent) = db_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            eprintln!("Failed to create db parent dir {}: {}", parent.display(), err);
        }
    }

    let db_str = db_path.to_string_lossy().replace('\\', "/");
    normalize_sqlite_url(&format!("sqlite://{}", db_str))
}

fn normalize_sqlite_url(raw: &str) -> String {
    let trimmed = raw.trim();

    if trimmed.starts_with("sqlite::") {
        return trimmed.to_string();
    }

    if let Some(path) = trimmed.strip_prefix("sqlite://") {
        let normalized_path = path.replace('\\', "/");

        // Windows absolute paths in sqlite URLs should be sqlite:///C:/...
        if normalized_path
            .as_bytes()
            .get(1)
            .is_some_and(|b| *b == b':')
        {
            return format!("sqlite:///{}", normalized_path);
        }

        return format!("sqlite://{}", normalized_path);
    }

    if let Some(path) = trimmed.strip_prefix("sqlite:") {
        let normalized_path = path.replace('\\', "/");

        if normalized_path
            .as_bytes()
            .get(1)
            .is_some_and(|b| *b == b':')
        {
            return format!("sqlite:///{}", normalized_path);
        }

        return format!("sqlite://{}", normalized_path);
    }

    let normalized_path = trimmed.replace('\\', "/");
    if normalized_path
        .as_bytes()
        .get(1)
        .is_some_and(|b| *b == b':')
    {
        return format!("sqlite:///{}", normalized_path);
    }

    format!("sqlite://{}", normalized_path)
}

async fn open_pool() -> Result<sqlx::SqlitePool, sqlx::Error> {
    let database_url = resolve_database_url();
    let options = SqliteConnectOptions::from_str(&database_url)?
        .create_if_missing(true)
        .foreign_keys(true);

    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
}

pub async fn get_questions() -> Result<Vec<Question>, sqlx::Error> {
    let pool = open_pool().await?;

    let questions = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id FROM questions ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    Ok(questions)
}

pub async fn get_questions_by_space(space_id: i64) -> Result<Vec<Question>, sqlx::Error> {
    let pool = open_pool().await?;

    let questions = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id FROM questions WHERE space_id = ? ORDER BY id",
    )
    .bind(space_id)
    .fetch_all(&pool)
    .await?;

    Ok(questions)
}

pub async fn get_spaces() -> Result<Vec<RecallSpace>, sqlx::Error> {
    let pool = open_pool().await?;

    let spaces = sqlx::query_as::<_, RecallSpace>(
        "SELECT id, name, description FROM recall_spaces ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    Ok(spaces)
}

pub async fn create_space(name: &str, description: Option<&str>) -> Result<RecallSpace, sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query("INSERT INTO recall_spaces (name, description) VALUES (?, ?)")
        .bind(name)
        .bind(description)
        .execute(&pool)
        .await?;

    let row = sqlx::query_as::<_, RecallSpace>("SELECT id, name, description FROM recall_spaces WHERE id = last_insert_rowid()")
        .fetch_one(&pool)
        .await?;

    Ok(row)
}

pub async fn modify_space(id: i64, name: &str, description: Option<&str>) -> Result<RecallSpace, sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query("UPDATE recall_spaces SET name = ?, description = ? WHERE id = ?")
        .bind(name)
        .bind(description)
        .bind(id)
        .execute(&pool)
        .await?;

    let row = sqlx::query_as::<_, RecallSpace>("SELECT id, name, description FROM recall_spaces WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await?;

    Ok(row)
}

pub async fn delete_space(id: i64) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    // Reassign any existing questions to default space (1) before deletion
    sqlx::query("UPDATE questions SET space_id = 1 WHERE space_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    sqlx::query("DELETE FROM recall_spaces WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(())
}

pub async fn save_questions(questions: Vec<QuestionInput>, model: String) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    for question in questions {
        sqlx::query(
            "INSERT INTO questions (question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&question.question)
        .bind(&question.option_a)
        .bind(&question.option_b)
        .bind(&question.option_c)
        .bind(&question.option_d)
        .bind(&question.correct_answer)
        .bind(&question.explanation)
        .bind(&model)
        .bind(if question.space_id > 0 { question.space_id } else { 1_i64 })
        .execute(&pool)
        .await?;
    }

    Ok(())
}

pub async fn load_model_config() -> Result<ModelConfig, sqlx::Error> {
    let pool = open_pool().await?;

    let config = sqlx::query_as::<_, ModelConfig>(
        "SELECT provider, base_url, selected_model, timeout_secs, api_key FROM model_settings WHERE id = 1",
    )
    .fetch_one(&pool)
    .await?;

    Ok(config)
}

pub async fn save_model_config(config: ModelConfig) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query(
                "INSERT INTO model_settings (id, provider, base_url, selected_model, timeout_secs, api_key, updated_at)
                 VALUES (1, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
            provider = excluded.provider,
            base_url = excluded.base_url,
            selected_model = excluded.selected_model,
            timeout_secs = excluded.timeout_secs,
            api_key = excluded.api_key,
            updated_at = excluded.updated_at",
    )
    .bind(&config.provider)
    .bind(&config.base_url)
    .bind(&config.selected_model)
    .bind(config.timeout_secs)
        .bind(&config.api_key)
    .execute(&pool)
    .await?;

    Ok(())
}

pub async fn run_smoke_test() -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(())
}
