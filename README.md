# Cadence

**The Todo App Grows Up: build a multi-user task service in Rust.**

Cadence is the project for book two of *Learn Rust with Tests*. It takes the
book-one CLI todo app and turns it into a real service: accounts, workspaces,
sharing, recurrence, reminders, an API, a CLI sync client, email, webhooks,
real-time updates, and production operations.

Status: **chapter 24 scaffolded** — the full service, CLI, API, sync,
reminders, webhooks, SSE, metrics, security hardening, performance indexes,
Docker packaging, tested SQLite backups, and 24 chapter drafts in
[book/](book/README.md). Includes CSRF, sessions, optimistic versions, soft
deletes, migrations, tracing, and CI. See [PLAN.md](PLAN.md) for the full
product, architecture, coverage, milestones, and risks.

## Run it

```console
$ cd cadence-app
$ cargo run -- migrate
$ cargo run -- user create --email you@example.com --name You \
    --password "correct horse battery staple" --verified
$ cargo run -- serve
$ curl http://127.0.0.1:3000/health
{"status":"ok"}
```

## Shape

The app starts as one `cadence` package. Crates split out only when a chapter
needs them:

- `cadence` — axum server, MPA, JSON API, workers
- later: `domain`, `store`, `web`, `api`, `worker`, `cli`, `mailer`
- One binary, modes growing from `serve`: `worker`, `migrate`, `user`, `backup`

## Principles

- Tests first, failure first, one behaviour at a time.
- Server-rendered MPA; no frontend framework, no HTMX, no npm.
- SQLite only; WAL and explicit migrations.
- Build the application engine; do not reimplement tokio or an HTTP framework.
- Every background job is idempotent and observable.
- Every chapter ends with a runnable demo and a failing test for the next step.

## Stack

Rust (edition 2024), axum, tokio, SQLx/SQLite, Askama, serde, thiserror,
tracing, argon2, ULID, chrono, lettre + provider adapter, clap, reqwest.
Tests with tower, insta, proptest, criterion, wiremock, and paused tokio time.

## Deploy

```console
$ cd cadence-app
$ docker compose up --build
```

The service listens on `:3000` and stores SQLite data in the `cadence-data`
volume. Migrations run at startup. Back up with:

```console
$ cargo run -- backup --database cadence.db --output cadence-backup.db
```

## Next step

Chapter 22: performance budgets and query-plan work.
