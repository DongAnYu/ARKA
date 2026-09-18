use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqliteConnection;
use std::path::PathBuf;
use std::str::FromStr;

use crate::models::learning_item::{
    GenerationMetadata, GenerationPipeline, LearningItem, LearningItemEditInput, LearningItemInput,
    QuestionVariant, ReviewIntervals, ReviewResponse, ReviewSubmission,
};
use crate::models::model_settings::ModelConfig;
use crate::models::question::{Question, QuestionInput};
use crate::models::recall_dashboard::{RecallDashboard, RecallSpaceSummary};
use crate::models::recall_space::RecallSpace;
use crate::services::database_backup;
use crate::services::learning_items;

#[cfg(not(feature = "eval-package"))]
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");
#[cfg(feature = "eval-package")]
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../src-tauri/migrations");

#[derive(Debug)]
pub(super) struct MigrationError {
    message: String,
    backup_path: Option<PathBuf>,
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Database migration stopped: {}", self.message)?;
        if let Some(path) = &self.backup_path {
            write!(
                f,
                "\nPre-migration backup: {}. Close ARKA before recovery. Restore this backup with the compatible app version; later reviews are not included.",
                path.display()
            )?;
        }
        Ok(())
    }
}

impl std::error::Error for MigrationError {}

pub(super) async fn run_migrations(
    connection: &mut SqliteConnection,
) -> Result<Option<PathBuf>, MigrationError> {
    let mut backup_path = None;
    let result: Result<(), String> = async {
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&mut *connection)
            .await
            .map_err(|error| error.to_string())?;

        let has_ledger: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;
        let applied: Vec<i64> = if has_ledger > 0 {
            sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success=1")
                .fetch_all(&mut *connection)
                .await
                .map_err(|error| error.to_string())?
        } else {
            Vec::new()
        };

        let pending = MIGRATOR
            .iter()
            .any(|migration| !applied.contains(&migration.version));
        let existing_tables: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_sqlx_migrations'",
        )
        .fetch_one(&mut *connection)
        .await
        .map_err(|error| error.to_string())?;

        if pending && existing_tables > 0 {
            backup_path = Some(
                database_backup::create(connection)
                    .await
                    .map_err(|error| error.to_string())?,
            );
        }

        MIGRATOR
            .run(&mut *connection)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
    .await;

    result
        .map(|_| backup_path.clone())
        .map_err(|message| MigrationError {
            message,
            backup_path,
        })
}

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

