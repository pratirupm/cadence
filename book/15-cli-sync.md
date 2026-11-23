# 15. The CLI sync client

The server has a change log and a push endpoint. The browser has forms. Neither
is useful on a train with no connection. This chapter builds a second client: a
CLI with its own SQLite cache, a cursor, a dirty flag, and a sync command.

The CLI is not a thin `curl` wrapper. It applies server changes locally, edits
the local copy, and pushes those edits back with a base version. That is enough
to demonstrate the sync contract without building a browser offline engine.

By the end of this chapter you can:

- open a local cache and list tasks without a server
- pull changes after a stored cursor
- apply a task payload that may encode `completed` as `0` or `true`
- rename, complete, and reopen a local task and mark it dirty
- push all dirty tasks in one batch
- build a deterministic operation ID from the content
- resolve a conflict by keeping the local or server version
- explain the limits: no offline create, no offline delete, one page per sync

The Rust lessons are a second binary, two SQLite databases in one program,
`reqwest`, cursor storage, and a content-addressed operation ID. The product
lesson is that offline editing is a local state machine, not a cache with a
refresh button.

## The command surface

The binary lives at `src/bin/cadence-cli.rs`:

```rust
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
```

The commands are:

```text
list          show the local cache
sync          pull changes, then push dirty edits
rename <id> <title>
done <id>
undo <id>
conflicts     list unresolved conflicts
resolve <id> --keep local|server
```

`list`, `rename`, `done`, `undo`, `conflicts`, and `resolve` work without a
server. Only `sync` needs `--token` or `CADENCE_TOKEN`. That split is the point:
the cache is useful offline.

The token is a bearer credential. Passing it on the command line puts it in
shell history; the environment variable is safer. A production client stores it
in an OS keyring. The book keeps the flag for a reproducible demo and names the
risk.

`--local` chooses a different cache. Two caches are independent: they have their
own cursors, dirty rows, and conflict inbox. This is useful for testing the
same server from two “machines.”

## The local schema

`open` creates three tables:

```sql
CREATE TABLE IF NOT EXISTS local_tasks (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    dirty INTEGER NOT NULL DEFAULT 0,
    deleted INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS local_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS local_conflicts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    local_title TEXT NOT NULL,
    local_completed INTEGER NOT NULL,
    server_title TEXT NOT NULL,
    server_completed INTEGER NOT NULL,
    server_version INTEGER NOT NULL,
    resolved INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

`local_tasks` is a subset of the server task: ID, title, completion, version,
dirty, deleted. The CLI does not need notes, priority, recurrence, or assignment
for this chapter. A full client would store them; the sync payload does not carry
them yet (chapter 14’s gap).

`local_meta` stores the cursor under the key `seq`. It could be a column on a
singleton row, but a key/value table is easy to extend for `server_url` or a
schema version later.

`local_conflicts` stores both sides of a conflict as values, not references, so a
later server change does not erase what the user was looking at. `resolved` is
the inbox state; the rows are kept for history.

The local database uses the same `db::connect` helper as the server. That helper
enables WAL, foreign keys, and a busy timeout. WAL is useful even for one process
because a read while a sync transaction is writing should not block.

## Pull

`sync` reads the cursor:

```rust
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
```

The HTTP client is `reqwest`. `bearer_auth` sets the header. `error_for_status`
turns a 401/403/500 into an error before deserialization, so a JSON error page
does not look like a successful empty page.

The request asks for 100 changes. That is one page. The CLI does not loop until
empty in this chapter; a large backlog needs two `sync` runs. The exercise asks
you to loop. The cursor makes the second run cheap.

Each change is applied in `seq` order:

```rust
for change in &page.changes {
    let payload: TaskPayload = serde_json::from_str(&change.payload)?;
    // conflict check and local upsert
}
```

The payload type is tolerant about completion:

```rust
struct TaskPayload {
    id: String,
    title: String,
    #[serde(deserialize_with = "deserialize_completed")]
    completed: i64,
    version: i64,
    deleted_at: Option<String>,
}
```

The trigger emits `completed` as `0`/`1`; the API’s task JSON emits `true`/`false`.
The custom deserializer accepts both. A client that assumes one shape breaks when
it switches from the log to the API. Tolerance at the boundary is cheap.

The local upsert:

```sql
INSERT INTO local_tasks (id, title, completed, version, dirty, deleted)
VALUES (?, ?, ?, ?, 0, ?)
ON CONFLICT (id) DO UPDATE SET
    title = excluded.title,
    completed = excluded.completed,
    version = excluded.version,
    dirty = 0,
    deleted = excluded.deleted
