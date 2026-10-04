use super::queue::plan_for_day;
use super::*;
use crate::models::learning_item::{
    LearningItem, RecallState, ReviewRating, ReviewResponse, ReviewSubmission,
};
use crate::models::study_plan::{DailyStudyPlan, StudyPreferences};
use crate::services::learning_items;
use chrono::Local;
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

async fn fixture() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::super::database::run_migrations(&mut *pool.acquire().await.unwrap())
        .await
        .unwrap();
    sqlx::query("INSERT INTO recall_spaces(id,name) VALUES(2,'Second')")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

async fn item(pool: &SqlitePool, id: i64, space: i64, state: &str, due: Option<&str>) {
    sqlx::query("INSERT INTO learning_items(id,target,answer,space_id,recall_state,next_review_at) VALUES(?,'Target','Answer',?,?,?)")
        .bind(id).bind(space).bind(state).bind(due).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO question_variants(id,learning_item_id,format,prompt,content_json) VALUES(?,?,'flashcard','Prompt','{}')")
        .bind(id).bind(id).execute(pool).await.unwrap();
}

async fn generated_after_start(pool: &SqlitePool, id: i64, space: i64) {
    item(pool, id, space, "new", None).await;
    // Use the persisted cutoff so this regression does not depend on clock timing.
    sqlx::query("UPDATE learning_items SET generated_at=(SELECT strftime('%Y-%m-%dT%H:%M:%SZ',started_at,'+1 second') FROM daily_study_state WHERE local_date=?) WHERE id=?")
        .bind(local_day()).bind(id).execute(pool).await.unwrap();
}

async fn limits(pool: &SqlitePool, daily_target: i64, max_new_items: i64) {
    save_preferences(
        pool,
        StudyPreferences {
            daily_target,
            max_new_items,
        },
        false,
    )
    .await
    .unwrap();
}

fn submission(id: i64) -> ReviewSubmission {
    ReviewSubmission {
        learning_item_id: id,
        variant_id: id,
        response: ReviewResponse::Flashcard {
            rating: ReviewRating::Good,
        },
    }
}

fn ids(plan: &DailyStudyPlan) -> Vec<i64> {
    plan.items.iter().map(|planned| planned.item.id).collect()
}

async fn submit(
    pool: &SqlitePool,
    plan: &DailyStudyPlan,
    item_id: i64,
) -> Result<LearningItem, sqlx::Error> {
    let is_extra = plan
        .items
        .iter()
        .find(|item| item.item.id == item_id)
        .unwrap()
        .is_extra;
    review(
        pool,
        &plan.local_date,
        plan.space_id,
        is_extra,
        submission(item_id),
    )
    .await
}

#[tokio::test]
async fn due_first_with_id_ties_new_fill_and_no_future_reviews() {
    let pool = fixture().await;
    limits(&pool, 5, 2).await;
    item(&pool, 1, 1, "new", None).await;
    item(&pool, 2, 1, "scheduled", Some("2000-01-02 00:00:00")).await;
    item(&pool, 3, 2, "scheduled", Some("2000-01-01 00:00:00")).await;
    item(&pool, 4, 1, "scheduled", Some("2000-01-01 00:00:00")).await;
    item(&pool, 5, 1, "scheduled", Some("2999-01-01 00:00:00")).await;
    item(&pool, 6, 2, "new", None).await;
    item(&pool, 7, 2, "new", None).await;

    let plan = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&plan), [3, 4, 2, 1, 6]);
    assert_eq!(plan.total_count, 5);
    assert_eq!(plan.items.iter().filter(|item| item.was_new).count(), 2);
}

