use sqlx::Row;
use sqlx::sqlite::SqlitePoolOptions;

use crate::models::question::Question;

fn resolve_database_url() -> String {
    let raw = std::env::var("DATABASE_URL").unwrap_or_else(|_| String::from("sqlite://review.db"));

    if raw.starts_with("sqlite:") && !raw.starts_with("sqlite://") {
        return raw.replacen("sqlite:", "sqlite://", 1);
    }

    raw
}

async fn insert_test_question(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO questions (id, question, option_a, option_b, option_c, option_d, correct_answer, explanation) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(1_i64)
    .bind("What is Rust?")
    .bind("Language")
    .bind("Database")
    .bind("Browser")
    .bind("OS")
    .bind("A")
    .bind("A systems programming language.")
    .execute(pool)
    .await?;

    Ok(())
}

async fn get_all_question(pool: &sqlx::SqlitePool) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("SELECT id, question FROM questions")
        .fetch_all(pool)
        .await?;

    println!("Loaded {} question", rows.len());

    for row in rows {
        let question: String = row.get("question");
        println!("Question: {}", question);
    }

    Ok(())
}

pub async fn get_questions() -> Result<Vec<Question>, sqlx::Error> {
    let database_url = resolve_database_url();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;

    let questions = sqlx::query_as::<_, Question>(
        "SELECT id, question, option_a, option_b, option_c, option_d, correct_answer, explanation FROM questions ORDER BY id",
    )
    .fetch_all(&pool)
    .await?;

    Ok(questions)
}

pub async fn run_smoke_test() -> Result<(), sqlx::Error> {
    let database_url = resolve_database_url();
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;
    insert_test_question(&pool).await?;
    get_all_question(&pool).await?;

    Ok(())
}
