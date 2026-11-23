# 15. The CLI sync client

**What you can do at the end.** Work from a terminal against a local SQLite
cache, pull server changes, rename or complete a task, and push the edits.

**The Rust you learn.** A second binary in the same package, local schema
ownership, HTTP clients, cursor storage, and deterministic operation ids.

**Build it.**

- Add `src/bin/cadence-cli.rs` with `--server`, `--token`, and `--local`.
- Create `local_tasks`, `local_meta`, and `local_conflicts` tables.
- `sync` pulls from `/api/v1/sync?since=...`, applies task payloads, stores the
  cursor, then pushes dirty rows.
- `rename`, `done`, and `undo` only change local state and set `dirty = 1`.
- Build one update operation per dirty row with a content hash in the op id, so
  retries are idempotent but different edits are not conflated.
- Update the local version only after a successful push.

The local cache is the source of truth while offline:

```text
local edit -> dirty=1 -> sync -> push -> version updated, dirty=0
```

**Checkpoint.**

```console
$ cargo run --bin cadence-cli -- --local local.db list
$ cargo run --bin cadence-cli -- --local local.db sync
$ cargo run --bin cadence-cli -- --local local.db rename <id> "new title"
$ cargo run --bin cadence-cli -- --local local.db sync
```

The manual run pulls a task, renames it, pushes it, and a second cache sees the
new title at version 2.

**Exercises.**

1. Add a `create` command that uses the JSON API.
2. Add a `--dry-run` flag that prints operations without sending them.