pub(crate) async fn open_pool() -> Result<sqlx::SqlitePool, sqlx::Error> {
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

pub async fn get_learning_items(
    space_id: Option<i64>,
    due_only: bool,
) -> Result<Vec<LearningItem>, sqlx::Error> {
    learning_items::list(&open_pool().await?, space_id, due_only).await
}

pub async fn get_new_learning_items(
    space_id: Option<i64>,
    limit: u32,
) -> Result<Vec<LearningItem>, sqlx::Error> {
    learning_items::list_new(&open_pool().await?, space_id, limit).await
}

pub async fn save_learning_items(
    items: Vec<LearningItemInput>,
) -> Result<Vec<LearningItem>, sqlx::Error> {
    learning_items::save(&open_pool().await?, items).await
}

pub async fn modify_learning_item(
    id: i64,
    item: LearningItemEditInput,
) -> Result<LearningItem, sqlx::Error> {
    learning_items::modify(&open_pool().await?, id, item).await
}

pub async fn review_learning_item(
    submission: ReviewSubmission,
) -> Result<LearningItem, sqlx::Error> {
    learning_items::review(&open_pool().await?, submission).await
}

pub async fn get_learning_item_review_intervals(id: i64) -> Result<ReviewIntervals, sqlx::Error> {
    learning_items::review_intervals(&open_pool().await?, id).await
}

async fn mcq_list(space_id: Option<i64>, due_only: bool) -> Result<Vec<Question>, sqlx::Error> {
    get_learning_items(space_id, due_only)
        .await?
        .into_iter()
        .map(learning_items::to_mcq)
        .filter_map(|result| result.transpose())
        .collect()
}

pub async fn get_questions() -> Result<Vec<Question>, sqlx::Error> {
    mcq_list(None, false).await
}
pub async fn get_questions_by_space(space_id: i64) -> Result<Vec<Question>, sqlx::Error> {
    mcq_list(Some(space_id), false).await
}
pub async fn get_due_questions(space_id: Option<i64>) -> Result<Vec<Question>, sqlx::Error> {
    mcq_list(space_id.filter(|id| *id > 0), true).await
}

pub async fn get_recall_dashboard() -> Result<RecallDashboard, sqlx::Error> {
    let pool = open_pool().await?;
    let rows = sqlx::query_as::<_, (i64, String, i64, i64, i64, i64, i64)>(
        "SELECT s.id,s.name,COUNT(i.id),
        COALESCE(SUM(CASE WHEN i.id IS NOT NULL AND i.recall_state='scheduled' AND (i.next_review_at IS NULL OR i.next_review_at<=CURRENT_TIMESTAMP) THEN 1 ELSE 0 END),0),
        COALESCE(SUM(CASE WHEN i.recall_state='scheduled' AND i.next_review_at<date('now') THEN 1 ELSE 0 END),0),
        COALESCE(SUM(CASE WHEN i.recall_state='new' THEN 1 ELSE 0 END),0),
        COALESCE(SUM(CASE WHEN i.last_reviewed_at>=date('now') AND i.last_reviewed_at<date('now','+1 day') THEN 1 ELSE 0 END),0)
        FROM recall_spaces s LEFT JOIN learning_items i ON i.space_id=s.id GROUP BY s.id,s.name ORDER BY s.id")
        .fetch_all(&pool).await?;
    let spaces: Vec<_> = rows
        .into_iter()
        .map(
            |(
                id,
                name,
                total_questions,
                due_count,
                overdue_count,
                new_count,
                reviewed_today_count,
            )| {
                RecallSpaceSummary {
                    id,
                    name,
                    total_questions,
                    due_count,
                    overdue_count,
                    new_count,
                    reviewed_today_count,
                }
            },
        )
        .collect();
    Ok(RecallDashboard {
        due_today_count: spaces.iter().map(|s| s.due_count - s.overdue_count).sum(),
        overdue_count: spaces.iter().map(|s| s.overdue_count).sum(),
        new_count: spaces.iter().map(|s| s.new_count).sum(),
        reviewed_today_count: spaces.iter().map(|s| s.reviewed_today_count).sum(),
        spaces,
    })
}

// The existing frontend sends an A-D label; map it to the stored stable option ID.
pub async fn review_question(
    question_id: i64,
    selected_option_id: String,
) -> Result<Question, sqlx::Error> {
    let pool = open_pool().await?;
    let item = learning_items::load(&mut *pool.acquire().await?, question_id).await?;
    let variant = item
        .variants
        .iter()
        .find_map(|v| {
            if let QuestionVariant::Mcq { id, options, .. } = v {
                Some((*id, options))
            } else {
                None
            }
        })
        .ok_or(sqlx::Error::RowNotFound)?;
    let index = match selected_option_id.as_str() {
        "A" => 0,
        "B" => 1,
        "C" => 2,
        "D" => 3,
        _ => return Err(sqlx::Error::Protocol("Invalid answer label".into())),
    };
    let option_id = variant
        .1
        .get(index)
        .ok_or(sqlx::Error::RowNotFound)?
        .id
        .clone();
    let reviewed = learning_items::review(
        &pool,
        ReviewSubmission {
            learning_item_id: question_id,
            variant_id: variant.0,
            response: ReviewResponse::Mcq {
                selected_option_id: option_id,
            },
        },
    )
    .await?;
    learning_items::to_mcq(reviewed)?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn modify_question(
    id: i64,
    question_input: QuestionInput,
) -> Result<Question, sqlx::Error> {
    let pool = open_pool().await?;
    let existing = learning_items::load(&mut *pool.acquire().await?, id).await?;
    let input = learning_items::from_mcq(question_input, String::new())?;
    let mut edit = LearningItemEditInput {
        target: existing.target,
        answer: input.answer,
        explanation: input.explanation,
        space_id: input.space_id,
        variants: input.variants,
    };
    // Old MCQ editor must not delete a flashcard attached to the same parent.
    for variant in existing.variants {
        if let QuestionVariant::Flashcard { prompt, .. } = variant {
            edit.variants
                .push(crate::models::learning_item::VariantInput::Flashcard { prompt });
        }
    }
    learning_items::to_mcq(learning_items::modify(&pool, id, edit).await?)?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn delete_question(id: i64) -> Result<(), sqlx::Error> {
    delete_questions(vec![id]).await
}
pub async fn delete_questions(ids: Vec<i64>) -> Result<(), sqlx::Error> {
    let pool = open_pool().await?;
    let mut transaction = pool.begin().await?;
    for id in ids {
        sqlx::query("DELETE FROM learning_items WHERE id=?")
            .bind(id)
            .execute(&mut *transaction)
            .await?;
    }
    transaction.commit().await
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
    let mut transaction = pool.begin().await?;

    // Delete all questions inside this space before deleting the space.
    sqlx::query("DELETE FROM learning_items WHERE space_id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;

    sqlx::query("DELETE FROM recall_spaces WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await?;

    transaction.commit().await
}

#[cfg(test)]
pub async fn save_questions(
    questions: Vec<QuestionInput>,
    model: String,
) -> Result<(), sqlx::Error> {
    save_questions_with_generation(
        questions,
        GenerationMetadata {
            model: Some(model),
            provider: None,
            pipeline: None,
            generated_at: None,
        },
    )
    .await
}

pub async fn save_questions_with_generation(
    questions: Vec<QuestionInput>,
    generation: GenerationMetadata,
) -> Result<(), sqlx::Error> {
    if generation.pipeline == Some(GenerationPipeline::Chunk) {
        return Err(sqlx::Error::Protocol(String::from(
            "Chunk generation must be saved through the learning-item draft workflow",
        )));
    }

    let inputs = questions
        .into_iter()
        .map(|q| {
            let mut input =
                learning_items::from_mcq(q, generation.model.clone().unwrap_or_default())?;
            input.generation = generation.clone();
            Ok::<LearningItemInput, sqlx::Error>(input)
        })
        .collect::<Result<Vec<_>, _>>()?;
    save_learning_items(inputs).await?;
    Ok(())
}

pub async fn load_model_config() -> Result<ModelConfig, sqlx::Error> {
    let pool = open_pool().await?;

    let config = sqlx::query_as::<_, ModelConfig>(
        "SELECT provider, base_url, selected_model, timeout_secs, api_key, llm_concurrency,
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
                    id, provider, base_url, selected_model, timeout_secs, api_key, llm_concurrency,
                    embedding_provider, embedding_base_url, embedding_selected_model,
                    embedding_timeout_secs, embedding_api_key, updated_at
                 )
                 VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(id) DO UPDATE SET
            provider = excluded.provider,
            base_url = excluded.base_url,
            selected_model = excluded.selected_model,
            timeout_secs = excluded.timeout_secs,
            api_key = excluded.api_key,
            llm_concurrency = excluded.llm_concurrency,
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
    .bind(config.llm_concurrency)
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

    let mut connection = pool.acquire().await?;
    let backup_path = run_migrations(&mut connection)
        .await
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    if let Some(path) = backup_path {
        log::info!("Pre-migration backup: {}", path.display());
    }
    drop(connection);
    pool.close().await;
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

    #[tokio::test]
    async fn chunk_generation_cannot_use_mcq_compatibility_saver() {
        let error = save_questions_with_generation(
            vec![QuestionInput {
                question: String::from("Legacy-shaped chunk question"),
                option_a: String::from("Correct"),
                option_b: String::from("Distractor one"),
                option_c: String::from("Distractor two"),
                option_d: String::from("Distractor three"),
                correct_answer: String::from("A"),
                explanation: None,
                space_id: 1,
            }],
            GenerationMetadata {
                model: Some(String::from("test-model")),
                provider: Some(String::from("test-provider")),
                pipeline: Some(GenerationPipeline::Chunk),
                generated_at: Some(String::from("2026-09-10T00:00:00Z")),
            },
        )
        .await
        .expect_err("chunk generation must not create an MCQ-only learning item");

        assert!(error.to_string().contains("learning-item draft workflow"));
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
        assert_eq!(config.llm_concurrency, 5);

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
            llm_concurrency: 7,
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
        assert_eq!(config.llm_concurrency, 7);
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
    async fn get_questions_loads_new_items_with_scheduler_defaults() {
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
        assert!(questions[0].next_review_at.is_none());
        assert!(questions[0].last_reviewed_at.is_none());
        let items = get_learning_items(None, false).await.unwrap();
        assert_eq!(
            items[0].recall_state,
            crate::models::learning_item::RecallState::New
        );
        let new_items = get_new_learning_items(None, 1).await.unwrap();
        assert_eq!(new_items.len(), 1);
        assert_eq!(new_items[0].id, items[0].id);
        assert!(get_new_learning_items(None, 0).await.unwrap().is_empty());
        let dashboard = get_recall_dashboard().await.unwrap();
        assert_eq!(dashboard.new_count, 1);
        assert_eq!(dashboard.due_today_count, 0);
        assert_eq!(dashboard.spaces.len(), 1);
        assert_eq!(dashboard.spaces[0].new_count, 1);
        assert_eq!(dashboard.spaces[0].due_count, 0);

        let variant_id = match &items[0].variants[0] {
            QuestionVariant::Mcq { id, .. } => *id,
            QuestionVariant::Flashcard { .. } => panic!("expected an MCQ variant"),
        };
        let reviewed = review_learning_item(ReviewSubmission {
            learning_item_id: items[0].id,
            variant_id,
            response: ReviewResponse::Mcq {
                selected_option_id: String::from("A"),
            },
        })
        .await
        .expect("first review should schedule the new item");
        assert_eq!(
            reviewed.recall_state,
            crate::models::learning_item::RecallState::Scheduled
        );
        assert_eq!(reviewed.schedule.repetitions, 1);
        assert_eq!(reviewed.schedule.interval_days, 1);
        assert!(reviewed.schedule.next_review_at.is_some());

        let dashboard = get_recall_dashboard().await.unwrap();
        assert_eq!(dashboard.new_count, 0);
        assert_eq!(dashboard.due_today_count, 0);
        assert_eq!(dashboard.reviewed_today_count, 1);
        assert_eq!(dashboard.spaces[0].new_count, 0);
        assert_eq!(dashboard.spaces[0].reviewed_today_count, 1);

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
        sqlx::query("UPDATE learning_items SET recall_state = 'scheduled'")
            .execute(&pool)
            .await
            .expect("test questions should enter scheduled recall");
        sqlx::query("UPDATE learning_items SET next_review_at = ? WHERE id IN (SELECT learning_item_id FROM question_variants WHERE prompt = ?)")
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

        let reviewed_question = review_question(saved_question.id, "A".into())
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
    #[tokio::test(flavor = "current_thread")]
    async fn upgraded_backend_supports_restart_edit_review_dashboard_and_delete() {
        let _guard = database_test_lock().lock().unwrap();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("arka-workflow-{stamp}.sqlite"));
        std::env::set_var("DATABASE_URL", format!("sqlite://{}", path.display()));
        // Populate the actual old schema first, then use the real startup path.
        let pool = open_pool().await.unwrap();
        let baseline = super::migration_tests::v016_migrator();
        baseline.run(&pool).await.unwrap();
        sqlx::raw_sql(include_str!(
            "../../tests/fixtures/migrations/v016_populated.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        run_smoke_test().await.unwrap();
        run_smoke_test().await.unwrap();
        assert_eq!(get_questions().await.unwrap().len(), 4);
        let reviewed = review_question(20, "B".into()).await.unwrap();
        assert_eq!(reviewed.repetitions, 5);
        let dashboard = get_recall_dashboard().await.unwrap();
        assert_eq!(dashboard.reviewed_today_count, 1);
        let changed = modify_question(
            20,
            QuestionInput {
                question: "Edited prompt".into(),
                option_a: "O(1)".into(),
                option_b: "O(log n)".into(),
                option_c: "O(n)".into(),
                option_d: "O(n²)".into(),
                correct_answer: "B".into(),
                explanation: None,
                space_id: 7,
            },
        )
        .await
        .unwrap();
        assert_eq!(changed.repetitions, 5);
        assert_eq!(changed.model.as_deref(), Some(""));
        let pool = open_pool().await.unwrap();
        let old_events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM legacy_review_history")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(old_events, 3);
        pool.close().await;
        delete_space(7).await.unwrap();
        assert_eq!(get_questions().await.unwrap().len(), 2);
        assert!(delete_space(1).await.is_err());
        delete_questions(vec![10, 40]).await.unwrap();
        assert!(get_questions().await.unwrap().is_empty());
        std::env::remove_var("DATABASE_URL");
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
#[path = "database_migration_tests.rs"]
mod migration_tests;
