---
title: "The change log"
description: "Record task changes with triggers, cursors, and tombstones."
---

The API can answer “what tasks exist now?” It cannot answer “what changed since I
last asked?” A client that polls the full list on every cycle wastes bandwidth and
still races with edits. A client that polls a timestamp needs a total order and
a way to see deletes.

This chapter adds a workspace change log. Every task insert and update writes a
row with a monotonic sequence, an operation, a version, and a JSON payload. A
client asks for changes after its last cursor. The log is the server’s record of
what happened, and the cursor is the client’s place in that record.

By the end of this chapter you can:

- read the change table and its task triggers
- explain why a trigger is atomic with the task write
- pull changes for the workspaces a user belongs to
- resume from a cursor and get only newer rows
- distinguish `create`, `update`, and `delete` operations
- see how a soft delete becomes a tombstone in the log
- describe what the log does not include yet
- explain the relationship between the change log and the audit log

The Rust lessons are `AUTOINCREMENT`, SQLite triggers, JSON payloads, cursor
queries, and typed rows over an append-only table. The systems lesson is that a
change log is a read model for replication, not a second source of truth.

## Why a change log

There are three common sync designs:

1. **Full snapshot.** The client downloads every task each time. Simple, correct,
   and wasteful. Deletes are obvious because the task is absent.
2. **Timestamp polling.** The client asks for rows with `updated_at > cursor`.
   Timestamps can collide, clock skew can hide rows, and deletes disappear.
3. **Append-only change log.** Every mutation writes an ordered event. The client
   keeps the last sequence. Deletes are explicit tombstones.

The change log is the third option. It is not event sourcing: the `tasks` table
is still the source of truth. The log is a replication aid that can be rebuilt
from the task table if it is lost (at the cost of history).

A `seq` is an integer assigned by SQLite’s `AUTOINCREMENT`. It is monotonic, not
gapless. Deleting the highest row can leave a gap in later values. Clients must use `>` and a stored
cursor, not assume the next value is `cursor + 1`.

The sequence is global across workspaces, not per workspace. A client that
belongs to two workspaces sees gaps when the other workspace changes; that is
fine. The cursor means “I have seen every change up to this sequence,” not “I
have seen every change in this workspace.”

## The schema

Create `migrations/0010_changes.sql`:

```sql
CREATE TABLE changes (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id TEXT NOT NULL,
    entity TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    op TEXT NOT NULL,
    version INTEGER NOT NULL,
    payload TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX changes_workspace_seq_idx
    ON changes (workspace_id, seq);

CREATE TABLE sync_operations (
    op_id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    response TEXT NOT NULL,
    created_at TEXT NOT NULL
);
```

`entity` is `task` for now. A future project or label event uses the same table
with a different value. `entity_id` is the ID of the affected row. `op` is
`create`, `update`, or `delete`. `version` is the task version after the change.
`payload` is the full task snapshot as JSON.

`sync_operations` is for the push side: it stores the response for an operation
ID so a retried batch does not apply the same operation twice. Chapter 16 uses it;
this chapter focuses on the pull side.

The index is `(workspace_id, seq)`. The pull query joins membership and filters
by `seq > cursor`; the index helps the per-workspace scan. A global `seq` index
would help a different query; measure before adding a second one.

## Triggers

The log is written by triggers, not by `TaskService`:

```sql
CREATE TRIGGER tasks_after_insert
AFTER INSERT ON tasks
BEGIN
    INSERT INTO changes (workspace_id, entity, entity_id, op, version, payload, created_at)
    VALUES (
        NEW.workspace_id,
        'task',
        NEW.id,
        'create',
        NEW.version,
        json_object(
            'id', NEW.id,
            'workspace_id', NEW.workspace_id,
            'project_id', NEW.project_id,
            'title', NEW.title,
            'notes', NEW.notes,
            'completed', NEW.completed,
            'priority', NEW.priority,
            'due_at', NEW.due_at,
            'version', NEW.version,
            'deleted_at', NEW.deleted_at
        ),
        NEW.updated_at
    );
END;
```

The update trigger is the same payload with an operation chosen from the old and
new state:

```sql
CASE
    WHEN NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL THEN 'delete'
    ELSE 'update'
END
```

A soft delete is an update that sets `deleted_at`. The trigger sees the old row
without a deletion timestamp and the new row with one, and records `delete`. The
payload still contains the row, including `deleted_at`, so a client can tombstone
it locally.

Why triggers instead of calling a `ChangeService` from every method?

- Every task mutation already goes through SQL; the trigger cannot be forgotten
  by a new service method.
- The change row commits in the same statement as the task change. There is no
  window where the task changed but the log did not.
- A migration or a manual repair that updates tasks also produces a log row.

The cost is hidden logic. A reader of `TaskService` does not see the insert into
`changes`; they have to know the schema. Debugging a missing change means checking
the trigger, not the Rust call. This is a real trade-off. For a single-table log
with a small team, triggers are a good fit. For complex business events with
external side effects, an explicit service call or an outbox is easier to reason
about.

