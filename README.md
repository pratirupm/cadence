# Cadence

**The Todo App Grows Up: build a multi-user task service in Rust.**

Cadence is book two of *Learn Rust with Tests*. It takes the book-one CLI todo
app and turns it into a real service: accounts, workspaces, sharing, recurrence,
reminders, an API, a CLI sync client, email, webhooks, real-time updates, and
production operations.

Status: **complete** — 24 chapters, the Astro/Starlight book site, the full
reference service, 80 Rust tests, Docker packaging, and tested SQLite backups.
The manuscript and site checks are green. See [PLAN.md](PLAN.md) for the product
plan and known production gaps.

## Book

```console
$ bun install
$ bun run dev
$ bun run build
$ bun run check
$ bun run links
```

The chapters live in `src/content/docs/`. The site uses Astro Starlight, 0xProto,
terminal syntax highlighting, and a reference-source page that points at
`cadence-app/` rather than printing the app inline.

## App

```console
$ cd cadence-app
$ cargo run -- migrate
$ cargo run -- user create --email you@example.com --name You \
    --password "correct horse battery staple" --verified
$ cargo run -- serve
$ curl http://127.0.0.1:3000/health
{"status":"ok"}
```

The app is one `cadence` package with modules, a server binary, and a separate
`cadence-cli` sync binary. Modes: `serve`, `migrate`, `backup`, `worker`,
`user`, `token`, `reminder`.

## Principles

- Tests first, failure first, one behaviour at a time.
- Server-rendered MPA; no frontend framework, no HTMX, no app npm build.
- SQLite only; WAL and explicit migrations.
- Build the application engine; do not reimplement tokio or an HTTP framework.
- Every background job is idempotent and observable.
- Every chapter ends with a runnable checkpoint.

## Stack

Rust (edition 2024), axum, tokio, SQLx/SQLite, Askama, serde, thiserror,
tracing, argon2, ULID, chrono, chrono-tz, clap, reqwest, hmac.
Tests: unit, repository, HTTP (`tower`), CLI primitives, and manual transport
checks. The book site uses Astro Starlight and Bun.

## Deploy

```console
$ cd cadence-app
$ docker compose up --build
```

The service listens on `:3000` and stores SQLite data in the `cadence-data`
volume. Back up with:

```console
$ cargo run -- backup --database cadence.db --output cadence-backup.db
```

## Next step

Publish the site. The repository has no Git remote yet, so the GitHub Pages
workflow has not run. The remaining known gaps are production concerns, not
manuscript work; they are listed in the deployment chapter and in `PLAN.md`.
