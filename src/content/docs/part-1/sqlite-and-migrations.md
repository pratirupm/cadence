---
title: "SQLite and migrations"
description: "Open a SQLite database, apply migrations, and build the first task repository."
---

The health endpoint proved that a request can cross the process boundary and
come back as a response. It did not prove that anything survives a restart. A
task service that forgets every task when the process exits is a demo, not a
service.

This chapter adds the smallest persistence layer that can be trusted: one SQLite
file, one migration, one table, and one repository. The repository is the only
part of the service that knows SQL. Handlers will ask it for tasks; tests will
ask it for tasks; nothing else will care that the bytes live in a file.

By the end of this chapter you can:

- create a database file and apply a migration
- insert a task, read it back, list tasks newest first
- mark a task done and reopen it
- delete a task
- reject an empty title before it reaches the database
- run the whole repository suite against an in-memory database

The Rust lessons are `sqlx`, async database calls, `FromRow`, transaction-free
single-row operations, ULIDs, and converting between a database row and a domain
type. The SQL lessons are smaller but real: constraints, indexes, and the
difference between a schema and a migration.

## Why SQLite

The first book stored tasks in `.todo.json`. That was the right choice for a
single-user CLI: no server, no setup, easy to inspect. A multi-user service has
different requirements:

- several requests can touch the data at once
- a write must not leave a half-written file
- queries need indexes and transactions
- tests need isolated databases

SQLite gives all four without asking the reader to install a database server.
It is a library, not a daemon. The database is a file. A test can use an
in-memory database; a deployment can put the file on a volume.

The trade-off is honest: SQLite has one writer at a time. For this book’s scale
that is fine, and the busy timeout, WAL mode, and short transactions in this
chapter are the tools that make it predictable. A service that outgrows one
writer needs a different database; that is a deployment decision, not a reason
to hide the SQL behind an ORM on day one.

## The schema

Create `migrations/0001_tasks.sql`:

```sql
CREATE TABLE tasks (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0 CHECK (completed IN (0, 1)),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX tasks_created_at_idx ON tasks (created_at DESC, id DESC);
```

Four decisions are embedded in those lines.

`id` is `TEXT`, not `INTEGER`. The IDs are ULIDs: 26-character, lexicographically
sortable identifiers. They are generated in Rust, not by the database, so a
client can create a task before it has a round trip. The sortable prefix also
makes “newest first” stable even when two rows share a timestamp.

`completed` is an integer with a `CHECK`. SQLite has no native boolean. The
check constraint is the schema’s promise that the column is only `0` or `1`.
Rust will still map it to `bool`; the constraint protects against a bug in
another language or a manual edit.

Timestamps are `TEXT` in RFC 3339 form. SQLite has no date type. Storing UTC
text keeps the values human-readable and sortable. The time zone is explicit in
the string, so a later chapter can convert to a user’s zone without guessing.

The index is for the list query. `ORDER BY created_at DESC, id DESC` can use it
directly. Without the index, SQLite would sort every row on every list request.
The second column, `id`, is the tie-breaker for equal timestamps.

Migrations are append-only. Do not edit `0001` after it has run on a database;
add `0002`, `0003`, and so on. A migration is a promise to every database that
has already applied it. The chapter’s schema will grow; the old files stay.

## Dependencies

Add the database dependencies:

```toml
sqlx = { version = "0.8", features = [
    "runtime-tokio", "sqlite", "migrate", "chrono", "macros"
] }
chrono = { version = "0.4", features = ["serde"] }
ulid = { version = "1", features = ["serde"] }
```

`runtime-tokio` tells SQLx which async runtime it may use. `sqlite` selects the
driver. `migrate` enables the `migrate!` macro and the migration table.
`chrono` enables `DateTime<Utc>` encoding. `macros` enables `FromRow` and
`query_as` derives.

`chrono` stores timestamps; `ulid` generates IDs. Neither is a framework. They
are small types with clear jobs.

## Connect and migrate

Create `src/db.rs`:

```rust
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
```

`create_if_missing` is convenient for development and for the first deployment.
It is not a backup policy. A production database file should be created by a
migration step, not by a typo in a path.

WAL mode lets readers continue while one writer appends. `foreign_keys` is off
by default in SQLite; turn it on explicitly so the later workspace tables can
enforce their references. The busy timeout makes a concurrent write wait for a
short time instead of failing immediately with `database is locked`.

The connection limit is one for `:memory:` because each connection to an
in-memory SQLite database gets its own private database. A pool with two
connections would give two different worlds. For a file, five connections is a
reasonable starting point; SQLite serializes writes anyway.

`sqlx::migrate!` embeds the migration files at compile time. The running binary
does not need a `migrations/` folder at deployment, but the binary does need to
be rebuilt when a migration changes. The migrations table records which files
have run and their checksums. Do not edit an applied file; SQLx will refuse.

## The domain type

Create `src/tasks.rs` and start with the type, not the SQL:

