# 24. Capstone

**What you can do at the end.** Run one end-to-end scenario that touches every
layer: signup, workspace, project, recurring task, reminder, API token, sync,
conflict, webhook, metrics, backup, and shutdown.

**The Rust you learn.** Integration, operational judgment, and the difference
between “the tests pass” and “the service is ready.”

**Build it.**

1. Start from an empty database and run `migrate`.
2. Sign up in the browser, create a workspace and project, and add a task.
3. Give the task a daily rule and complete it; confirm the next occurrence.
4. Create a reminder and run `worker --once`; confirm email plus in-app
   notification.
5. Create a read/write token and use the JSON API to list and create a task.
6. Sync the CLI, rename locally, push, and resolve a stale conflict.
7. Register a webhook, dispatch a task event, and inspect a signed delivery.
8. Scrape `/metrics`, back up the database, restore it into a fresh file, and
   start the service against the restore.
9. Send SIGTERM while a request is in flight; confirm graceful shutdown.

**Checkpoint.**

```console
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ docker compose up --build
```

The capstone is a runbook, not a new feature. Write down the commands, the
expected output, and one thing that surprised you.

**Exercises.**

1. Add a health check to the container image using the real `/health` route.
2. Write a one-page incident runbook: database locked, queue backlog, webhook
   dead letter, lost session key.
