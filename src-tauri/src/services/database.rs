use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;

use crate::models::model_settings::ModelConfig;
use crate::models::question::{Question, QuestionInput};
use crate::models::recall_dashboard::{RecallDashboard, RecallSpaceSummary};
use crate::models::recall_space::RecallSpace;
use crate::services::scheduler::{Rating, SM2Scheduler};

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
            eprintln!(
                "Failed to create db parent dir {}: {}",
                parent.display(),
                err
            );
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

async fn ensure_default_space(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO recall_spaces (id, name, description) VALUES (1, 'General', 'Default space for ungrouped questions')",
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_questions() -> Result<Vec<Question>, sqlx::Error> {
    let pool = open_pool().await?;

    let questions = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at FROM questions ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    Ok(questions)
}

pub async fn get_questions_by_space(space_id: i64) -> Result<Vec<Question>, sqlx::Error> {
    let pool = open_pool().await?;

    let questions = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at FROM questions WHERE space_id = ? ORDER BY id",
    )
    .bind(space_id)
    .fetch_all(&pool)
    .await?;

    Ok(questions)
}

pub async fn get_due_questions(space_id: Option<i64>) -> Result<Vec<Question>, sqlx::Error> {
    let pool = open_pool().await?;

    let questions = if let Some(space_id) = space_id.filter(|id| *id > 0) {
        sqlx::query_as::<_, Question>(
            "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at
             FROM questions
             WHERE space_id = ?
               AND (next_review_at IS NULL OR next_review_at <= CURRENT_TIMESTAMP)
             ORDER BY COALESCE(next_review_at, '1970-01-01 00:00:00') ASC, id ASC",
        )
        .bind(space_id)
        .fetch_all(&pool)
        .await?
    } else {
        sqlx::query_as::<_, Question>(
            "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at
             FROM questions
             WHERE next_review_at IS NULL OR next_review_at <= CURRENT_TIMESTAMP
             ORDER BY COALESCE(next_review_at, '1970-01-01 00:00:00') ASC, id ASC",
        )
        .fetch_all(&pool)
        .await?
    };

    Ok(questions)
}

pub async fn get_recall_dashboard() -> Result<RecallDashboard, sqlx::Error> {
    let pool = open_pool().await?;
    ensure_default_space(&pool).await?;

    let totals = sqlx::query_as::<_, (i64, i64, i64, i64)>(
        "SELECT
            COALESCE(SUM(CASE
                WHEN next_review_at IS NULL
                    OR (next_review_at >= date('now')
                        AND next_review_at <= CURRENT_TIMESTAMP)
                THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN next_review_at IS NOT NULL AND next_review_at < date('now')
                THEN 1 ELSE 0 END), 0),
            (SELECT COUNT(*) FROM review_history WHERE reviewed_at >= date('now')),
            (SELECT COALESCE(SUM(is_correct), 0) FROM review_history WHERE reviewed_at >= date('now'))
         FROM questions",
    )
    .fetch_one(&pool)
    .await?;

    let space_rows = sqlx::query_as::<_, (i64, String, i64, i64, i64, i64, i64)>(
        "SELECT
            recall_spaces.id,
            recall_spaces.name,
            COUNT(questions.id),
            COALESCE(SUM(CASE
                WHEN questions.id IS NOT NULL
                    AND (questions.next_review_at IS NULL
                        OR questions.next_review_at <= CURRENT_TIMESTAMP)
                THEN 1 ELSE 0 END), 0),
            COALESCE(SUM(CASE
                WHEN questions.next_review_at IS NOT NULL
                    AND questions.next_review_at < date('now')
                THEN 1 ELSE 0 END), 0),
            (SELECT COUNT(*)
               FROM review_history
               INNER JOIN questions AS reviewed_questions ON reviewed_questions.id = review_history.question_id
               WHERE reviewed_questions.space_id = recall_spaces.id
                 AND review_history.reviewed_at >= date('now'))
            , (SELECT COALESCE(SUM(review_history.is_correct), 0)
               FROM review_history
               INNER JOIN questions AS reviewed_questions ON reviewed_questions.id = review_history.question_id
               WHERE reviewed_questions.space_id = recall_spaces.id
                 AND review_history.reviewed_at >= date('now'))
         FROM recall_spaces
         LEFT JOIN questions ON questions.space_id = recall_spaces.id
         GROUP BY recall_spaces.id, recall_spaces.name
         ORDER BY recall_spaces.id",
    )
    .fetch_all(&pool)
    .await?;

    let spaces = space_rows
        .into_iter()
        .map(
            |(
                id,
                name,
                total_questions,
                due_count,
                overdue_count,
                reviewed_today_count,
                correct_today_count,
            )| RecallSpaceSummary {
                id,
                name,
                total_questions,
                due_count,
                overdue_count,
                reviewed_today_count,
                correct_today_count,
            },
        )
        .collect();

    Ok(RecallDashboard {
        due_today_count: totals.0,
        overdue_count: totals.1,
        reviewed_today_count: totals.2,
        correct_today_count: totals.3,
        spaces,
    })
}

