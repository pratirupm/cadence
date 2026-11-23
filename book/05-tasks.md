# 5. Tasks

**What you can do at the end.** Add, list, rename, complete, reopen, and delete
tasks inside a project.

**The Rust you learn.** Optimistic concurrency, soft deletes, `Option` fields,
and the difference between a missing row and a forbidden row.

**Build it.**

- Extend the task table with workspace, project, creator, assignee, notes,
  due date, priority, version, and deleted-at columns.
- Require workspace membership for reads and write permission for changes.
- Check project access separately: a private project needs project membership.
- Increment `version` on every mutation and update only when the incoming
  version matches.
- Soft-delete rows so sync clients can see a tombstone.

The update is a compare-and-set:

```rust
UPDATE tasks
SET title = ?, version = version + 1, updated_at = ?
WHERE id = ? AND version = ? AND deleted_at IS NULL
```

Zero affected rows means conflict, not “not found.”

**Checkpoint.**

```console
$ cargo test tasks
```

The tests cover trimming, empty titles, newest-first order, completion,
renaming, soft deletion, viewer restrictions, private projects, and filter
queries.

**Exercises.**

1. Add a `set_priority` method with the same version check.
2. Add a test that two concurrent renames produce one conflict.
