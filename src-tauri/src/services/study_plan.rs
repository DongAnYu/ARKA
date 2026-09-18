use super::learning_items;
use crate::models::learning_item::{LearningItem, ReviewSubmission};
use crate::models::study_plan::{DailyStudyPlan, PlannedItem, StudyPreferences};
use chrono::Local;
use sqlx::{SqliteConnection, SqlitePool};

#[derive(Debug)]
struct DailyState {
    local_date: String,
    started_at: String,
    daily_target: i64,
    max_new_items: i64,
}

fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

pub fn local_day() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub async fn preferences(pool: &SqlitePool) -> Result<StudyPreferences, sqlx::Error> {
    read_preferences(&mut *pool.acquire().await?).await
}

async fn read_preferences(conn: &mut SqliteConnection) -> Result<StudyPreferences, sqlx::Error> {
    let (daily_target, max_new_items) =
        sqlx::query_as("SELECT daily_target,max_new_items FROM study_preferences WHERE id=1")
            .fetch_one(conn)
            .await?;
    Ok(StudyPreferences {
        daily_target,
        max_new_items,
    })
}

pub async fn save_preferences(
    pool: &SqlitePool,
    preferences: StudyPreferences,
    apply_today: bool,
) -> Result<StudyPreferences, sqlx::Error> {
    if !(1..=10000).contains(&preferences.daily_target)
        || !(0..=preferences.daily_target).contains(&preferences.max_new_items)
    {
        return Err(invalid("Daily target must be 1–10,000; maximum new items must be between 0 and the daily target."));
    }

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query("UPDATE study_preferences SET daily_target=?,max_new_items=? WHERE id=1")
        .bind(preferences.daily_target)
        .bind(preferences.max_new_items)
        .execute(&mut *tx)
        .await?;
    if apply_today {
        sqlx::query(
            "UPDATE daily_study_state SET daily_target=?,max_new_items=? WHERE local_date=?",
        )
        .bind(preferences.daily_target)
        .bind(preferences.max_new_items)
        .bind(local_day())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(preferences)
}

async fn validate_scope(
    conn: &mut SqliteConnection,
    space_id: Option<i64>,
) -> Result<(), sqlx::Error> {
    if let Some(space_id) = space_id {
        let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM recall_spaces WHERE id=?")
            .bind(space_id)
            .fetch_optional(conn)
            .await?;
        if exists.is_none() {
            return Err(invalid("This Recall Space no longer exists."));
        }
    }
    Ok(())
}

async fn state_for_day(conn: &mut SqliteConnection, day: &str) -> Result<DailyState, sqlx::Error> {
    let preferences = read_preferences(conn).await?;
    sqlx::query(
        "INSERT OR IGNORE INTO daily_study_state(local_date,utc_offset_seconds,daily_target,max_new_items) VALUES(?,?,?,?)",
    )
    .bind(day)
    .bind(Local::now().offset().local_minus_utc())
    .bind(preferences.daily_target)
    .bind(preferences.max_new_items)
    .execute(&mut *conn)
    .await?;

    let (local_date, started_at, daily_target, max_new_items) = sqlx::query_as(
        "SELECT local_date,started_at,daily_target,max_new_items FROM daily_study_state WHERE local_date=?",
    )
    .bind(day)
    .fetch_one(conn)
    .await?;
    Ok(DailyState {
        local_date,
        started_at,
        daily_target,
        max_new_items,
    })
}

async fn daily_counts(
    conn: &mut SqliteConnection,
    day: &str,
) -> Result<(i64, i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT COALESCE(SUM(is_extra=0),0), COALESCE(SUM(is_extra=1),0),
                COALESCE(SUM(was_new=1),0)
         FROM review_events WHERE local_date=?",
    )
    .bind(day)
    .fetch_one(conn)
    .await
}

