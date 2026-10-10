use chrono::Local;
use sqlx::SqliteConnection;

use super::preferences::read_preferences;

#[derive(Debug)]
pub(super) struct DailyState {
    pub(super) local_date: String,
    pub(super) started_at: String,
    pub(super) daily_target: i64,
    pub(super) max_new_items: i64,
}

pub fn local_day() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub(super) async fn state_for_day(
    conn: &mut SqliteConnection,
    day: &str,
) -> Result<DailyState, sqlx::Error> {
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

pub(super) async fn daily_counts(
    conn: &mut SqliteConnection,
    day: &str,
) -> Result<(i64, i64, i64), sqlx::Error> {
    sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(is_extra=1),0),
                COALESCE(SUM(was_new=1),0)
         FROM review_events WHERE local_date=?",
    )
    .bind(day)
    .fetch_one(conn)
    .await
}
