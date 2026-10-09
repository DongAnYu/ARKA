use crate::models::learning_item::{LearningItem, ReviewSubmission};
use crate::services::learning_items;
use sqlx::{SqliteConnection, SqlitePool};

use super::invalid;
use super::queue::{candidate_ids, validate_scope};
use super::state::{daily_counts, local_day, state_for_day};

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

pub(in crate::services) async fn record_event(
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