async fn candidate_ids(
    conn: &mut SqliteConnection,
    state: &DailyState,
    space_id: Option<i64>,
    limit: i64,
) -> Result<Vec<(i64, bool)>, sqlx::Error> {
    if limit <= 0 {
        return Ok(Vec::new());
    }

    let (_, _, new_completed) = daily_counts(conn, &state.local_date).await?;
    let mut candidates = Vec::new();
    let scheduled: Vec<i64> = sqlx::query_scalar(
        "SELECT item.id FROM learning_items item
         WHERE (? IS NULL OR item.space_id=?)
           AND item.recall_state='scheduled'
           AND (item.next_review_at IS NULL OR datetime(item.next_review_at)<=datetime(?))
           AND (item.generated_at IS NULL OR datetime(item.generated_at)<=datetime(?))
           AND EXISTS(SELECT 1 FROM question_variants variant WHERE variant.learning_item_id=item.id)
           AND NOT EXISTS(SELECT 1 FROM review_events event
                          WHERE event.local_date=? AND event.learning_item_id=item.id)
         ORDER BY COALESCE(datetime(item.next_review_at),datetime('1970-01-01')),item.id
         LIMIT ?",
    )
    .bind(space_id)
    .bind(space_id)
    .bind(&state.started_at)
    .bind(&state.started_at)
    .bind(&state.local_date)
    .bind(limit)
    .fetch_all(&mut *conn)
    .await?;
    candidates.extend(scheduled.into_iter().map(|id| (id, false)));

    let remaining = limit - candidates.len() as i64;
    let new_limit = remaining.min((state.max_new_items - new_completed).max(0));
    if new_limit > 0 {
        let new_items: Vec<i64> = sqlx::query_scalar(
            "SELECT item.id FROM learning_items item
             WHERE (? IS NULL OR item.space_id=?)
               AND item.recall_state='new'
               AND (item.generated_at IS NULL OR datetime(item.generated_at)<=datetime(?))
               AND EXISTS(SELECT 1 FROM question_variants variant WHERE variant.learning_item_id=item.id)
               AND NOT EXISTS(SELECT 1 FROM review_events event
                              WHERE event.local_date=? AND event.learning_item_id=item.id)
             ORDER BY item.id
             LIMIT ?",
        )
        .bind(space_id)
        .bind(space_id)
        .bind(&state.started_at)
        .bind(&state.local_date)
        .bind(new_limit)
        .fetch_all(&mut *conn)
        .await?;
        candidates.extend(new_items.into_iter().map(|id| (id, true)));
    }
    Ok(candidates)
}

async fn load_plan_items(
    conn: &mut SqliteConnection,
    candidates: Vec<(i64, bool)>,
    is_extra: bool,
) -> Result<Vec<PlannedItem>, sqlx::Error> {
    let mut items = Vec::with_capacity(candidates.len());
    for (item_id, was_new) in candidates {
        items.push(PlannedItem {
            was_new,
            is_extra,
            item: learning_items::load(conn, item_id).await?,
        });
    }
    Ok(items)
}

async fn snapshot(
    conn: &mut SqliteConnection,
    state: &DailyState,
    space_id: Option<i64>,
    extra: bool,
) -> Result<DailyStudyPlan, sqlx::Error> {
    let (completed_count, extra_completed_count, _) = daily_counts(conn, &state.local_date).await?;
    let remaining = (state.daily_target - completed_count).max(0);
    if extra && remaining > 0 {
        return Err(invalid(
            "Complete today's plan before starting extra study.",
        ));
    }

    let limit = if extra { state.daily_target } else { remaining };
    let candidates = candidate_ids(conn, state, space_id, limit).await?;
    let total_count = if extra {
        completed_count
    } else {
        completed_count + candidates.len() as i64
    };
    let can_study_more =
        remaining == 0 && !candidate_ids(conn, state, space_id, 1).await?.is_empty();
    let items = load_plan_items(conn, candidates, extra).await?;

    Ok(DailyStudyPlan {
        local_date: state.local_date.clone(),
        daily_target: state.daily_target,
        max_new_items: state.max_new_items,
        space_id,
        completed_count,
        total_count,
        extra_completed_count,
        can_study_more,
        items,
    })
}

pub async fn get_plan(
    pool: &SqlitePool,
    space_id: Option<i64>,
    extra: bool,
) -> Result<DailyStudyPlan, sqlx::Error> {
    plan_for_day(pool, &local_day(), space_id, extra).await
}

