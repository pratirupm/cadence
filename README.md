# Cadence

**The Todo App Grows Up: build a multi-user task service in Rust.**

Cadence is the project for book two of *Learn Rust with Tests*. It takes the
book-one CLI todo app and turns it into a real service: accounts, workspaces,
sharing, recurrence, reminders, an API, a CLI sync client, email, webhooks,
real-time updates, and production operations.

Status: **chapter 2 scaffolded** — health endpoint, config, structured errors,
`cadence serve`, migrations, SQLite task storage, graceful shutdown, tracing,
and CI. See [PLAN.md](PLAN.md) for the full product, architecture, book outline,
coverage, milestones, and risks.

## Run it

```console
$ cd cadence-app
$ cargo run -- migrate
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

## Next step

Chapter 2: SQLite, migrations, and the first task repository.
