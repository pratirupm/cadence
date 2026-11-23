# 14. The change log

**What you can do at the end.** Ask for everything that changed in your
workspaces after a cursor and get ordered task operations.

**The Rust you learn.** Event logs, monotonic sequences, SQLite triggers,
tombstones, and read models built for replication.

**Build it.**

- Add a `changes` table: `seq`, `workspace_id`, `entity`, `entity_id`, `op`,
  `version`, `payload`, `created_at`.
- Add `tasks_after_insert` and `tasks_after_update` triggers.
- Encode the full task as JSON in the trigger payload.
- Mark deletes with `op = 'delete'` when `deleted_at` first appears.
- Add `SyncService::pull` filtered by workspace membership and `seq > since`.
- Return `{ changes, cursor }`; the cursor is the last sequence returned.

The trigger makes the log atomic with the write:

```sql
INSERT INTO changes (...)
VALUES (NEW.workspace_id, 'task', NEW.id, 'update', NEW.version, ...);
```

**Checkpoint.**

```console
$ cargo test sync
```

The tests assert create/update/delete order, versions 1→2→3, payloads, cursor
resumption, and that outsiders see no changes.

**Exercises.**

1. Add project changes to the log and a test for a project rename.
2. Add a retention job for old changes.