async fn plan_for_day(
    pool: &SqlitePool,
    day: &str,
    space_id: Option<i64>,
    extra: bool,
) -> Result<DailyStudyPlan, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    validate_scope(&mut tx, space_id).await?;
    let state = state_for_day(&mut tx, day).await?;
    let plan = snapshot(&mut tx, &state, space_id, extra).await?;
    tx.commit().await?;
    Ok(plan)
}

pub async fn review(
    pool: &SqlitePool,
    plan_date: &str,
    space_id: Option<i64>,
    is_extra: bool,
    submission: ReviewSubmission,
) -> Result<LearningItem, sqlx::Error> {
    let day = local_day();
    if plan_date != day {
        return Err(invalid(
            "A new study day has started. Return to Recall for today's plan.",
        ));
    }

    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    validate_scope(&mut tx, space_id).await?;
    let already_reviewed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM review_events WHERE local_date=? AND learning_item_id=?",
    )
    .bind(&day)
    .bind(submission.learning_item_id)
    .fetch_one(&mut *tx)
    .await?;
    if already_reviewed > 0 {
        return learning_items::load(&mut tx, submission.learning_item_id).await;
    }

    let state = state_for_day(&mut tx, &day).await?;
    let (completed_count, _, _) = daily_counts(&mut tx, &day).await?;
    let remaining = (state.daily_target - completed_count).max(0);
    if is_extra && remaining > 0 {
        return Err(invalid(
            "Complete today's plan before starting extra study.",
        ));
    }
    let limit = if is_extra {
        state.daily_target
    } else {
        remaining
    };
    let candidates = candidate_ids(&mut tx, &state, space_id, limit).await?;
    let was_new = candidates
        .iter()
        .find_map(|(item_id, was_new)| {
            (*item_id == submission.learning_item_id).then_some(*was_new)
        })
        .ok_or_else(|| invalid("This item is no longer in the selected study queue. Return to Recall to refresh your session."))?;

    let result = learning_items::review_in_transaction(&mut tx, submission).await?;
    record_event(&mut tx, result.id, was_new, is_extra, &day).await?;
    tx.commit().await?;
    Ok(result)
}

pub(super) async fn record_event(
    conn: &mut SqliteConnection,
    item_id: i64,
    was_new: bool,
    is_extra: bool,
    day: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO review_events(learning_item_id,local_date,was_new,is_extra) VALUES(?,?,?,?)",
    )
    .bind(item_id)
    .bind(day)
    .bind(was_new)
    .bind(is_extra)
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::learning_item::{RecallState, ReviewRating, ReviewResponse};
    use sqlx::sqlite::SqlitePoolOptions;

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
    async fn daily_state_freezes_settings_and_excludes_items_generated_after_start() {
        let pool = fixture().await;
        limits(&pool, 3, 1).await;
        item(&pool, 1, 1, "new", None).await;
        let original = get_plan(&pool, None, false).await.unwrap();
        sqlx::query("UPDATE daily_study_state SET started_at='2026-01-01 00:00:00'")
            .execute(&pool)
            .await
            .unwrap();
        item(&pool, 2, 1, "scheduled", None).await;
        sqlx::query("UPDATE learning_items SET generated_at='2026-01-02T00:00:00Z' WHERE id=2")
            .execute(&pool)
            .await
            .unwrap();
        limits(&pool, 8, 2).await;

        let same = get_plan(&pool, None, false).await.unwrap();
        assert_eq!(ids(&same), ids(&original));
        assert_eq!(same.daily_target, 3);

        let tomorrow = (Local::now().date_naive() + chrono::Duration::days(1)).to_string();
        let next = plan_for_day(&pool, &tomorrow, None, false).await.unwrap();
        assert_eq!(next.daily_target, 8);
        assert_eq!(ids(&next), [2, 1]);
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
        item(&pool, 1, 1, "new", None).await;
        item(&pool, 2, 1, "new", None).await;
        let before = get_plan(&pool, None, false).await.unwrap();
        submit(&pool, &before, 1).await.unwrap();
        pool.close().await;

        let reopened = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        let after = get_plan(&reopened, None, false).await.unwrap();
        assert_eq!(after.completed_count, 1);
        assert_eq!(ids(&after), [2]);
        reopened.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