#[tokio::test]
async fn caps_reviews_and_assigns_fewer_when_only_new_are_available() {
    let pool = fixture().await;
    limits(&pool, 3, 1).await;
    for id in 1..=5 {
        item(&pool, id, 1, "scheduled", None).await;
    }
    item(&pool, 6, 2, "new", None).await;
    item(&pool, 7, 2, "new", None).await;

    assert_eq!(ids(&get_plan(&pool, None, false).await.unwrap()), [1, 2, 3]);
    let focused = get_plan(&pool, Some(2), false).await.unwrap();
    assert_eq!(ids(&focused), [6]);
    assert_eq!(focused.total_count, 1);

    save_preferences(
        &pool,
        StudyPreferences {
            daily_target: 3,
            max_new_items: 0,
        },
        true,
    )
    .await
    .unwrap();
    assert!(get_plan(&pool, Some(2), false)
        .await
        .unwrap()
        .items
        .is_empty());
}

#[tokio::test]
async fn rejects_invalid_settings_in_backend_and_database() {
    let pool = fixture().await;
    for (daily_target, max_new_items) in [(0, 0), (-1, 0), (2, 3), (2, -1), (10001, 0)] {
        assert!(save_preferences(
            &pool,
            StudyPreferences {
                daily_target,
                max_new_items
            },
            true
        )
        .await
        .is_err());
    }
    assert_eq!(
        preferences(&pool).await.unwrap(),
        StudyPreferences {
            daily_target: 20,
            max_new_items: 5
        }
    );
    assert!(sqlx::query("UPDATE study_preferences SET max_new_items=21")
        .execute(&pool)
        .await
        .is_err());
}

#[tokio::test]
async fn spaces_share_daily_target_and_new_item_quota() {
    let pool = fixture().await;
    limits(&pool, 3, 1).await;
    item(&pool, 1, 1, "new", None).await;
    item(&pool, 2, 2, "new", None).await;
    item(&pool, 3, 2, "scheduled", None).await;
    item(&pool, 4, 2, "scheduled", None).await;

    let first = get_plan(&pool, Some(1), false).await.unwrap();
    submit(&pool, &first, 1).await.unwrap();

    let second = get_plan(&pool, Some(2), false).await.unwrap();
    assert_eq!(second.completed_count, 1);
    assert_eq!(second.total_count, 3);
    assert_eq!(ids(&second), [3, 4]);
    assert!(second.items.iter().all(|item| !item.was_new));

    let global = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&global), [3, 4]);
    assert_eq!(global.completed_count, 1);
    assert!(get_plan(&pool, Some(999), false).await.is_err());
}

#[tokio::test]
async fn successful_review_is_atomic_and_retry_is_idempotent() {
    let pool = fixture().await;
    item(&pool, 1, 1, "new", None).await;
    let plan = get_plan(&pool, None, false).await.unwrap();

    let first = submit(&pool, &plan, 1).await.unwrap();
    let retry = review(&pool, &plan.local_date, None, false, submission(1))
        .await
        .unwrap();
    assert_eq!(first.schedule, retry.schedule);
    assert_eq!(retry.recall_state, RecallState::Scheduled);
    assert_eq!(
        get_plan(&pool, None, false).await.unwrap().completed_count,
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM review_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn event_failure_rolls_back_schedule_and_progress() {
    let pool = fixture().await;
    item(&pool, 1, 1, "new", None).await;
    let plan = get_plan(&pool, None, false).await.unwrap();
    sqlx::query("CREATE TRIGGER fail_event BEFORE INSERT ON review_events BEGIN SELECT RAISE(ABORT,'test failure'); END")
        .execute(&pool).await.unwrap();

    assert!(submit(&pool, &plan, 1).await.is_err());
    let stored = learning_items::load(&mut *pool.acquire().await.unwrap(), 1)
        .await
        .unwrap();
    assert_eq!(stored.recall_state, RecallState::New);
    assert!(stored.schedule.last_reviewed_at.is_none());
    assert_eq!(
        get_plan(&pool, None, false).await.unwrap().completed_count,
        0
    );
}

#[tokio::test]
async fn generated_items_fill_an_initially_empty_plan_on_refresh() {
    let pool = fixture().await;
    limits(&pool, 20, 5).await;
    let empty = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(empty.total_count, 0);
    assert!(empty.items.is_empty());

    for id in 1..=10 {
        generated_after_start(&pool, id, 1).await;
    }

    let refreshed = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&refreshed), [1, 2, 3, 4, 5]);
    assert_eq!(refreshed.total_count, 5);
    assert_eq!(refreshed.completed_count, 0);
    assert!(refreshed.items.iter().all(|item| item.was_new));
    assert_eq!(
        ids(&get_plan(&pool, None, false).await.unwrap()),
        ids(&refreshed)
    );

    let reviewed = submit(&pool, &refreshed, 1).await.unwrap();
    assert_eq!(reviewed.recall_state, RecallState::Scheduled);
    let after = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(after.completed_count, 1);
    assert_eq!(ids(&after), [2, 3, 4, 5]);
}

