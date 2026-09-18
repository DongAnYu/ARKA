use super::super::database_backup::restore_copy;
use super::{run_migrations, MIGRATOR};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
use sqlx::{Connection, SqliteConnection};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static FIXTURE_COUNTER: AtomicU64 = AtomicU64::new(0);
const V016_LAST_MIGRATION: i64 = 20260831090000;
const LEARNING_ITEMS_MIGRATION: i64 = 20260907000100;
const DAILY_STUDY_PLANS_MIGRATION: i64 = 20260916000100;

struct Fixture {
    directory: PathBuf,
    connection: SqliteConnection,
}

impl Fixture {
    async fn empty_v016() -> Self {
        Self::v016(false).await
    }

    async fn populated_v016() -> Self {
        Self::v016(true).await
    }

    async fn v016(populated: bool) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "arka-upgrade-{}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap(),
            FIXTURE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let options = SqliteConnectOptions::new()
            .filename(directory.join("review.db"))
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal);
        let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
        // Use real release migrations AND their SQLx checksums/ledger. Newer
        // migrations must not leak into the baseline as the project evolves.
        v016_migrator().run(&mut connection).await.unwrap();
        if populated {
            sqlx::raw_sql(include_str!(
                "../../tests/fixtures/migrations/v016_populated.sql"
            ))
            .execute(&mut connection)
            .await
            .unwrap();
        }
        Self {
            directory,
            connection,
        }
    }

    async fn close(self) {
        self.connection.close().await.unwrap();
        // Only this test's uniquely created directory is removed.
        std::fs::remove_dir_all(self.directory).unwrap();
    }
}

async fn count(connection: &mut SqliteConnection, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(connection).await.unwrap()
}

async fn assert_all_current_migrations_applied(connection: &mut SqliteConnection) {
    let applied = count(
        connection,
        "SELECT COUNT(*) FROM _sqlx_migrations WHERE success=1",
    )
    .await;
    assert_eq!(applied, MIGRATOR.iter().count() as i64);
    assert_eq!(count(connection, "SELECT COUNT(*) FROM study_preferences WHERE id=1 AND daily_target=20 AND max_new_items=5").await, 1);
    assert_eq!(count(connection, "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('daily_study_state','review_events')").await, 2);
    assert_eq!(count(connection, "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('daily_study_plans','daily_study_plan_items')").await, 0);
    assert_eq!(
        count(
            connection,
            "SELECT COUNT(*) FROM user_profile WHERE id=1 AND display_name=''"
        )
        .await,
        1
    );
}

#[tokio::test]
async fn derived_queue_migration_preserves_preferences_daily_state_and_events() {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    migrator_through(DAILY_STUDY_PLANS_MIGRATION)
        .run(&mut connection)
        .await
        .unwrap();

    sqlx::query("UPDATE study_preferences SET daily_target=42,max_new_items=7 WHERE id=1")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO daily_study_plans(id,local_date,utc_offset_seconds,daily_target,max_new_items,space_id) VALUES(9,'2026-09-16',3600,30,6,1)")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO learning_items(id,answer,space_id,recall_state) VALUES(500,'Answer',1,'scheduled')")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO daily_study_plan_items(id,plan_id,learning_item_id,was_new,is_extra,completed_at) VALUES(700,9,500,1,1,'2026-09-16 09:30:00')")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("INSERT INTO review_events(id,learning_item_id,plan_item_id,local_date,was_new,reviewed_at) VALUES(900,500,700,'2026-09-16',1,'2026-09-16 09:30:00')")
        .execute(&mut connection)
        .await
        .unwrap();

    MIGRATOR.run(&mut connection).await.unwrap();

    assert_eq!(count(&mut connection, "SELECT COUNT(*) FROM study_preferences WHERE id=1 AND daily_target=42 AND max_new_items=7").await, 1);
    assert_eq!(count(&mut connection, "SELECT COUNT(*) FROM daily_study_state WHERE local_date='2026-09-16' AND started_at IS NOT NULL AND utc_offset_seconds=3600 AND daily_target=30 AND max_new_items=6").await, 1);
    assert_eq!(count(&mut connection, "SELECT COUNT(*) FROM review_events WHERE id=900 AND learning_item_id=500 AND local_date='2026-09-16' AND was_new=1 AND is_extra=1 AND reviewed_at='2026-09-16 09:30:00'").await, 1);
    assert_eq!(count(&mut connection, "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('daily_study_plans','daily_study_plan_items')").await, 0);
    assert!(sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut connection)
        .await
        .unwrap()
        .is_empty());
    connection.close().await.unwrap();
}

