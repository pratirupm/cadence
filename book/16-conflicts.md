# 16. Conflicts

Two clients can edit the same task. One renames it, the other completes it. If
the server simply applies both, the last writer wins and the first user silently
loses an edit. If the server refuses every second write, the application becomes
unusable on a slow network. Optimistic concurrency picks a third path: accept the
write only if it was based on the current version, and return the current state
when it was not.

This chapter makes that path explicit. It covers the server’s conflict result,
the client’s conflict inbox, what “keep local” and “keep server” mean, and the
places where the model is still weak.

By the end of this chapter you can:

- explain a base version and why it is not the same as the latest version
- read a server `conflict` result and its current-task payload
- detect a conflict during a pull without overwriting a dirty local row
- list unresolved conflicts from the CLI
- resolve by keeping local or server and sync again
- explain why the operation ID includes a content hash
- identify duplicate-conflict and delete-conflict gaps
- decide when a field-level merge would be worth the complexity

The Rust lessons are result enums encoded as data, compare-and-set across a
network, and a local state machine for resolution. The product lesson is that a
conflict is a user decision, not an error to hide.

## What a conflict is

A conflict exists when all three are true:

1. The client has a local change it has not successfully pushed.
2. The server’s task version is at least the version the client based its change
   on.
3. The content differs.

The client stores the server version it last saw. That is the **base version**.
When it pushes an update, it sends `base_version`. The server loads the current
task and compares:

```rust
if task.version != operation.base_version {
    return Err(TaskError::Conflict);
}
```

If the versions differ, the server does not write. It returns a `conflict` result
containing the current task. The client now has two versions of the truth: its
local edit and the server row.

The comparison is entity-level, not field-level. A rename and a completion on
different fields still conflict if they share a base version. A field-level merge
could combine them, but it needs a rule for every pair of fields and a way to
show the merge. The book chooses entity-level first.

## The server result

`SyncResult` is a small data type:

```rust
pub struct SyncResult {
    pub op_id: String,
    pub status: String,
    pub version: Option<i64>,
    pub conflict: Option<Value>,
    pub error: Option<String>,
}
```

`status` is `ok`, `conflict`, or `error`. A batch response contains one result
per operation:

```json
{
  "results": [
    {"op_id":"task-...-v1-...","status":"conflict","version":2,
     "conflict":{"id":"...","title":"buy oat milk","completed":false,"version":2},
     "error":null}
  ]
}
```

The HTTP status of the batch is `200` even when one operation conflicts. The
batch succeeded; an individual operation did not. Returning `409` for the batch
would force the client to parse a body anyway, and a batch can contain one
conflict and one success. Per-operation status is the honest shape.

`SyncService::apply_operation` handles an update:

```rust
match tasks.apply_sync_update(
    user_id,
    &operation.entity_id,
    operation.title.as_deref(),
    operation.completed,
    operation.base_version,
).await {
    Ok(task) => Ok(SyncResult { status: "ok", version: Some(task.version), ... }),
    Err(TaskError::Conflict) => {
        let current = tasks.get(user_id, &operation.entity_id).await?;
        Ok(SyncResult {
            status: "conflict",
            version: Some(current.version),
            conflict: Some(serde_json::to_value(current)?),
            ...
        })
    }
    Err(error) => Err(SyncError::Task(error)),
}
```

The conflict payload is the current task, not a diff. The client already has its
local values; the server sends the values it would have to merge against. A diff
would be smaller but requires a merge base the client may not have.

`apply_sync_update` checks authorization before the version. A user who cannot
access the task gets `Forbidden`, not a conflict; do not leak the current state to
an unauthorized caller.

## The push replay

`sync_operations` stores the response per `(op_id, user_id)`:

```rust
if let Some(stored) = sqlx::query_scalar::<_, String>(
    "SELECT response FROM sync_operations WHERE op_id = ? AND user_id = ?",
)
.bind(&operation.op_id)
.bind(user_id)
.fetch_optional(&self.pool)
.await?
{
    results.push(serde_json::from_str(&stored)?);
    continue;
}
```

A retry with the same operation ID returns the stored result, including a
conflict. That is important: the server must not re-evaluate a conflict against a
newer state on a retry, because the client already received the first answer.

The operation ID is derived from content on the client:

```rust
fn op_id(id: &str, version: i64, title: &str, completed: bool) -> String {
    let digest = Sha256::digest(format!("{id}:{version}:{title}:{completed}"));
    format!("task-{id}-v{version}-{}", hex::encode(&digest[..8]))
}
```

