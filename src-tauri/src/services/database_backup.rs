//! Consistent SQLite backup and restore support, independent of schema migrations.
use sqlx::{Connection, Row, SqliteConnection};
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
fn invalid(message: &str) -> sqlx::Error {
    sqlx::Error::Protocol(message.into())
}
pub(super) async fn check_database(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let checks: Vec<String> = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_all(&mut *connection)
        .await?;
    if checks != ["ok"] {
        return Err(invalid("Database integrity check failed"));
    }
    if !sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await?
        .is_empty()
    {
        return Err(invalid("Database contains broken foreign keys"));
    }
    Ok(())
}

pub async fn create(connection: &mut SqliteConnection) -> Result<PathBuf, sqlx::Error> {
    let databases = sqlx::query("PRAGMA database_list")
        .fetch_all(&mut *connection)
        .await?;
    let file: String = databases
        .iter()
        .find(|row| row.get::<String, _>("name") == "main")
        .ok_or_else(|| invalid("Main database is unavailable"))?
        .get("file");
    if file.is_empty() {
        return Err(invalid(
            "Migration backup requires a file-backed database for recovery",
        ));
    }
    let destination = PathBuf::from(format!(
        "{file}.pre-migration-{}.sqlite",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    // SQLite refuses an existing nonempty destination; explicitly reject any existing path.
    if destination.exists() {
        return Err(invalid("Backup destination already exists"));
    }
    sqlx::query("VACUUM main INTO ?")
        .bind(destination.to_string_lossy().as_ref())
        .execute(&mut *connection)
        .await?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&destination)
        .read_only(true);
    let mut backup = SqliteConnection::connect_with(&options).await?;
    check_database(&mut backup).await?;
    backup.close().await?;
    Ok(destination)
}

/// Recovery creates a separate verified file; never overwrites the original automatically.
/// Close the app, then select this restored copy using the compatible app version.
#[cfg(test)]
pub async fn restore_copy(backup: &Path, destination: &Path) -> Result<(), sqlx::Error> {
    if destination.exists() {
        return Err(invalid("Recovery destination must not exist"));
    }
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(backup)
        .read_only(true);
    let mut connection = SqliteConnection::connect_with(&options).await?;
    check_database(&mut connection).await?;
    sqlx::query("VACUUM main INTO ?")
        .bind(destination.to_string_lossy().as_ref())
        .execute(&mut connection)
        .await?;
    connection.close().await?;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(destination)
        .read_only(true);
    let mut restored = SqliteConnection::connect_with(&options).await?;
    check_database(&mut restored).await?;
    restored.close().await
}