#[tokio::test]
async fn populated_v016_upgrades_to_current_schema() {
    let mut fixture = Fixture::populated_v016().await;
    let backup_path = run_migrations(&mut fixture.connection).await.unwrap();
    assert!(backup_path.unwrap().is_file());
    assert_all_current_migrations_applied(&mut fixture.connection).await;
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM learning_items"
        )
        .await,
        4
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM question_variants WHERE format='flashcard'"
        )
        .await,
        0
    );
    let answers: Vec<String> = sqlx::query_scalar("SELECT answer FROM learning_items ORDER BY id")
        .fetch_all(&mut fixture.connection)
        .await
        .unwrap();
    assert_eq!(answers, ["first", "O(log n)", "丙", "d"]);
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM learning_items WHERE target IS NULL AND provider IS NULL AND generated_at IS NULL AND source_json IS NULL").await, 4);
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM legacy_review_history"
        )
        .await,
        3
    );
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM learning_items WHERE id=20 AND repetitions=4 AND interval_days=38 AND ease_factor=2.5 AND next_review_at='2026-10-10 12:00:00' AND model='' AND space_id=7").await, 1);
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM learning_items WHERE recall_state='scheduled'"
        )
        .await,
        4
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM pragma_table_info('learning_items') WHERE name='status'"
        )
        .await,
        0
    );
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM model_settings WHERE selected_model='configured-model' AND api_key='fixture-only-not-a-real-key' AND llm_concurrency=3").await, 1);
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM legacy_review_history WHERE id=105 AND question_id=20 AND is_correct=0 AND reviewed_at='2026-08-01 01:02:03'").await, 1);
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='questions'"
        )
        .await,
        0
    );
    fixture.close().await;
}

#[tokio::test]
async fn empty_database_and_repeat_upgrade_are_safe() {
    let mut fixture = Fixture::empty_v016().await;
    run_migrations(&mut fixture.connection).await.unwrap();
    let before = std::fs::read_dir(&fixture.directory).unwrap().count();
    let repeated = run_migrations(&mut fixture.connection).await.unwrap();
    assert!(repeated.is_none());
    assert_eq!(
        std::fs::read_dir(&fixture.directory).unwrap().count(),
        before
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM learning_items"
        )
        .await,
        0
    );
    fixture.close().await;
}

#[tokio::test]
async fn invalid_answer_fails_closed_with_recoverable_original() {
    let mut fixture = Fixture::populated_v016().await;
    sqlx::query("UPDATE questions SET correct_answer='invalid' WHERE id=20")
        .execute(&mut fixture.connection)
        .await
        .unwrap();
    let error = run_migrations(&mut fixture.connection).await.unwrap_err();
    assert!(error.backup_path.as_ref().unwrap().is_file());
    assert!(error.to_string().contains("Close ARKA"));
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM questions WHERE id=20 AND correct_answer='invalid'"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='learning_items'"
        )
        .await,
        0
    );
    fixture.close().await;
}

#[tokio::test]
async fn late_copy_failure_rolls_back_all_schema_changes() {
    let mut fixture = Fixture::populated_v016().await;
    // A space FK did not exist in the old questions table. Failure occurs after DDL.
    sqlx::query("UPDATE questions SET space_id=999 WHERE id=20")
        .execute(&mut fixture.connection)
        .await
        .unwrap();
    assert!(run_migrations(&mut fixture.connection).await.is_err());
    assert_eq!(
        count(&mut fixture.connection, "SELECT COUNT(*) FROM questions").await,
        4
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM review_history"
        )
        .await,
        3
    );
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('learning_items','question_variants','arka_content_schema')").await, 0);
    fixture.close().await;
}