Without the hash, a client could edit the title after a successful push at the
same base version, reuse `task-{id}-v{version}`, and receive the old success. The
new edit would never apply. The content hash makes a different edit a different
operation while keeping retries of the same edit idempotent.

## Detecting a conflict on pull

The push path is explicit: the server says `conflict`. The pull path has to
detect the same condition locally:

```rust
if let Some((version, dirty, title, completed)) = row
    && dirty
    && payload.version >= version
{
    record_conflict(pool, &payload, &title, completed).await?;
    continue;
}
```

A dirty row means the user has unpublished work. A server version greater than or
equal to the local version means the server has moved at least as far. The CLI
records both sides and does not apply the server payload.

Why `>=` and not `>`? A same-version server row with different content is still a
disagreement. It can happen if a previous push applied but the response was lost,
or if a manual repair changed the row without incrementing the version. Treating
it as a conflict is conservative and visible.

If the row is not dirty, the server payload wins. That is the normal sync: the
client is a cache, not an independent copy.

The check uses the local title and completion, not the whole task. The local
schema does not store notes, priority, or assignment yet. A full client would
store every field it intends to merge.

## The local conflict inbox

```sql
CREATE TABLE local_conflicts (
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

The inbox stores both values, not references, so a later server change does not
erase the state the user was shown. `server_version` is the base for a “keep
local” resolution.

`conflicts` prints:

```text
1  01J...  local: buy oat milk  server(v2): buy milk
```

The user sees the conflict ID, task ID, local value, server version, and server
value. The local completion is stored too; the printout is shortened for the
terminal.

`record_conflict` always inserts. A task can accumulate several rows for repeated
disagreements. That is useful history but noisy; the exercise asks for dedupe on
`(task_id, server_version, local_title, local_completed)`.

## Resolving

`resolve <id> --keep local`:

```sql
UPDATE local_tasks
SET title = ?, completed = ?, version = ?, dirty = 1
WHERE id = ?
```

The local values are kept, the base version becomes the server version, and the
row is dirty again. The next `sync` pushes the local choice with a base that
matches the server. The server accepts it and the local version advances.

`resolve <id> --keep server`:

```sql
UPDATE local_tasks
SET title = ?, completed = ?, version = ?, dirty = 0
WHERE id = ?
```

The server values overwrite the local row and the dirty flag clears. The next
sync pulls whatever changed after the conflict; there is no local edit left to
push.

In both cases the inbox row is marked `resolved = 1`. It is not deleted. A later
`conflicts` run hides it. If the same task conflicts again, a new row appears.

The resolution is per conflict row, not per task. Resolving one of three rows for
the same task leaves two. The user must resolve all of them, or the inbox should
group by task. The CLI prints the task ID so the user can see the duplicates; a
future UI should group them.

## Worked two-client trace

Two clients start from task v1:

```text
client A: local title "buy milk", version 1
client B: local title "buy milk", version 1
```

Client A renames to “buy oat milk” and syncs:

```text
A push: base 1, title "buy oat milk"
server: v2, title "buy oat milk"
A local: version 2, dirty 0
```

Client B renames to “buy soy milk” without pulling:

```text
B push: base 1, title "buy soy milk"
server: conflict, current v2 "buy oat milk"
B inbox: local "buy soy milk" / server v2 "buy oat milk"
```

If B chooses local:

```text
B resolve --keep local
B local: version 2, dirty 1, title "buy soy milk"
B sync: base 2 -> server v3 "buy soy milk"
```

If B chooses server:

```text
B resolve --keep server
B local: version 2, dirty 0, title "buy oat milk"
```

The version numbers are the protocol. The titles are the user’s decision. Neither
side is “right” in the abstract; the product asks.

## Decision table

The client’s behavior depends on two facts: is the local row dirty, and how does
the server version compare?

| Local dirty | Server version vs local | Result |
| --- | --- | --- |
| no | any | apply server payload |
| yes | greater | conflict, keep local dirty |
| yes | equal | conflict, keep local dirty |
| yes | less | apply server payload |

The last row is unusual: a server version lower than the local version means the
local row is ahead. That can happen if a previous push succeeded and the local
version was advanced but the dirty flag was not cleared. Applying the lower
server payload would lose the local state. The current code records a conflict
when `payload.version >= version`, so the lower case falls through to the upsert.
A stricter client would treat “local ahead” as a repair, not a merge. Exercise 2
in chapter 15 covers the dirty-flag bug.

The table is the whole sync decision for a task. If you add fields that can be
merged, the table gains a “both changed different fields” row and a merge rule.
That is the cost of field-level merge.

## Why not last-write-wins

Last-write-wins is simpler:

```text
server payload arrives -> overwrite local row
```

It is also wrong for an offline editor. A user who spends ten minutes editing a
task on a train expects that edit to survive a stale server row. If the server
wins by arrival time, the edit silently disappears. If the local always wins, a
server-side moderator’s correction disappears. A conflict at least asks.

The version is not a timestamp. Two edits can share a timestamp to the second and
still be different. The version increments per successful write, so it is a total
order for one entity. Use it; do not compare `updated_at`.

## Tests and manual evidence

The server side is covered by `pushes_updates_and_reports_conflicts`:

- a valid update with base 1 returns `ok`, version 2
- replaying the same `op_id` returns the stored result
- a stale update with base 1 returns `conflict` with the current task and version

The CLI’s pure operation-ID and payload parsing are covered by two unit tests in
chapter 15. The manual two-client run is the integration evidence:

```console
$ cadence-cli ... sync
conflict: local edits to 01J... are older than server v1
conflict: local edits to 01J... are older than server v2
pulled 2 change(s) to cursor 2
conflict on task-01J...-v1-...: run `conflicts` to resolve
$ cadence-cli ... conflicts
1  01J...  local: local edit  server(v1): buy milk
2  01J...  local: local edit  server(v2): buy oat milk
$ cadence-cli ... resolve 3 --keep local
$ cadence-cli ... sync
pulled 0 change(s) to cursor 2
pushed task-01J...-v2-...
```

The duplicate inbox rows are visible in that output: the pull recorded the
disagreement at v1 and v2, then the push recorded v2 again. This is the dedupe
gap, not a merge bug.

## Checkpoint

```console
$ cargo test sync
$ cargo test --bin cadence-cli
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