#[tokio::test]
async fn generated_items_fill_remaining_capacity_without_displacing_session_items() {
    let pool = fixture().await;
    limits(&pool, 6, 4).await;
    item(&pool, 1, 1, "scheduled", Some("2000-01-01 00:00:00")).await;
    item(&pool, 2, 1, "new", None).await;
    item(&pool, 3, 1, "new", None).await;
    let session = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&session), [1, 2, 3]);
    submit(&pool, &session, 2).await.unwrap();

    for id in 4..=7 {
        generated_after_start(&pool, id, 2).await;
    }
    let refreshed = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&refreshed), [1, 3, 4, 5]);
    assert_eq!(refreshed.completed_count, 1);
    assert_eq!(refreshed.total_count, 5);
    assert!(!refreshed.items[0].was_new);

    // The remaining items from the session started before generation still submit.
    submit(&pool, &session, 1).await.unwrap();
    submit(&pool, &session, 3).await.unwrap();
    let focused = get_plan(&pool, Some(2), false).await.unwrap();
    assert_eq!(focused.completed_count, 3);
    assert_eq!(ids(&focused), [4, 5]);
    submit(&pool, &focused, 4).await.unwrap();
    submit(&pool, &focused, 5).await.unwrap();

    let done = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(done.completed_count, 5);
    assert!(done.items.is_empty());
    assert!(!done.can_study_more);
}

#[tokio::test]
async fn generated_items_cannot_exceed_daily_target_or_new_allowance() {
    for (daily_target, max_new_items) in [(2, 2), (6, 1), (6, 0)] {
        let pool = fixture().await;
        limits(&pool, daily_target, max_new_items).await;
        item(&pool, 1, 1, "scheduled", None).await;
        item(&pool, 2, 1, "new", None).await;
        let session = get_plan(&pool, None, false).await.unwrap();
        for id in ids(&session) {
            submit(&pool, &session, id).await.unwrap();
        }
        for id in 3..=5 {
            generated_after_start(&pool, id, 2).await;
        }

        for scope in [None, Some(2)] {
            let after = get_plan(&pool, scope, false).await.unwrap();
            assert_eq!(after.completed_count, session.total_count);
            assert!(after.items.is_empty());
            assert!(
                review(&pool, &after.local_date, scope, false, submission(3))
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn daily_state_freezes_settings_and_review_cutoff_but_allows_new_generation() {
    let pool = fixture().await;
    limits(&pool, 3, 2).await;
    item(&pool, 1, 1, "new", None).await;
    let original = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&original), [1]);
    sqlx::query("UPDATE daily_study_state SET started_at='2026-01-01 00:00:00'")
        .execute(&pool)
        .await
        .unwrap();
    item(&pool, 2, 1, "scheduled", None).await;
    sqlx::query("UPDATE learning_items SET generated_at='2026-01-02T00:00:00Z' WHERE id=2")
        .execute(&pool)
        .await
        .unwrap();
    generated_after_start(&pool, 3, 1).await;
    item(&pool, 4, 1, "scheduled", Some("2026-01-01 00:00:01")).await;
    limits(&pool, 8, 3).await;

    let same = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(ids(&same), [1, 3]);
    assert_eq!(same.daily_target, 3);
    assert_eq!(same.max_new_items, 2);

    let tomorrow = (Local::now().date_naive() + chrono::Duration::days(1)).to_string();
    let next = plan_for_day(&pool, &tomorrow, None, false).await.unwrap();
    assert_eq!(next.daily_target, 8);
    assert_eq!(next.max_new_items, 3);
    assert_eq!(ids(&next), [2, 4, 1, 3]);
}

#[tokio::test]
async fn applying_lower_target_today_preserves_completed_work() {
    let pool = fixture().await;
    limits(&pool, 3, 1).await;
    for id in 1..=4 {
        item(&pool, id, 1, "scheduled", None).await;
    }
    let first = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &first, 1).await.unwrap();
    let second = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &second, 2).await.unwrap();

    save_preferences(
        &pool,
        StudyPreferences {
            daily_target: 1,
            max_new_items: 0,
        },
        true,
    )
    .await
    .unwrap();
    let reduced = get_plan(&pool, None, false).await.unwrap();
    assert_eq!((reduced.completed_count, reduced.total_count), (2, 2));
    assert!(reduced.items.is_empty());
    assert_eq!(reduced.daily_target, 1);
}