```

A create inserts. An update overwrites. A delete sets `deleted = 1`; `list` hides
it. The server payload carries `deleted_at`, and the presence of a value is the
tombstone.

The upsert is not always unconditional. Before it, the CLI checks:

```rust
if let Some((version, dirty, title, completed)) = row
    && dirty
    && payload.version >= version
{
    record_conflict(pool, &payload, &title, completed).await?;
    continue;
}
```

A dirty local row with a server version at least as new as the local version is a
conflict. The CLI records both sides and keeps the local row. Chapter 16 explains
why `>=` rather than `>`: a same-version but different content is still a
disagreement worth showing.

After the page is applied, the cursor is stored:

```rust
set_last_seq(pool, page.cursor).await?;
println!("pulled {} change(s) to cursor {}", page.changes.len(), page.cursor);
```

If applying a change fails mid-page, the cursor is not advanced. The next run
re-fetches the page and re-applies the earlier changes. The upsert is idempotent,
so re-applying is safe. The conflict inbox is the exception: a failed apply can
record a conflict before the cursor update; the next run may record it again.
A dedupe key on `(task_id, server_version)==` would fix that; the book leaves it
visible.

## Local edits

`rename` and `done` only touch the local database:

```rust
sqlx::query("UPDATE local_tasks SET title = ?, dirty = 1 WHERE id = ? AND deleted = 0")
    .bind(title)
    .bind(id)
    .execute(pool)
    .await?;
```

The version is not incremented locally. It stays the server version the client
last saw. That is the base version for the next push. If the client incremented
locally, the server would compare the wrong number.

`dirty = 1` is the only signal that this row needs a push. `list` prints a `*`
after the version so the user can see pending work:

```text
  01J...  buy milk  (v1) *
x 01J...  call dentist  (v2)
```

A local rename is not validated against the server’s title length or other
rules. The push can fail with a 400; the local row stays dirty. A future form
should validate locally with the same rules, but the server is still the
authority.

## Push

`push` selects dirty, non-deleted rows:

```sql
SELECT id, title, completed, version FROM local_tasks
WHERE dirty = 1 AND deleted = 0
```

It builds one update operation per row:

```rust
SyncOperation {
    op_id: op_id(id, version, title, completed),
    entity: "task".to_owned(),
    entity_id: id.clone(),
    op: "update".to_owned(),
    base_version: version,
    title: Some(title.clone()),
    completed: Some(completed),
}
```

The operation ID is derived from content:

```rust
fn op_id(id: &str, version: i64, title: &str, completed: bool) -> String {
    let digest = Sha256::digest(format!("{id}:{version}:{title}:{completed}"));
    format!("task-{id}-v{version}-{}", hex::encode(&digest[..8]))
}
```

Why not `task-{id}-v{version}`? Because a previous successful push with that ID
could be replayed after the user edits the title again at the same base version.
The server would return the old success and the new edit would be lost. The
content hash makes a different edit a different operation.

The ID is deterministic, so a network retry with the same content reuses it. The
server’s `sync_operations` table returns the stored result instead of applying
the update twice. That is the client half of idempotency.

The batch is sent as JSON:

```rust
let response: SyncPushResponse = client
    .post(format!("{server}/api/v1/sync"))
    .bearer_auth(token)
    .json(&SyncPush { operations })
    .send()
    .await?
    .error_for_status()?
    .json()
    .await?;
```

The response has one result per operation. The client does not assume the order
matches, though the current server preserves it. It finds the local row by
recomputing the `op_id` from the row’s fields:

```rust
if let Some((id, _, _, _)) = rows
    .iter()
    .find(|(id, title, completed, version)| {
        result.op_id == op_id(id, *version, title, *completed)
    })
{
    // update local version, clear dirty
}
```

If the server returns `ok`, the local version becomes the server’s version and
`dirty = 0`. If it returns `conflict`, the client records both sides and leaves
the row dirty. If it returns `error`, it prints the message and leaves the row
dirty. A partial success is normal; the user can fix one task and sync again.

The local update is guarded by `version < ?`:

```sql
UPDATE local_tasks SET version = ?, dirty = 0
WHERE id = ? AND version < ?
```

The guard prevents a stale result from overwriting a newer local edit made while
the request was in flight. In a single-threaded CLI the race is unlikely, but the
guard costs nothing and documents the intent.

## Conflicts and resolution

The pull and push paths both call `record_conflict`:

```sql
INSERT INTO local_conflicts
(task_id, local_title, local_completed, server_title, server_completed,
 server_version, resolved)