pub async fn review_question_with_outcome(
    question_id: i64,
    rating: Rating,
    is_correct: bool,
) -> Result<Question, sqlx::Error> {
    let pool = open_pool().await?;

    let mut question = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at FROM questions WHERE id = ?",
    )
    .bind(question_id)
    .fetch_one(&pool)
    .await?;

    SM2Scheduler::review_question(&mut question, rating);

    sqlx::query(
        "UPDATE questions
         SET repetitions = ?, interval_days = ?, ease_factor = ?, next_review_at = ?, last_reviewed_at = ?
         WHERE id = ?",
    )
    .bind(question.repetitions)
    .bind(question.interval_days)
    .bind(question.ease_factor)
    .bind(question.next_review_at)
    .bind(question.last_reviewed_at)
    .bind(question.id)
    .execute(&pool)
    .await?;

    sqlx::query("INSERT INTO review_history (question_id, is_correct) VALUES (?, ?)")
        .bind(question.id)
        .bind(is_correct)
        .execute(&pool)
        .await?;

    Ok(question)
}

pub async fn modify_question(
    id: i64,
    question_input: QuestionInput,
) -> Result<Question, sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query(
        "UPDATE questions SET question = ?, option_a = ?, option_b = ?, option_c = ?, option_d = ?, correct_answer = ?, explanation = ?, space_id = ? WHERE id = ?",
    )
    .bind(&question_input.question)
    .bind(&question_input.option_a)
    .bind(&question_input.option_b)
    .bind(&question_input.option_c)
    .bind(&question_input.option_d)
    .bind(&question_input.correct_answer)
    .bind(&question_input.explanation)
    .bind(if question_input.space_id > 0 { question_input.space_id } else { 1_i64 })
    .bind(id)
    .execute(&pool)
    .await?;

    let updated_question = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at FROM questions WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;

    Ok(updated_question)
}

pub async fn delete_question(id: i64) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query("DELETE FROM questions WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(())
}

pub async fn delete_questions(ids: Vec<i64>) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    for id in ids {
        sqlx::query("DELETE FROM questions WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await?;
    }

    Ok(())
}

pub async fn get_spaces() -> Result<Vec<RecallSpace>, sqlx::Error> {
    let pool = open_pool().await?;
    ensure_default_space(&pool).await?;

    let spaces = sqlx::query_as::<_, RecallSpace>(
        "SELECT id, name, description FROM recall_spaces ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    Ok(spaces)
}

pub async fn create_space(
    name: &str,
    description: Option<&str>,
) -> Result<RecallSpace, sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query("INSERT INTO recall_spaces (name, description) VALUES (?, ?)")
        .bind(name)
        .bind(description)
        .execute(&pool)
        .await?;

    let row = sqlx::query_as::<_, RecallSpace>(
        "SELECT id, name, description FROM recall_spaces WHERE id = last_insert_rowid()",
    )
    .fetch_one(&pool)
    .await?;

    Ok(row)
}