#[tokio::test]
async fn study_more_keeps_original_target_and_daily_new_limit() {
    let pool = fixture().await;
    limits(&pool, 1, 1).await;
    item(&pool, 1, 1, "new", None).await;
    item(&pool, 2, 1, "new", None).await;

    let plan = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &plan, 1).await.unwrap();
    assert!(!get_plan(&pool, None, false).await.unwrap().can_study_more);

    item(&pool, 3, 1, "scheduled", None).await;
    let extra = get_plan(&pool, None, true).await.unwrap();
    assert_eq!(ids(&extra), [3]);
    assert!(extra.items[0].is_extra);
    submit(&pool, &extra, 3).await.unwrap();

    let done = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(
        (
            done.completed_count,
            done.total_count,
            done.extra_completed_count
        ),
        (1, 1, 1)
    );
    assert!(!done.can_study_more);
}

#[tokio::test]
async fn deletion_preserves_completed_progress() {
    let pool = fixture().await;
    item(&pool, 1, 1, "new", None).await;
    item(&pool, 2, 1, "new", None).await;
    let plan = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &plan, 1).await.unwrap();

    sqlx::query("DELETE FROM learning_items")
        .execute(&pool)
        .await
        .unwrap();
    let after = get_plan(&pool, None, false).await.unwrap();
    assert_eq!((after.completed_count, after.total_count), (1, 1));
    assert!(after.items.is_empty());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM review_events WHERE learning_item_id IS NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn yesterday_queue_cannot_be_submitted_today() {
    let pool = fixture().await;
    item(&pool, 1, 1, "new", None).await;
    let yesterday = (Local::now().date_naive() - chrono::Duration::days(1)).to_string();
    let old = plan_for_day(&pool, &yesterday, None, false).await.unwrap();

    assert!(review(&pool, &old.local_date, None, false, submission(1))
        .await
        .is_err());
    assert_eq!(
        get_plan(&pool, None, false).await.unwrap().completed_count,
        0
    );
}

#[tokio::test]
async fn daily_state_and_progress_survive_database_reopen() {
    let path = std::env::temp_dir().join(format!(
        "arka-plan-{}-{}.db",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap()
    ));
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    super::super::database::run_migrations(&mut *pool.acquire().await.unwrap())
        .await
        .unwrap();
    limits(&pool, 3, 2).await;
    assert!(get_plan(&pool, None, false).await.unwrap().items.is_empty());
    generated_after_start(&pool, 1, 1).await;
    let before = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &before, 1).await.unwrap();
    generated_after_start(&pool, 2, 1).await;
    pool.close().await;

    let reopened = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let after = get_plan(&reopened, None, false).await.unwrap();
    assert_eq!(after.completed_count, 1);
    assert_eq!(ids(&after), [2]);
    assert_eq!((after.daily_target, after.max_new_items), (3, 2));
    submit(&reopened, &after, 2).await.unwrap();
    let done = get_plan(&reopened, None, false).await.unwrap();
    assert_eq!(done.completed_count, 2);
    assert!(done.items.is_empty());
    reopened.close().await;
    std::fs::remove_file(path).unwrap();
}