There is no trigger for hard `DELETE`. The service soft-deletes tasks; a hard
delete would remove the row without a tombstone. If you add a purge job, it must
either write a final change or accept that clients will never see the purge.

## The payload

The payload is the task shape the sync client needs:

```json
{
  "id": "01J...",
  "workspace_id": "01J...",
  "project_id": "01J...",
  "title": "buy milk",
  "notes": "",
  "completed": 0,
  "priority": 0,
  "due_at": null,
  "version": 2,
  "deleted_at": null
}
```

`completed` is `0`/`1` because it comes from SQLite. The CLI’s parser accepts both
integer and boolean forms; the trigger always emits integers. The Rust `Task`
type decodes the column as `bool`, but the JSON payload is generated by SQLite,
not by `serde`.

The payload does not include `recurrence`, `recurrence_timezone`, `created_by`,
or `assigned_to`. That is a real gap: a sync client cannot reconstruct those
fields from the log. The table has them, and the `Task` JSON from the API does.
The exercise asks you to extend the payload and consider a payload version.

A payload without a version is a schema. When you add a field, old clients ignore
it; when you remove one, old clients may break. A `payload_version` column or a
version field in the JSON makes the change explicit.

## Pulling changes

`SyncService::pull` is the read side:

```rust
pub async fn pull(&self, user_id: &str, since: i64, limit: i64) -> Result<SyncPage, SyncError> {
    let changes = sqlx::query_as::<_, Change>(
        "SELECT c.seq, c.workspace_id, c.entity, c.entity_id, c.op, c.version,
                c.payload, c.created_at
         FROM changes c
         JOIN workspace_members m ON m.workspace_id = c.workspace_id
         WHERE m.user_id = ? AND c.seq > ?
         ORDER BY c.seq
         LIMIT ?",
    )
    .bind(user_id)
    .bind(since)
    .bind(limit.clamp(1, 500))
    .fetch_all(&self.pool)
    .await?;

    let cursor = changes.last().map(|change| change.seq).unwrap_or(since);

    Ok(SyncPage { changes, cursor })
}
```

The join is the authorization. A user sees changes only for workspaces where they
have a membership row. If they are removed from a workspace, the join stops
returning its changes on the next pull. Their local copy may still have the old
tasks; the client needs a separate “workspace removed” event or a full resync.
That is a known limitation of a membership-scoped log.

The `limit` is clamped to 1–500. A client cannot ask for the whole table in one
request. The cursor is the last returned `seq`, or the input `since` if the page
is empty. An empty page does not advance the cursor.

`ORDER BY c.seq` is the total order. The client applies changes in that order. A
create followed by an update must not be applied in reverse; the version fields
help detect an out-of-order or stale local row.

The pull is not a snapshot. If a change is inserted while the page is being read,
it may appear on the next page; the client loops until an empty page, then
continues from the cursor. The page boundary is not a transaction boundary.

## The API route

`GET /api/v1/sync?since=...&limit=...` requires a read token:

```rust
let page = SyncService::new(state.pool.clone())
    .pull(&user.user_id, query.since.unwrap_or(0), query.limit.unwrap_or(100))
    .await?;
Ok(Json(page))
```

The default `since` is `0`, which means “from the beginning.” A fresh client
should start there, apply the changes, and store the cursor. A client that lost
its cursor can do the same; it is a full resync.

The default limit is 100. An empty database returns `{ "changes": [], "cursor": 0 }`.

## Push, briefly

`POST /api/v1/sync` accepts a batch of operations. Each operation has an `op_id`,
`entity`, `entity_id`, `op`, `base_version`, and optional `title`/`completed`.
The server checks `sync_operations` for the `op_id` first; a replay returns the
stored result.

For an update, the server calls `TaskService::apply_sync_update`, which checks
the base version and authorization. A conflict returns the current task in the
result, not an HTTP error. The batch itself succeeds; individual operations have
statuses.

For a delete, the server loads the task, compares the version, calls soft delete,
and records the result. An unsupported entity or operation returns `status:
"error"` for that operation. Chapter 16 covers the conflict inbox; this chapter
only needs the pull path.

The push is a batch, not a transaction across operations. One operation can
succeed and the next fail. That is intentional: a client can retry only the
failures. The `sync_operations` table makes each retry idempotent.

## Worked change sequence

A task’s life looks like this in the log:

| seq | op | version | payload highlights |
| ---: | --- | ---: | --- |
| 1 | create | 1 | title “buy milk”, completed 0 |
| 2 | update | 2 | completed 1 |
| 3 | delete | 3 | deleted_at set |

A client with `since = 0` receives all three and stores cursor `3`. A client with
`since = 1` receives the update and delete, then cursor `3`. A client with
`since = 3` receives an empty page and keeps cursor `3`.