#[tokio::test]
async fn interrupted_transaction_is_rolled_back_on_reopen() {
    let mut fixture = Fixture::populated_v016().await;
    sqlx::raw_sql("BEGIN IMMEDIATE;")
        .execute(&mut fixture.connection)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/20260907000100_learning_items.sql"
    ))
    .execute(&mut fixture.connection)
    .await
    .unwrap();
    fixture.connection.close().await.unwrap();
    let options = SqliteConnectOptions::new()
        .filename(fixture.directory.join("review.db"))
        .foreign_keys(true);
    fixture.connection = SqliteConnection::connect_with(&options).await.unwrap();
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='learning_items'"
        )
        .await,
        0
    );
    assert_eq!(
        count(&mut fixture.connection, "SELECT COUNT(*) FROM questions").await,
        4
    );
    run_migrations(&mut fixture.connection).await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn wal_backup_restores_a_separate_v016_database_without_overwriting() {
    let mut fixture = Fixture::populated_v016().await;
    let backup = run_migrations(&mut fixture.connection)
        .await
        .unwrap()
        .unwrap();
    let destination = fixture.directory.join("restored.db");
    restore_copy(&backup, &destination).await.unwrap();
    assert!(restore_copy(&backup, &destination).await.is_err());
    let options = SqliteConnectOptions::new()
        .filename(&destination)
        .read_only(true);
    let mut restored = SqliteConnection::connect_with(&options).await.unwrap();
    assert_eq!(
        count(&mut restored, "SELECT COUNT(*) FROM questions").await,
        4
    );
    assert_eq!(
        count(&mut restored, "SELECT COUNT(*) FROM review_history").await,
        3
    );
    assert_eq!(
        count(
            &mut restored,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='learning_items'"
        )
        .await,
        0
    );
    restored.close().await.unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn current_schema_enforces_parent_schedule_variants_and_cascades() {
    let mut fixture = Fixture::populated_v016().await;
    run_migrations(&mut fixture.connection).await.unwrap();
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='review_history'"
        )
        .await,
        0
    );
    assert_eq!(count(&mut fixture.connection, "SELECT COUNT(*) FROM pragma_table_info('learning_items') WHERE name IN ('rating','is_correct','attempt_id','schedule_version')").await, 0);
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM pragma_table_info('legacy_review_history')"
        )
        .await,
        4
    );
    assert!(
        sqlx::query("UPDATE learning_items SET recall_state='invalid' WHERE id=20")
            .execute(&mut fixture.connection)
            .await
            .is_err()
    );
    // Renderers use explicit variant formats, independently of the parent schedule.
    sqlx::query("INSERT INTO question_variants(id,learning_item_id,format,prompt,content_json) VALUES(100,20,'flashcard','What is the complexity?','{}')").execute(&mut fixture.connection).await.unwrap();
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM question_variants WHERE learning_item_id=20"
        )
        .await,
        2
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM learning_items WHERE id=20 AND interval_days=38"
        )
        .await,
        1
    );
    sqlx::query("DELETE FROM learning_items WHERE id=20")
        .execute(&mut fixture.connection)
        .await
        .unwrap();
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM legacy_review_history WHERE question_id=20"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &mut fixture.connection,
            "SELECT COUNT(*) FROM question_variants WHERE learning_item_id=20"
        )
        .await,
        0
    );
    fixture.close().await;
}

#[tokio::test]
async fn migration_refuses_in_memory_database_without_backup() {
    let options = SqliteConnectOptions::new().in_memory(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    sqlx::query("CREATE TABLE user_data(id INTEGER)")
        .execute(&mut connection)
        .await
        .unwrap();
    let error = run_migrations(&mut connection).await.unwrap_err();
    assert!(error.message.contains("file-backed"));
    assert!(error.backup_path.is_none());
    connection.close().await.unwrap();
}

pub(crate) fn v016_migrator() -> sqlx::migrate::Migrator {
    migrator_through(V016_LAST_MIGRATION)
}

fn migrator_through(version: i64) -> sqlx::migrate::Migrator {
    sqlx::migrate::Migrator {
        migrations: std::borrow::Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ..sqlx::migrate::Migrator::DEFAULT
    }
}

#[tokio::test]
async fn fresh_install_uses_only_sqlx_history_without_backup() {
    let options = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();
    assert!(run_migrations(&mut connection).await.unwrap().is_none());
    assert_all_current_migrations_applied(&mut connection).await;
    assert_eq!(
        count(&mut connection, "SELECT COUNT(*) FROM learning_items").await,
        0
    );
    assert_eq!(
        count(
            &mut connection,
            "SELECT COUNT(*) FROM sqlite_master WHERE name='arka_content_schema'"
        )
        .await,
        0
    );
    connection.close().await.unwrap();
}

#[tokio::test]
async fn sqlx_rejects_changed_checksums_even_without_pending_migrations() {
    let mut fixture = Fixture::empty_v016().await;
    run_migrations(&mut fixture.connection).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=x'00' WHERE version=?")
        .bind(LEARNING_ITEMS_MIGRATION)
        .execute(&mut fixture.connection)
        .await
        .unwrap();
    assert!(run_migrations(&mut fixture.connection).await.is_err());
    fixture.close().await;
}
