use crate::models::learning_item::{LearningItem, ReviewRating, ReviewResponse, ReviewSubmission};
use crate::models::study_plan::{DailyStudyPlan, StudyPreferences};
use crate::services::study_plan::{get_plan, review, save_preferences};
use sqlx::{sqlite::SqlitePoolOptions, SqlitePool};

async fn fixture(max_new_items: i64) -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    super::run_migrations(&mut *pool.acquire().await.unwrap())
        .await
        .unwrap();
    save_preferences(
        &pool,
        StudyPreferences {
            daily_target: 2,
            max_new_items,
        },
        false,
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO learning_items(id,target,answer,space_id,recall_state) VALUES(1,'Target','Answer',1,'new'),(2,'Target','Answer',1,'new')")
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO question_variants(id,learning_item_id,format,prompt,content_json) VALUES(1,1,'flashcard','Prompt','{}'),(2,2,'flashcard','Prompt','{}')")
        .execute(&pool).await.unwrap();
    pool
}

fn ids(plan: &DailyStudyPlan) -> Vec<i64> {
    plan.items.iter().map(|planned| planned.item.id).collect()
}

async fn submit(
    pool: &SqlitePool,
    plan: &DailyStudyPlan,
    id: i64,
) -> Result<LearningItem, sqlx::Error> {
    review(
        pool,
        &plan.local_date,
        plan.space_id,
        false,
        ReviewSubmission {
            learning_item_id: id,
            variant_id: id,
            response: ReviewResponse::Flashcard {
                rating: ReviewRating::Good,
            },
        },
    )
    .await
}

#[tokio::test]
async fn deleting_unanswered_item_removes_variants_without_counting_a_review() {
    let pool = fixture(2).await;
    let plan = get_plan(&pool, Some(1), false).await.unwrap();
    assert_eq!(ids(&plan), [1, 2]);

    sqlx::query("DELETE FROM learning_items WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let variants: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM question_variants WHERE learning_item_id=1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(variants, 0);
    let refreshed = get_plan(&pool, Some(1), false).await.unwrap();
    assert_eq!(refreshed.completed_count, 0);
    assert_eq!(ids(&refreshed), [2]);
    submit(&pool, &plan, 2).await.unwrap();
    let completed = get_plan(&pool, None, false).await.unwrap();
    assert_eq!(completed.completed_count, 1);
    assert!(completed.items.is_empty());
}

#[tokio::test]
async fn deleting_reviewed_item_preserves_progress_and_new_item_quota() {
    let pool = fixture(1).await;
    let plan = get_plan(&pool, None, false).await.unwrap();
    submit(&pool, &plan, 1).await.unwrap();
    sqlx::query("DELETE FROM learning_items WHERE id=1")
        .execute(&pool)
        .await
        .unwrap();
    let event: (Option<i64>, bool, bool) =
        sqlx::query_as("SELECT learning_item_id,was_new,is_extra FROM review_events")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(event, (None, true, false));
    for scope in [None, Some(1)] {
        let refreshed = get_plan(&pool, scope, false).await.unwrap();
        assert_eq!(refreshed.completed_count, 1);
        assert!(refreshed.items.is_empty());
    }
}
