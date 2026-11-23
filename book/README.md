# Cadence — book two

**The Todo App Grows Up: build a multi-user task service in Rust.**

This directory holds the 24 chapters of book two. Each chapter maps to a working
checkpoint in `cadence-app/`, a set of tests, and a small set of exercises. The
reference implementation is the source of truth; chapter code excerpts are
intentionally short.

| Part | Chapters |
| --- | --- |
| I — From CLI to service | [1](01-the-sequel.md) · [2](02-sqlite-and-migrations.md) · [3](03-accounts.md) · [4](04-workspaces-and-projects.md) · [5](05-tasks.md) · [6](06-the-mpa.md) |
| II — The task engine | [7](07-query-dsl.md) · [8](08-recurrence.md) · [9](09-next-occurrence.md) · [10](10-reminders.md) · [11](11-notifications.md) · [12](12-sharing-deep.md) · [13](13-api.md) |
| III — Sync and real time | [14](14-change-log.md) · [15](15-cli-sync.md) · [16](16-conflicts.md) · [17](17-sse.md) · [18](18-live-ui.md) |
| IV — Production | [19](19-webhooks.md) · [20](20-observability.md) · [21](21-security.md) · [22](22-performance.md) · [23](23-deployment.md) · [24](24-capstone.md) |

## How to read a chapter

Each chapter follows the same shape:

1. **What you can do at the end.**
2. **The Rust you learn.** The concepts the chapter forces, not just uses.
3. **Build it.** The smallest sequence of changes that produces the behaviour.
4. **Checkpoint.** Commands and tests that prove the checkpoint.
5. **Exercises.** One or two changes that make the idea yours.

Run everything from `cadence-app/`.

```console
$ cargo test
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo fmt --check
```
