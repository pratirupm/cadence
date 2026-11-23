use std::process::ExitCode;

use cadence::db;
use cadence::sync::{SyncOperation, SyncPage, SyncPush, SyncPushResponse};
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

#[derive(Debug, Parser)]
#[command(name = "cadence-cli", about = "Offline-capable Cadence client")]
struct Cli {
    #[arg(long, env = "CADENCE_SERVER", default_value = "http://127.0.0.1:3000")]
    server: String,
    #[arg(long, env = "CADENCE_TOKEN")]
    token: Option<String>,
    #[arg(long, default_value = "cadence-local.db")]
    local: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List locally cached tasks.
    List,
    /// Pull server changes, then push local edits.
    Sync,
    /// Rename a locally cached task.
    Rename { id: String, title: String },
    /// Mark a locally cached task done.
    Done { id: String },
    /// Reopen a locally cached task.
    Undo { id: String },
    /// List unresolved conflicts.
    Conflicts,
    /// Resolve a conflict by keeping the local or server version.
    Resolve {
        id: i64,
        #[arg(long, value_parser = ["local", "server"])]
        keep: String,
    },
}

#[derive(Debug, Deserialize)]
struct TaskPayload {
    id: String,
    title: String,
    #[serde(deserialize_with = "deserialize_completed")]
    completed: i64,
    version: i64,
    deleted_at: Option<String>,
}

fn deserialize_completed<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Bool(value) => Ok(if value { 1 } else { 0 }),
        Value::Number(value) => value
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("integer")),
        _ => Err(serde::de::Error::custom("boolean or integer")),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let pool = open(&cli.local).await?;

    match cli.command {
        Command::List => list(&pool).await?,
        Command::Sync => {
            let token = cli
                .token
                .as_deref()
                .ok_or("set --token or CADENCE_TOKEN to sync")?;
            sync(&pool, &cli.server, token).await?;
        }
        Command::Rename { id, title } => {
            sqlx::query("UPDATE local_tasks SET title = ?, dirty = 1 WHERE id = ? AND deleted = 0")
                .bind(title)
                .bind(id)
                .execute(&pool)
                .await?;
        }
        Command::Done { id } => {
            set_completed(&pool, &id, true).await?;
        }
        Command::Undo { id } => {
            set_completed(&pool, &id, false).await?;
        }
        Command::Conflicts => list_conflicts(&pool).await?,
        Command::Resolve { id, keep } => resolve_conflict(&pool, id, &keep).await?,
    }

    Ok(())
}

async fn open(path: &str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let pool = db::connect(path).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_tasks (
            id TEXT PRIMARY KEY NOT NULL,
            title TEXT NOT NULL,
            completed INTEGER NOT NULL DEFAULT 0,
            version INTEGER NOT NULL DEFAULT 1,
            dirty INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_meta (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_conflicts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id TEXT NOT NULL,
            local_title TEXT NOT NULL,
            local_completed INTEGER NOT NULL,
            server_title TEXT NOT NULL,
            server_completed INTEGER NOT NULL,
            server_version INTEGER NOT NULL,
            resolved INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&pool)
    .await?;

    Ok(pool)
}

async fn list(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, (String, String, bool, i64, bool)>(
        "SELECT id, title, completed, version, dirty FROM local_tasks
         WHERE deleted = 0 ORDER BY completed, title",
    )
    .fetch_all(pool)
    .await?;

    for (id, title, completed, version, dirty) in rows {
        let mark = if completed { "x" } else { " " };
        let dirty = if dirty { " *" } else { "" };
        println!("{mark} {id}  {title}  (v{version}){dirty}");
    }

    Ok(())
}

async fn set_completed(
    pool: &SqlitePool,
    id: &str,
    completed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query("UPDATE local_tasks SET completed = ?, dirty = 1 WHERE id = ? AND deleted = 0")
        .bind(completed)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn sync(
    pool: &SqlitePool,
    server: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new();
    let since = last_seq(pool).await?;
    let page: SyncPage = client
        .get(format!("{server}/api/v1/sync"))
        .bearer_auth(token)
        .query(&[("since", since.to_string()), ("limit", "100".to_owned())])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    for change in &page.changes {
        let payload: TaskPayload = serde_json::from_str(&change.payload)?;
        let row = sqlx::query_as::<_, (i64, bool, String, bool)>(
            "SELECT version, dirty, title, completed FROM local_tasks WHERE id = ?",
        )
        .bind(&payload.id)
        .fetch_optional(pool)
        .await?;

        if let Some((version, dirty, title, completed)) = row
            && dirty
            && payload.version >= version
        {
            record_conflict(pool, &payload, &title, completed).await?;
            eprintln!(
                "conflict: local edits to {} are older than server v{}",
                payload.id, payload.version
            );
            continue;
        }

        sqlx::query(
            "INSERT INTO local_tasks (id, title, completed, version, dirty, deleted)
             VALUES (?, ?, ?, ?, 0, ?)
             ON CONFLICT (id) DO UPDATE SET
                 title = excluded.title,
                 completed = excluded.completed,
                 version = excluded.version,
                 dirty = 0,
                 deleted = excluded.deleted",
        )
        .bind(&payload.id)
        .bind(&payload.title)
        .bind(payload.completed != 0)
        .bind(payload.version)
        .bind(payload.deleted_at.is_some())
        .execute(pool)
        .await?;
    }

    set_last_seq(pool, page.cursor).await?;
    println!(
        "pulled {} change(s) to cursor {}",
        page.changes.len(),
        page.cursor
    );

    push(pool, &client, server, token).await
}

async fn push(
    pool: &SqlitePool,
    client: &Client,
    server: &str,
    token: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, (String, String, bool, i64)>(
        "SELECT id, title, completed, version FROM local_tasks
         WHERE dirty = 1 AND deleted = 0",
    )
    .fetch_all(pool)
    .await?;

    if rows.is_empty() {
        return Ok(());
    }

    let operations = rows
        .iter()
        .map(|(id, title, completed, version)| SyncOperation {
            op_id: op_id(id, *version, title, *completed),
            entity: "task".to_owned(),
            entity_id: id.clone(),
            op: "update".to_owned(),
            base_version: *version,
            title: Some(title.clone()),
            completed: Some(*completed),
        })
        .collect();

    let response: SyncPushResponse = client
        .post(format!("{server}/api/v1/sync"))
        .bearer_auth(token)
        .json(&SyncPush { operations })
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    for result in response.results {
        if result.status == "ok" {
            if let Some(version) = result.version
                && let Some((id, _, _, _)) = rows.iter().find(|(id, title, completed, version)| {
                    result.op_id == op_id(id, *version, title, *completed)
                })
            {
                sqlx::query(
                    "UPDATE local_tasks SET version = ?, dirty = 0
                     WHERE id = ? AND version < ?",
                )
                .bind(version)
                .bind(id)
                .bind(version)
                .execute(pool)
                .await?;
            }
            println!("pushed {}", result.op_id);
        } else if result.status == "conflict" {
            if let Some((_, title, completed, _)) =
                rows.iter().find(|(id, title, completed, version)| {
                    result.op_id == op_id(id, *version, title, *completed)
                })
                && let Some(value) = &result.conflict
            {
                let payload: TaskPayload = serde_json::from_value(value.clone())?;
                record_conflict(pool, &payload, title, *completed).await?;
            }
            eprintln!("conflict on {}: run `conflicts` to resolve", result.op_id);
        } else {
            eprintln!(
                "error on {}: {}",
                result.op_id,
                result.error.unwrap_or_default()
            );
        }
    }

    Ok(())
}

async fn record_conflict(
    pool: &SqlitePool,
    server: &TaskPayload,
    local_title: &str,
    local_completed: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO local_conflicts
         (task_id, local_title, local_completed, server_title, server_completed,
          server_version, resolved)
         VALUES (?, ?, ?, ?, ?, ?, 0)",
    )
    .bind(&server.id)
    .bind(local_title)
    .bind(local_completed)
    .bind(&server.title)
    .bind(server.completed != 0)
    .bind(server.version)
    .execute(pool)
    .await?;

    Ok(())
}

async fn list_conflicts(pool: &SqlitePool) -> Result<(), Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, (i64, String, String, String, i64)>(
        "SELECT id, task_id, local_title, server_title, server_version
         FROM local_conflicts WHERE resolved = 0 ORDER BY id",
    )
    .fetch_all(pool)
    .await?;

    for (id, task_id, local, server, version) in rows {
        println!("{id}  {task_id}  local: {local}  server(v{version}): {server}");
    }

    Ok(())
}

