use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

/// Open the SQLite database, creating the file if needed.
pub async fn connect(database: &str) -> Result<SqlitePool, sqlx::Error> {
    let url = database_url(database);
    let options = SqliteConnectOptions::from_str(&url)?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let max_connections = if url.contains(":memory:") { 1 } else { 5 };

    SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
}

/// Apply every migration in order.
pub async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

fn database_url(database: &str) -> String {
    if database.starts_with("sqlite:") {
        database.to_owned()
    } else {
        format!("sqlite://{database}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_sqlite_scheme_to_a_path() {
        assert_eq!(database_url("cadence.db"), "sqlite://cadence.db");
    }

    #[test]
    fn leaves_a_sqlite_url_alone() {
        assert_eq!(database_url("sqlite::memory:"), "sqlite::memory:");
    }
}