```rust
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

This is the shape the rest of the service wants. It is not the same as a row:
the table has an integer `completed`, the struct has a `bool`; the table has
text timestamps, the struct has `DateTime<Utc>`. `FromRow` performs the
conversion for `query_as`.

`DateTime<Utc>` is a type, not a string. A string can be “yesterday” or
“2026-13-01”; a date type cannot. The database may store text, but the moment it
enters Rust, it becomes a value with invariants. That is the point of a domain
type.

`PartialEq` and `Eq` make tests readable: a created task and a loaded task can be
compared directly. `Clone` lets a handler return a copy while keeping the
original for an event. `Debug` makes a failed assertion printable.

## The error type

The repository has two kinds of failure: invalid input and database failure.
Keep them separate from the HTTP error type:

```rust
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task title cannot be empty")]
    EmptyTitle,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
```

`#[from]` gives you `?` conversion. A `sqlx::Error` becomes
`TaskError::Database` without a `map_err` at every call site. `#[error(transparent)]`
forwards the display of the inner error, which is useful in logs. The HTTP layer
will later decide that `EmptyTitle` is a 400 and `Database` is a 500. The
repository does not know about status codes.

## The repository

```rust
#[derive(Debug, Clone)]
pub struct TaskRepository {
    pool: SqlitePool,
}

impl TaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, title: &str) -> Result<Task, TaskError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }

        let now = Utc::now();
        let task = Task {
            id: Ulid::new().to_string(),
            title: title.to_owned(),
            completed: false,
            created_at: now,
            updated_at: now,
        };

        sqlx::query(
            "INSERT INTO tasks (id, title, completed, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&task.id)
        .bind(&task.title)
        .bind(task.completed)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(task)
    }
}
```

The repository owns the pool. It does not own the server, the request, or the
configuration. That makes it usable from tests, handlers, workers, and later the
CLI sync client.

`create` trims before it validates. A title made of spaces is not a title. The
check happens before the ID is generated, so a rejected task does not consume an
ID or touch the database.

SQLx binds values. The `?` placeholders are not string interpolation. A title
containing `'; DROP TABLE tasks; --` is stored as text, not executed. This is the
default; do not build SQL by formatting values into the string.

The returned `Task` is the value that was inserted. That avoids a second read
and keeps the timestamp consistent.

Add the read methods:

```rust
pub async fn get(&self, id: &str) -> Result<Option<Task>, TaskError> {
    let task = sqlx::query_as::<_, Task>(
        "SELECT id, title, completed, created_at, updated_at FROM tasks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&self.pool)
    .await?;

    Ok(task)
}

pub async fn list(&self) -> Result<Vec<Task>, TaskError> {
    let tasks = sqlx::query_as::<_, Task>(
        "SELECT id, title, completed, created_at, updated_at
         FROM tasks
         ORDER BY created_at DESC, id DESC",
    )
    .fetch_all(&self.pool)
    .await?;

    Ok(tasks)
}
```

`fetch_optional` returns `None` for a missing row; `fetch_all` returns an empty
vector for no rows. Neither is an error. “Not found” is a normal outcome for a
lookup, and the caller decides whether it is exceptional. The repository does
not turn it into a 404; that is a web concern.

The list query is deliberately explicit about columns. `SELECT *` is shorter,
but it couples the struct to the column order and quietly breaks when a future
migration adds a column. Name what you need.

Add the mutations:

```rust
pub async fn set_completed(&self, id: &str, completed: bool) -> Result<bool, TaskError> {
    let result = sqlx::query("UPDATE tasks SET completed = ?, updated_at = ? WHERE id = ?")
        .bind(completed)
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;

    Ok(result.rows_affected() == 1)
}

pub async fn delete(&self, id: &str) -> Result<bool, TaskError> {
    let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(&self.pool)
        .await?;

    Ok(result.rows_affected() == 1)
}
```

`rows_affected` is the honest result. A missing ID is not a database error; it
is a `false`. The HTTP layer can map that to 404. This is the same distinction
as `Option` versus `Result`: absence is not failure.

## Tests

