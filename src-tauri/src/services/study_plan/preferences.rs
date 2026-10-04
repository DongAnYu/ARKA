use crate::models::study_plan::StudyPreferences;
use sqlx::{SqliteConnection, SqlitePool};

use super::invalid;
use super::state::local_day;

pub async fn preferences(pool: &SqlitePool) -> Result<StudyPreferences, sqlx::Error> {
    read_preferences(&mut *pool.acquire().await?).await
}

pub(super) async fn read_preferences(
    conn: &mut SqliteConnection,
) -> Result<StudyPreferences, sqlx::Error> {
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