async fn resolve_conflict(
    pool: &SqlitePool,
    id: i64,
    keep: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, (String, String, bool, String, bool, i64)>(
        "SELECT task_id, local_title, local_completed, server_title, server_completed,
                server_version
         FROM local_conflicts WHERE id = ? AND resolved = 0",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or("conflict not found")?;

    let (task_id, local_title, local_completed, server_title, server_completed, server_version) =
        row;

    if keep == "local" {
        sqlx::query(
            "UPDATE local_tasks
             SET title = ?, completed = ?, version = ?, dirty = 1
             WHERE id = ?",
        )
        .bind(local_title)
        .bind(local_completed)
        .bind(server_version)
        .bind(&task_id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "UPDATE local_tasks
             SET title = ?, completed = ?, version = ?, dirty = 0
             WHERE id = ?",
        )
        .bind(server_title)
        .bind(server_completed)
        .bind(server_version)
        .bind(&task_id)
        .execute(pool)
        .await?;
    }

    sqlx::query("UPDATE local_conflicts SET resolved = 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;

    Ok(())
}

fn op_id(id: &str, version: i64, title: &str, completed: bool) -> String {
    let digest = Sha256::digest(format!("{id}:{version}:{title}:{completed}"));
    format!("task-{id}-v{version}-{}", hex::encode(&digest[..8]))
}

async fn last_seq(pool: &SqlitePool) -> Result<i64, Box<dyn std::error::Error>> {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM local_meta WHERE key = 'seq'")
        .fetch_optional(pool)
        .await?;

    Ok(value.and_then(|value| value.parse().ok()).unwrap_or(0))
}

async fn set_last_seq(pool: &SqlitePool, seq: i64) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
        "INSERT INTO local_meta (key, value) VALUES ('seq', ?)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(seq.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_deterministic_and_content_sensitive() {
        let first = op_id("task-1", 2, "buy milk", false);
        let again = op_id("task-1", 2, "buy milk", false);
        let changed = op_id("task-1", 2, "buy oat milk", false);

        assert_eq!(first, again);
        assert_ne!(first, changed);
        assert!(first.starts_with("task-task-1-v2-"));
    }

    #[test]
    fn payload_accepts_boolean_and_integer_completion() {
        let as_integer: TaskPayload = serde_json::from_str(
            r#"{"id":"1","title":"x","completed":1,"version":1,"deleted_at":null}"#,
        )
        .unwrap();
        let as_boolean: TaskPayload = serde_json::from_str(
            r#"{"id":"1","title":"x","completed":true,"version":1,"deleted_at":null}"#,
        )
        .unwrap();

        assert_eq!(as_integer.completed, 1);
        assert_eq!(as_boolean.completed, 1);
    }
}
