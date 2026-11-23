# 16. Conflicts

**What you can do at the end.** Detect a stale local edit, inspect both
versions, keep one, and sync again.

**The Rust you learn.** Optimistic concurrency across a network, conflict
records, state transitions, and keeping the user in control.

**Build it.**

- On push, compare `base_version` with the server version.
- Return `status = "conflict"` plus the current task JSON.
- On pull, if a local row is dirty and the server version is greater than or
  equal, record a conflict instead of overwriting.
- Store local and server titles, completions, and the server version.
- Add `conflicts` and `resolve --keep local|server` to the CLI.
- Keeping local rebases the version and leaves the row dirty; keeping server
  overwrites the cache and clears dirty.

A conflict is data, not an exception:

```rust
Ok(task) if task.version == operation.base_version => apply,
Ok(task) => conflict(task),
```

**Checkpoint.**

```console
$ cargo test sync
$ cargo run --bin cadence-cli -- --local local.db conflicts
$ cargo run --bin cadence-cli -- --local local.db resolve 1 --keep local
```

The manual run shows three conflict records (create, update, push), resolves
one, and pushes the local edit as version 3.

**Exercises.**

1. Add a field-level merge for title and completion.
2. Add a conflict count to `/metrics`.