VALUES (?, ?, ?, ?, ?, ?, 0)
```

`conflicts` prints the unresolved rows. `resolve <id> --keep local` sets the
local row’s version to the server version, keeps the local title/completion, and
marks it dirty again. The next sync pushes the local choice against the new base.
`--keep server` overwrites the local row with the server values and clears dirty.

Marking the conflict resolved does not delete it. The inbox has history; a later
`conflicts` run does not show it. If the same disagreement recurs, a new row is
created.

The CLI does not merge fields automatically. A conflict is a user decision, not
a last-write-wins guess. Chapter 16 details the semantics and the tests.

## Security and storage

The local database is a credential surface: it contains task titles and a cached
session token is not stored there, but the API token is in the environment or the
flag. Do not commit `cadence-local.db`; the repository’s `.gitignore` includes
`*.db`.

The server URL is a string. A malicious `--server` can point at a host that
collects the bearer token. That is the user’s choice, like `curl` with a token.
The client should not silently follow redirects to a different origin; the
current `reqwest` policy is the default. Exercise 3 asks you to restrict it.

The CLI does not log the token or the raw request. `reqwest` errors can include
the URL but not the header. Keep it that way.

## Tests and manual evidence

Two unit tests cover the client’s pure parts:

- `operation_ids_are_deterministic_and_content_sensitive`: same content produces
  the same ID; a different title produces a different ID.
- `payload_accepts_boolean_and_integer_completion`: both encodings decode.

The sync path is verified manually against a running server:

```console
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db \
    --server http://127.0.0.1:3000 --token "$TOKEN" sync
pulled 1 change(s) to cursor 1
  01J...  buy milk  (v1)
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db list
  01J...  buy milk  (v1)
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db rename 01J... "buy oat milk"
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db sync
pulled 0 change(s) to cursor 1
pushed task-01J...-v1-...
```

A second cache pointed at the same server pulls the update:

```console
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local2.db ... sync
pulled 2 change(s) to cursor 2
  01J...  buy oat milk  (v2)
```

The manual run is the integration evidence. A full automated test would start the
server on an ephemeral port and run the binary against it; that is an exercise.

## Checkpoint

```console
$ cargo test --bin cadence-cli
$ cargo run --bin cadence-cli -- --local local.db list
$ cargo run --bin cadence-cli -- --local local.db --server ... --token ... sync
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

`list` must work with the server stopped. `sync` must pull, push, and update the
cursor.

## Failure modes you will meet

**`set --token or CADENCE_TOKEN to sync`.** The sync command needs a bearer; the
offline commands do not. Export the token or pass the flag.

**`error sending request`.** The server is down, the URL is wrong, or TLS failed.
The local cache is untouched; retry when the server is back.

**A change is applied twice.** The cursor advanced only after a page, but a
failed apply can re-fetch. Upserts are idempotent; conflict rows can duplicate.
Dedupe the inbox by `(task_id, server_version, local_title)` if it matters.

**A local edit disappears after sync.** The server version was newer and the row
was not dirty, so the pull overwrote it. Mark local edits dirty immediately; do
not wait for `sync`.

**A conflict appears at the same version.** The dirty check uses `>=`, so a
same-version disagreement is treated as a conflict. That is conservative; the
alternative is to compare content.

**Only the first 100 changes arrive.** The CLI fetches one page. Run `sync` again,
or implement the loop in exercise 1.

**New tasks cannot be created offline.** `push` only sends `update` operations.
There is no local create ID mapping or server create endpoint in the batch. The
exercise asks for it.

**A deleted local task stays deleted on the server.** `push` filters
`deleted = 0`, and there is no delete operation. Deletes are not offline-capable
yet.

## Under the hood

The system has two SQLite databases: the server’s database and the CLI’s local
cache. They live in different processes; the CLI talks HTTP, not a shared file.
Do not point both at the same database file.

`reqwest::Client` is reused across the pull and push in one `sync`. It holds a
connection pool. Creating a new client per request throws away the pool and the
TLS session cache.

The cursor is a log position, not a version. The CLI stores it separately from
each task’s version. Do not try to derive one from the other.

The operation ID is content-addressed, but not a hash of the entire request. It
covers the fields that can change in this chapter: title and completion. If you
add a field to the operation, include it in the hash, or two different operations
can collide.

## Exercises

1. **Loop pages.** Keep pulling until `changes` is empty, then push. Add a test
   with more than 100 changes; use a smaller page size for the test.
2. **Create offline.** Add a local `create` command with a temporary ID. On push,
   send a create operation, map the server ID back, and update the local row.
   This is the hardest exercise; think about idempotency.
3. **Restrict redirects.** Configure `reqwest` to reject cross-origin redirects.
   Add a test with a mock server that redirects to a different host.
4. **Store the token safely.** Read it from the OS keyring, with an env fallback.
   Do not put the token in the local database.
5. **Add `--dry-run`.** Print the operations without sending them. Test that a
   dry run leaves dirty rows and the cursor unchanged.

## What comes next

The client can pull, edit, push, and record a conflict. Chapter 16 makes the
conflict model explicit: base versions, tombstones, the inbox lifecycle, and the
resolution rules the CLI already follows.