The repository tests need a database, but not a file. Use an in-memory database
with one connection, apply migrations, and build the repository:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn repository() -> TaskRepository {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        TaskRepository::new(pool)
    }

    #[tokio::test]
    async fn creates_and_reads_a_task() {
        let repository = repository().await;

        let created = repository.create("buy milk").await.unwrap();
        let loaded = repository.get(&created.id).await.unwrap().unwrap();

        assert_eq!(loaded, created);
        assert_eq!(loaded.title, "buy milk");
        assert!(!loaded.completed);
    }
}
```

The fixture is one function. Every test gets a fresh database; no test depends
on another. This is the same isolation principle as book one’s temporary files,
with a different storage engine.

Add tests for the behaviours that can regress:

- `trims_the_title`: `"  buy milk  "` becomes `"buy milk"`.
- `rejects_an_empty_title`: `"   "` returns `EmptyTitle` and leaves the table
  empty.
- `lists_newest_first`: two creates, then assert `["second", "first"]`.
- `completes_and_reopens_a_task`: completion is a toggle, not a one-way door.
- `reports_a_missing_task`: `get` returns `None`, `set_completed` and `delete`
  return `false`.
- `deletes_a_task`: after delete, `get` returns `None`.

The list order relies on ULIDs being monotonic enough for two creates in the same
test. The `ORDER BY created_at DESC, id DESC` is explicit; if the timestamps are
equal, the ID decides. If you change the ID format, this test should make you
think about the ordering contract.

## Wiring the pool into the server

The router needs the pool. Introduce `AppState` in `lib.rs`:

```rust
use sqlx::SqlitePool;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}
```

`SqlitePool` is already a cheap, cloneable handle. `AppState` is a struct rather
than a bare pool so later chapters can add services without changing every
handler signature. `#[derive(Clone)]` is required by axum’s state extraction.

The health handler does not need the state. `.with_state` still makes the router
type `Router<()>`, which is what `axum::serve` expects. Handlers that need state
will take `State<AppState>` and get a clone of the state.

In `main`, connect and migrate before serving:

```rust
let pool = db::connect(&database).await?;
db::migrate(&pool).await?;
axum::serve(listener, app(AppState { pool }))
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

The `serve` command now accepts `--database`; the `migrate` command applies the
migrations without starting a server:

```console
$ cargo run -- migrate
$ cargo run -- serve
```

At this checkpoint the health test builds an `AppState` with an in-memory pool.
It does not query the database, but the router type needs a state value.

## Checkpoint

```console
$ cargo run -- migrate
$ cargo test tasks
$ cargo test
$ cargo run -- serve
$ curl http://127.0.0.1:3000/health
```

The task tests prove the persistence contract. The health test still proves the
router. The manual run proves that the binary opens the database and applies the
migration before it accepts requests.

## Failure modes you will meet

**`migration ... was previously applied but has been modified`.** You edited a
migration that has already run. Restore the file and add a new migration. The
checksum is the database’s memory of the schema.

**`database is locked`.** Another connection held a write longer than the busy
timeout. Check for a long transaction or a second process using the same file.
For tests, use one in-memory connection; for production, keep write transactions
short.

**`no such table: tasks`.** The migration did not run, or the server connected to
a different file. Run `migrate` with the same `--database` value, and check the
path. A relative path is relative to the process working directory.

**A timestamp has the wrong time zone.** The database stores UTC. Convert to a
user’s zone at the edge, not in the repository. A `DateTime<Utc>` that becomes a
local string too early cannot be converted back reliably.

**`completed` decodes as an integer error.** SQLite stores `0`/`1`; the struct
wants `bool`. SQLx handles the standard mapping. If you change the column to
text, the mapping breaks. Keep the schema and the type aligned.

**An empty title gets inserted.** The validation must run before the insert, and
the database should have a `NOT NULL` constraint as a second line of defense.
Do not rely on the client form’s `required` attribute.

## Under the hood

`sqlx` is not an ORM. It executes SQL and maps rows. `query_as` is a runtime
query; `query!` can check SQL against a live database at compile time, but that
requires a database during the build and an offline metadata file for CI. This
book uses runtime queries to keep the first persistence chapter approachable.

SQLite stores the database in one file plus `-wal` and `-shm` files while WAL is
active. A backup must copy a consistent snapshot, not just the main file while a
write is in progress. Chapter 23 uses `VACUUM INTO` for that reason.

ULIDs are 128-bit identifiers encoded in Crockford base32. The first 48 bits are
a millisecond timestamp, so they sort roughly by creation time. They are not a
security boundary; they are a stable, sortable key.

## Exercises

1. **Add `notes`.** Write `migrations/0002_notes.sql` with
   `ALTER TABLE tasks ADD COLUMN notes TEXT NOT NULL DEFAULT ''`. Add the field
   to `Task`, update every query, and add a test that notes default to empty.
2. **Add a completed filter.** Write `list_completed(&self, completed: bool)`.
   Add a test for both values. Which index would help, and why is the existing
   `created_at` index not enough?
3. **Make `create` transactional.** The current insert is one statement, so it is
   already atomic. Add a second table for labels and insert both in one
   transaction. Prove with a failing insert that neither row remains.
4. **Change the missing-task contract.** Make `get` return `TaskError::NotFound`
   instead of `Option<Task>`. Update the tests. Which callers become simpler, and
   which become noisier?
5. **Test a restart.** Create a file-backed database, insert a task, drop the
   pool, reopen it, and assert the task is still there. Use `tempfile` so the
   test cleans up.

## What comes next

The repository can store tasks, but there is no user and no place to put a task.
Chapter 3 adds accounts, password hashing, sessions, and the first real reason
for an error variant beyond “not found.” The task table will wait; the next
boundary is identity.
