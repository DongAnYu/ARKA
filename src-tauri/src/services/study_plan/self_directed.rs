use crate::models::learning_item::{LearningItem, RecallState, ReviewSubmission};
use crate::models::study_plan::{SelfDirectedStudySession, StudyItemKind};
use crate::services::learning_items;
use chrono::Utc;
use sqlx::SqlitePool;

use super::invalid;
use super::queue::validate_scope;
use super::review::record_event;
use super::state::{local_day, state_for_day};

// Self-directed study shares progress with the daily plan, but not its limits
// or first-open cutoff. Future reviews and items already studied today stay out.
const AVAILABLE: &str = "(? IS NULL OR item.space_id=?)
    AND (item.recall_state='new' OR (item.recall_state='scheduled'
         AND (item.next_review_at IS NULL OR datetime(item.next_review_at)<=datetime(?))))
    AND EXISTS(SELECT 1 FROM question_variants variant WHERE variant.learning_item_id=item.id)
    AND NOT EXISTS(SELECT 1 FROM review_events event
                   WHERE event.local_date=? AND event.learning_item_id=item.id)";

pub async fn session(
    pool: &SqlitePool,
    space_id: Option<i64>,
    kind: StudyItemKind,
    limit: i64,
) -> Result<SelfDirectedStudySession, sqlx::Error> {
    if !(1..=10000).contains(&limit) {
        return Err(invalid("Study batch limit must be between 1 and 10,000."));
    }
    let day = local_day();
    let kind = match kind {
        StudyItemKind::All => "all",
        StudyItemKind::New => "new",
        StudyItemKind::Reviews => "scheduled",
    };
    let mut tx = pool.begin().await?;
    validate_scope(&mut tx, space_id).await?;
    let query = format!(
        "SELECT item.id FROM learning_items item WHERE {AVAILABLE}
        AND (?='all' OR item.recall_state=?)
        ORDER BY item.recall_state='new',
                 COALESCE(datetime(item.next_review_at),datetime('1970-01-01')),item.id LIMIT ?"
    );
    let ids: Vec<i64> = sqlx::query_scalar(&query)
        .bind(space_id)
        .bind(space_id)
        .bind(Utc::now().naive_utc().to_string())
        .bind(&day)
        .bind(kind)
        .bind(kind)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await?;
    let mut items = Vec::with_capacity(ids.len());
    for id in ids {
        items.push(learning_items::load(&mut tx, id).await?);
    }
    tx.commit().await?;
    Ok(SelfDirectedStudySession {
        local_date: day,
        space_id,
        items,
    })
}

pub async fn review(
    pool: &SqlitePool,
    session_date: &str,
    space_id: Option<i64>,
    submission: ReviewSubmission,
) -> Result<LearningItem, sqlx::Error> {
    let day = local_day();
    if session_date != day {
        return Err(invalid(
            "A new study day has started. Return to Recall to start a session.",
        ));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    validate_scope(&mut tx, space_id).await?;
    let item = learning_items::load(&mut tx, submission.learning_item_id).await?;
    if space_id.is_some_and(|id| item.space_id != id) {
        return Err(invalid("This item is not in the selected Recall Space."));
    }
    let already_reviewed: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM review_events WHERE local_date=? AND learning_item_id=?)",
    )
    .bind(&day)
    .bind(item.id)
    .fetch_one(&mut *tx)
    .await?;
    if already_reviewed {
        return Ok(item);
    }
    let query = format!(
        "SELECT EXISTS(SELECT 1 FROM learning_items item
        WHERE item.id=? AND {AVAILABLE})"
    );
    let available: bool = sqlx::query_scalar(&query)
        .bind(item.id)
        .bind(space_id)
        .bind(space_id)
        .bind(Utc::now().naive_utc().to_string())
        .bind(&day)
        .fetch_one(&mut *tx)
        .await?;
    if !available {
        return Err(invalid(
            "This item is not due yet or no longer available. Return to Recall.",
        ));
    }
    state_for_day(&mut tx, &day).await?;
    let result = learning_items::review_in_transaction(&mut tx, submission).await?;
    record_event(
        &mut tx,
        result.id,
        item.recall_state == RecallState::New,
        false,
        &day,
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}
