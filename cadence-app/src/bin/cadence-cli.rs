use std::process::ExitCode;

use cadence::db;
use cadence::sync::{SyncOperation, SyncPage, SyncPush, SyncPushResponse};
use clap::{Parser, Subcommand};
use reqwest::Client;
use serde::Deserialize;
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
}

#[derive(Debug, Deserialize)]
struct TaskPayload {
    id: String,
    title: String,
    completed: i64,
    version: i64,
    deleted_at: Option<String>,
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
        let row =
            sqlx::query_as::<_, (i64, bool)>("SELECT version, dirty FROM local_tasks WHERE id = ?")
                .bind(&payload.id)
                .fetch_optional(pool)
                .await?;

        if let Some((version, dirty)) = row
            && dirty
            && payload.version > version
        {
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
            op_id: format!("task-{id}-v{version}"),
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
                && let Some((id, _, _, _)) = rows
                    .iter()
                    .find(|(id, _, _, version)| result.op_id == format!("task-{id}-v{version}"))
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
            eprintln!("conflict on {}: resolve in the web app", result.op_id);
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