pub async fn modify_space(
    id: i64,
    name: &str,
    description: Option<&str>,
) -> Result<RecallSpace, sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query("UPDATE recall_spaces SET name = ?, description = ? WHERE id = ?")
        .bind(name)
        .bind(description)
        .bind(id)
        .execute(&pool)
        .await?;

    let row = sqlx::query_as::<_, RecallSpace>(
        "SELECT id, name, description FROM recall_spaces WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&pool)
    .await?;

    Ok(row)
}

pub async fn delete_space(id: i64) -> Result<(), sqlx::Error> {
    if id == 1 {
        return Err(sqlx::Error::Protocol(
            "Default space 'General' cannot be deleted.".into(),
        ));
    }

    let pool = open_pool().await?;

    // Delete all questions inside this space before deleting the space.
    sqlx::query("DELETE FROM questions WHERE space_id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    sqlx::query("DELETE FROM recall_spaces WHERE id = ?")
        .bind(id)
        .execute(&pool)
        .await?;

    Ok(())
}

pub async fn save_questions(
    questions: Vec<QuestionInput>,
    model: String,
) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    for question in questions {
        sqlx::query(
            "INSERT INTO questions (question, option_a, option_b, option_c, option_d, correct_answer, explanation, model, space_id, repetitions, interval_days, ease_factor, next_review_at, last_reviewed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, 2.5, CURRENT_TIMESTAMP, NULL)",
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
        "SELECT provider, base_url, selected_model, timeout_secs, api_key,
                embedding_provider, embedding_base_url, embedding_selected_model,
                embedding_timeout_secs, embedding_api_key
         FROM model_settings
         WHERE id = 1",
    )
    .fetch_one(&pool)
    .await?;

    Ok(config)
}

pub async fn save_model_config(config: ModelConfig) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;

    sqlx::query(
        "INSERT INTO model_settings (
                    id, provider, base_url, selected_model, timeout_secs, api_key,
                    embedding_provider, embedding_base_url, embedding_selected_model,
                    embedding_timeout_secs, embedding_api_key, updated_at
                 )
                 VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
            provider = excluded.provider,
            base_url = excluded.base_url,
            selected_model = excluded.selected_model,
            timeout_secs = excluded.timeout_secs,
            api_key = excluded.api_key,
            embedding_provider = excluded.embedding_provider,
            embedding_base_url = excluded.embedding_base_url,
            embedding_selected_model = excluded.embedding_selected_model,
            embedding_timeout_secs = excluded.embedding_timeout_secs,
            embedding_api_key = excluded.embedding_api_key,
            updated_at = excluded.updated_at",
    )
    .bind(&config.provider)
    .bind(&config.base_url)
    .bind(&config.selected_model)
    .bind(config.timeout_secs)
    .bind(&config.api_key)
    .bind(&config.embedding_provider)
    .bind(&config.embedding_base_url)
    .bind(&config.embedding_selected_model)
    .bind(config.embedding_timeout_secs)
    .bind(&config.embedding_api_key)
    .execute(&pool)
    .await?;

    Ok(())
}

