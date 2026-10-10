use crate::models::study_plan::{DailyStudyPlan, PlannedItem};
use crate::services::learning_items;
use sqlx::{SqliteConnection, SqlitePool};

use super::invalid;
use super::state::{daily_counts, local_day, state_for_day, DailyState};

pub(super) async fn validate_scope(
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

pub(super) async fn candidate_ids(
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
        // Newly generated items can fill unused capacity within today's fixed limits.
        let new_items: Vec<i64> = sqlx::query_scalar(
            "SELECT item.id FROM learning_items item
             WHERE (? IS NULL OR item.space_id=?)
               AND item.recall_state='new'
               AND EXISTS(SELECT 1 FROM question_variants variant WHERE variant.learning_item_id=item.id)
               AND NOT EXISTS(SELECT 1 FROM review_events event
                              WHERE event.local_date=? AND event.learning_item_id=item.id)
             ORDER BY item.id
             LIMIT ?",
        )
        .bind(space_id)
        .bind(space_id)
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
    let (completed_count, extra_completed_count, new_completed) =
        daily_counts(conn, &state.local_date).await?;
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
    let new_items_blocked_by_limit = if new_completed >= state.max_new_items {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM learning_items item
             WHERE (? IS NULL OR item.space_id=?) AND item.recall_state='new'
               AND EXISTS(SELECT 1 FROM question_variants variant WHERE variant.learning_item_id=item.id)
               AND NOT EXISTS(SELECT 1 FROM review_events event
                              WHERE event.local_date=? AND event.learning_item_id=item.id))",
        )
        .bind(space_id)
        .bind(space_id)
        .bind(&state.local_date)
        .fetch_one(&mut *conn)
        .await?
    } else {
        false
    };
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
        new_items_blocked_by_limit,
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

pub(super) async fn plan_for_day(
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
