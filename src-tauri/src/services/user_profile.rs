use crate::models::user_profile::UserProfile;
use sqlx::SqlitePool;

const MAX_DISPLAY_NAME_CHARS: usize = 80;

fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}

fn normalize_display_name(value: &str) -> Result<String, sqlx::Error> {
    let display_name = value.trim();

    if display_name.chars().count() > MAX_DISPLAY_NAME_CHARS {
        return Err(invalid("Your name must be 80 characters or fewer."));
    }

    if display_name.chars().any(char::is_control) {
        return Err(invalid("Your name cannot contain control characters."));
    }

    Ok(display_name.to_owned())
}

pub async fn get(pool: &SqlitePool) -> Result<UserProfile, sqlx::Error> {
    let display_name = sqlx::query_scalar("SELECT display_name FROM user_profile WHERE id = 1")
        .fetch_one(pool)
        .await?;

    Ok(UserProfile { display_name })
}

pub async fn save(pool: &SqlitePool, profile: UserProfile) -> Result<UserProfile, sqlx::Error> {
    let display_name = normalize_display_name(&profile.display_name)?;

    sqlx::query(
        "INSERT INTO user_profile (id, display_name) VALUES (1, ?)
         ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name",
    )
    .bind(&display_name)
    .execute(pool)
    .await?;

    Ok(UserProfile { display_name })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE user_profile (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                display_name TEXT NOT NULL DEFAULT '' CHECK (length(display_name) <= 80)
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO user_profile (id, display_name) VALUES (1, '')")
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn saves_trims_clears_and_round_trips_unicode_names() {
        let pool = pool().await;

        assert_eq!(get(&pool).await.unwrap().display_name, "");
        assert_eq!(
            save(
                &pool,
                UserProfile {
                    display_name: String::from("  Dông-An O’Yu  "),
                },
            )
            .await
            .unwrap()
            .display_name,
            "Dông-An O’Yu"
        );
        assert_eq!(get(&pool).await.unwrap().display_name, "Dông-An O’Yu");
        assert_eq!(
            save(
                &pool,
                UserProfile {
                    display_name: String::from("   "),
                },
            )
            .await
            .unwrap()
            .display_name,
            ""
        );
    }

    #[tokio::test]
    async fn rejects_names_longer_than_eighty_characters() {
        let pool = pool().await;
        let error = save(
            &pool,
            UserProfile {
                display_name: "名".repeat(81),
            },
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("80 characters or fewer"));
        assert_eq!(get(&pool).await.unwrap().display_name, "");
    }
}