The manual two-cache trace is the end-to-end checkpoint. Use three terminals:
server, cache A, cache B.

## Failure modes you will meet

**A conflict at the same version.** The local row is dirty and the server version
equals the local version. The CLI uses `>=`, so it records the disagreement. A
client that uses `>` overwrites a local edit with a same-version server row.

**The same conflict appears three times.** Pull and push both record, and the
inbox does not dedupe. Resolve the task’s rows together or add a unique key.

**`resolve` says “conflict not found.”** The ID is already resolved, belongs to a
different local database, or the row was deleted. Check `conflicts` without the
resolved filter.

**A resolution does not push.** `--keep local` sets `dirty = 1`; run `sync`. A
resolution by itself is local, like any other edit.

**A 409 says the key was reused for a different request.** That is idempotency
collision, not a version conflict. The request hash changed; use a new key only
for a new request.

**A delete conflict has no payload to merge.** The server returns the current
task for a delete conflict, but the client has no local create/delete model. The
delete path is incomplete; the exercise asks for a tombstone-aware resolution.

**A stale conflict overwrites a newer resolution.** The inbox row is
`resolved = 1`, but a second row for the same task is unresolved. Resolve by
conflict ID, not by task ID, or group the rows.

## Under the hood

Optimistic concurrency is a compare-and-set on a version. The server does not
hold a lock between the client’s read and write; it detects the collision at
write time. The conflict response is the repair path.

A vector clock or CRDT would allow concurrent edits to merge without a user
decision. That is a different product: it needs a merge function per field, a
stable ordering for duplicates, and a UI that explains automatic merges. For a
todo app with a few fields, version-per-entity is simpler and predictable.

The tombstone is a versioned delete. A client that has the task at v2 and pulls a
delete at v3 removes it locally. A client with dirty v2 edits gets a conflict:
the server says “gone” and the client must decide whether to recreate. This
chapter does not implement recreate; it is an exercise.

## Exercises

1. **Dedupe the inbox.** Add a unique index on
   `(task_id, server_version, local_title, local_completed)` and upsert instead of
   insert. Update the conflict test.
2. **Group by task.** Make `conflicts` show one block per task with all unresolved
   rows, and make `resolve` accept a task ID plus a choice.
3. **Handle delete conflicts.** Define a result for “server deleted, local edited.”
   Offer “keep local (recreate)” and “keep server (discard).” Test both.
4. **Field-level merge.** Merge title from local and completion from server when
   the fields differ. Define what happens when both changed the same field.
5. **Automate the two-client test.** Start the server on an ephemeral port, run
   two local databases, and script the conflict and resolution. Assert the final
   server version.

## What comes next

The sync protocol can repair a conflict, but the browser still refreshes on a
button. Chapter 17 adds server-sent events so a connected client can see task
changes as they happen, with the same workspace scoping as the pull.