If the client applies the update before the create, it has a task it never
created. Apply in `seq` order, and treat an update for an unknown entity as a
reason to resync rather than an instruction to invent a row.

The version and sequence are different clocks. `seq` is the log position;
`version` is the entity’s edit count. A create is version 1 at seq 1. A later
unrelated task’s create can be seq 4 while this task is still version 1. Do not
use one as the other.

## Cursor rules

The cursor is a promise: “I have applied every change with `seq <= cursor`.”
Five rules follow from that promise:

1. Query with `seq > cursor`, never `>=`.
2. Advance only after applying the page successfully.
3. Store the server’s returned cursor, not the last ID you happened to parse.
4. Do not assume the next cursor is `cursor + 1`; gaps are normal.
5. On a lost or corrupt cursor, resync from 0 rather than guessing.

A client that violates rule 1 duplicates a change. A client that violates rule 2
can skip a change it never applied. Both are worse than a slow sync.

## Tests

Four tests cover the log:

- `records_create_update_and_delete_changes`: create, complete, delete; the ops
  are `["create", "update", "delete"]`, versions 1→2→3, payload contains the
  title, and the cursor is the last sequence.
- `pushes_updates_and_reports_conflicts`: a valid update returns `ok` with the
  new version; the same `op_id` replays; a stale base version returns `conflict`
  with the current task.
- `only_returns_changes_from_member_workspaces`: an outsider gets an empty page.
- `resumes_after_a_cursor`: a one-row page, then the next page after its cursor.

Run:

```console
$ cargo test sync
$ cargo test --test api
```

The trigger tests use the real services, so they prove the SQL and the Rust path
agree. A unit test that inserts into `changes` directly would not.

## Checkpoint

```console
$ cargo test sync
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

There is no CLI pull yet. The API route is the checkpoint; chapter 15 builds the
client that uses it.

## Failure modes you will meet

**A task changes but no change row appears.** The trigger is missing, disabled, or
the operation used a different table. Check `sqlite_master` for the trigger and
run a manual insert in a test.

**A delete looks like an update.** The trigger’s `CASE` compares `OLD.deleted_at`
and `NEW.deleted_at`. If both are non-null, it is an update; if the new one is
null, it is a restore. Decide how restore should appear and test it.

**A client misses a row.** It advanced its cursor past a sequence that was not
committed, or used `>=` instead of `>`. The cursor must be the last applied
sequence and the query must be strictly greater.

**A client sees gaps.** Gaps are normal. `AUTOINCREMENT` never reuses a value,
including after a deleted maximum. Do not loop “until cursor + 1.”

**A client sees another workspace’s change.** The pull join is missing or uses
the wrong user ID. The log itself is global; the membership join is the
authorization.

**A removed member still sees changes.** The membership row is gone, so the next
pull should not include the workspace. The client may still have local rows; it
needs a workspace-removal signal or a full resync.

**The payload is missing a field.** The trigger’s `json_object` is the schema. Add
the field in a migration and decide whether old clients can ignore it.

**The log grows forever.** There is no compaction. A production design archives
changes once every active client has acknowledged them, or rebuilds snapshots.
The book leaves retention to the deployment chapter.

## Under the hood

SQLite triggers run inside the statement that fired them. The task insert and the
change insert share a transaction even though the Rust code only issued the task
insert. That is the atomicity benefit.

`json_object` is part of SQLite’s JSON1 extension, bundled in the SQLite build
used by SQLx. The payload is text, not a typed column. The Rust side parses it
when needed; the database does not enforce its shape.

`AUTOINCREMENT` is stronger than `INTEGER PRIMARY KEY` alone: it guarantees a new
value is greater than every previous value, even after deletes. Without it,
SQLite might reuse an ID after deleting the maximum row, which would break a
cursor.

The change log is not the audit log. The audit records *people and actions*
(member added, invitation accepted). The change log records *task state for
replication*. They have different retention, access, and payload needs. Do not
merge them into one “events” table without deciding which consumers need which
guarantees.

## Exercises

1. **Log projects.** Add insert/update triggers for projects with a project
   payload. Update `pull` consumers and test a rename.
2. **Extend the task payload.** Add `recurrence`, `recurrence_timezone`,
   `created_by`, and `assigned_to`. Add a payload version and test an old-shaped
   row.
3. **Add compaction.** Delete changes below a cursor once all known clients have
   acknowledged it. Decide how the server knows “all clients.” Test the vacuum.
4. **Add a workspace-removed event.** When a membership is deleted, write a
   change that tells clients to drop the workspace. Test the pull after removal.
5. **Make pull page-safe.** Add a test that inserts a change between two pages
   and asserts the client sees it exactly once when it loops to empty.

## What comes next

The server can list changes after a cursor. Chapter 15 builds the CLI that stores
the cursor locally, applies the payloads to a SQLite cache, edits offline, and
pushes operations back. The log’s ordering and versions become the client’s
conflict rules.
