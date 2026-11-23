# 2. SQLite and migrations

**What you can do at the end.** Open a database, apply migrations, and keep
tasks across restarts.

**The Rust you learn.** `sqlx`, async database calls, `FromRow`, transactions,
ULIDs, and the difference between a row and a domain type.

**Build it.**

- Add `sqlx` with `runtime-tokio`, `sqlite`, `migrate`, `chrono`, and `macros`.
- Add `chrono` and `ulid`.
- Write `migrations/0001_tasks.sql` with `id`, `title`, `completed`,
  `created_at`, and `updated_at`.
- In `db.rs`, parse the SQLite URL, enable WAL, foreign keys, and a busy
  timeout, then run `sqlx::migrate!`.
- In `tasks.rs`, define `Task`, `TaskError`, and `TaskRepository` with `create`,
  `get`, `list`, `set_completed`, and `delete`.
- Add a `migrate` subcommand and connect the server to the same pool.

A repository method returns a domain value, not a raw row:

```rust
pub async fn create(&self, title: &str) -> Result<Task, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(TaskError::EmptyTitle);
    }
    // insert, then return the Task
}
```

**Checkpoint.**

```console
$ cargo run -- migrate
$ cargo test tasks
$ cargo run -- serve
```

The tests use an in-memory SQLite pool with one connection, apply the
migrations, and assert that titles are trimmed, empty titles are rejected,
tasks are listed newest first, and missing ids return `None` or `false`.

**Exercises.**

1. Add a `notes` column through a new migration, then update the repository.
2. Add an index for `completed` and explain which query it helps.