pub async fn run_smoke_test() -> Result<(), sqlx::Error> {
    let pool = open_pool().await.map_err(|err| {
        log::error!("Failed to open the application database: {err}");
        err
    })?;

    log::info!("Applying database migrations");

    #[cfg(not(feature = "eval-package"))]
    if let Err(err) = sqlx::migrate!("./migrations").run(&pool).await {
        log::error!("Database migration failed: {err}");
        return Err(err.into());
    }

    #[cfg(feature = "eval-package")]
    if let Err(err) = sqlx::migrate!("../src-tauri/migrations").run(&pool).await {
        log::error!("Database migration failed: {err}");
        return Err(err.into());
    }

    log::info!("Database migrations completed successfully");

    ensure_default_space(&pool).await.map_err(|err| {
        log::error!("Failed to ensure the default recall space exists: {err}");
        err
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn database_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn model_settings_migration_adds_unconfigured_embedding_defaults() {
        let _guard = database_test_lock()
            .lock()
            .expect("database test lock should not be poisoned");

        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let db_path =
            std::env::temp_dir().join(format!("arka-embedding-defaults-{unique_id}.sqlite"));

        std::env::set_var("DATABASE_URL", format!("sqlite://{}", db_path.display()));

        run_smoke_test()
            .await
            .expect("migrations should add embedding settings");
        let config = load_model_config()
            .await
            .expect("default model settings should load");

        assert_eq!(config.embedding_provider, "ollama");
        assert_eq!(config.embedding_base_url, "http://localhost:11434");
        assert!(config.embedding_selected_model.is_empty());
        assert_eq!(config.embedding_timeout_secs, 60);
        assert_eq!(config.embedding_api_key, None);

        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn model_settings_preserve_generation_and_embedding_providers() {
        let _guard = database_test_lock()
            .lock()
            .expect("database test lock should not be poisoned");

        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("arka-openai-config-{unique_id}.sqlite"));

        std::env::set_var("DATABASE_URL", format!("sqlite://{}", db_path.display()));

        run_smoke_test()
            .await
            .expect("migrations should support the OpenAI provider");
        save_model_config(ModelConfig {
            provider: String::from("openai"),
            base_url: String::from("https://api.openai.com/v1"),
            selected_model: String::from("test-model"),
            timeout_secs: 60,
            api_key: Some(String::from("test-key")),
            embedding_provider: String::from("openrouter"),
            embedding_base_url: String::from("https://openrouter.ai/api/v1"),
            embedding_selected_model: String::from("test-embedding-model"),
            embedding_timeout_secs: 45,
            embedding_api_key: Some(String::from("test-embedding-key")),
        })
        .await
        .expect("OpenAI model settings should save");

        let config = load_model_config()
            .await
            .expect("OpenAI model settings should reload");
        assert_eq!(config.provider, "openai");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert_eq!(config.selected_model, "test-model");
        assert_eq!(config.api_key.as_deref(), Some("test-key"));
        assert_eq!(config.embedding_provider, "openrouter");
        assert_eq!(config.embedding_base_url, "https://openrouter.ai/api/v1");
        assert_eq!(config.embedding_selected_model, "test-embedding-model");
        assert_eq!(config.embedding_timeout_secs, 45);
        assert_eq!(
            config.embedding_api_key.as_deref(),
            Some("test-embedding-key")
        );

        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_questions_loads_saved_questions_with_scheduler_fields() {
        let _guard = database_test_lock()
            .lock()
            .expect("database test lock should not be poisoned");

        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("arka-scheduler-{unique_id}.sqlite"));

        std::env::set_var("DATABASE_URL", format!("sqlite://{}", db_path.display()));

        run_smoke_test()
            .await
            .expect("migrations should run against temp database");

        save_questions(
            vec![QuestionInput {
                question: "What scheduler are we adding?".to_string(),
                option_a: "SM-2".to_string(),
                option_b: "FIFO".to_string(),
                option_c: "LIFO".to_string(),
                option_d: "Random".to_string(),
                correct_answer: "A".to_string(),
                explanation: Some("SM-2 tracks interval, repetitions, and ease.".to_string()),
                space_id: 1,
            }],
            "test-model".to_string(),
        )
        .await
        .expect("question should save with scheduler defaults");

        let questions = get_questions()
            .await
            .expect("questions should map from sqlite rows");

        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].repetitions, 0);
        assert_eq!(questions[0].interval_days, 0);
        assert_eq!(questions[0].ease_factor, 2.5);
        assert!(questions[0].next_review_at.is_some());
        assert!(questions[0].last_reviewed_at.is_none());

        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_due_questions_filters_by_due_time_and_optional_space() {
        let _guard = database_test_lock()
            .lock()
            .expect("database test lock should not be poisoned");

        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("arka-due-questions-{unique_id}.sqlite"));

        std::env::set_var("DATABASE_URL", format!("sqlite://{}", db_path.display()));

        run_smoke_test()
            .await
            .expect("migrations should run against temp database");

        let space = create_space("Biology", Some("Due review filter test"))
            .await
            .expect("space should be created");

        save_questions(
            vec![
                QuestionInput {
                    question: "Due in general".to_string(),
                    option_a: "A".to_string(),
                    option_b: "B".to_string(),
                    option_c: "C".to_string(),
                    option_d: "D".to_string(),
                    correct_answer: "A".to_string(),
                    explanation: None,
                    space_id: 1,
                },
                QuestionInput {
                    question: "Due in biology".to_string(),
                    option_a: "A".to_string(),
                    option_b: "B".to_string(),
                    option_c: "C".to_string(),
                    option_d: "D".to_string(),
                    correct_answer: "A".to_string(),
                    explanation: None,
                    space_id: space.id,
                },
            ],
            "test-model".to_string(),
        )
        .await
        .expect("questions should save with scheduler defaults");

        let pool = open_pool().await.expect("pool should open");
        let future_review_at = Utc::now().naive_utc() + Duration::days(3);
        sqlx::query("UPDATE questions SET next_review_at = ? WHERE question = ?")
            .bind(future_review_at)
            .bind("Due in biology")
            .execute(&pool)
            .await
            .expect("question should be moved into the future");

        let all_due = get_due_questions(None)
            .await
            .expect("due questions should load without filter");
        assert_eq!(all_due.len(), 1);
        assert_eq!(all_due[0].question, "Due in general");

        let general_due = get_due_questions(Some(1))
            .await
            .expect("general due questions should load");
        assert_eq!(general_due.len(), 1);
        assert_eq!(general_due[0].space_id, 1);

        let biology_due = get_due_questions(Some(space.id))
            .await
            .expect("biology due questions should load");
        assert!(biology_due.is_empty());

        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(db_path);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn review_question_persists_scheduler_updates() {
        let _guard = database_test_lock()
            .lock()
            .expect("database test lock should not be poisoned");

        let unique_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("arka-review-question-{unique_id}.sqlite"));

        std::env::set_var("DATABASE_URL", format!("sqlite://{}", db_path.display()));

        run_smoke_test()
            .await
            .expect("migrations should run against temp database");

        save_questions(
            vec![QuestionInput {
                question: "Persist scheduler update".to_string(),
                option_a: "A".to_string(),
                option_b: "B".to_string(),
                option_c: "C".to_string(),
                option_d: "D".to_string(),
                correct_answer: "A".to_string(),
                explanation: None,
                space_id: 1,
            }],
            "test-model".to_string(),
        )
        .await
        .expect("question should save with scheduler defaults");

        let saved_question = get_questions()
            .await
            .expect("saved questions should load")
            .into_iter()
            .next()
            .expect("expected one saved question");

        let reviewed_question = review_question_with_outcome(saved_question.id, Rating::Easy, true)
            .await
            .expect("review should persist");

        assert_eq!(reviewed_question.repetitions, 1);
        assert_eq!(reviewed_question.interval_days, 1);
        assert!(reviewed_question.last_reviewed_at.is_some());
        assert!(reviewed_question.next_review_at.is_some());

        let reloaded_question = get_questions()
            .await
            .expect("reloaded questions should load")
            .into_iter()
            .next()
            .expect("expected one reloaded question");

        assert_eq!(reloaded_question.repetitions, 1);
        assert_eq!(reloaded_question.interval_days, 1);
        assert!(reloaded_question.last_reviewed_at.is_some());
        assert!(reloaded_question.next_review_at.is_some());

        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(db_path);
    }
}
