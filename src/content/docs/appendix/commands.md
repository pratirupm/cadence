---
title: Command reference
description: Server, CLI, API, test, database, and deployment commands used in the book.
---

All server commands run from your package directory (`cadence/` in the chapters,
`cadence-app/` in the reference repository). During development, `cargo run --`
replaces an installed `cadence`.

## Build and test

```console
$ cargo build
$ cargo build --release --bin cadence
$ cargo test
$ cargo test --lib
$ cargo test --test web
$ cargo test --test api
$ cargo test <filter>
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

Integration suites:

```text
tests/health.rs   health and 404
tests/web.rs      MPA signup/login/task/logout
tests/api.rs      bearer tokens and idempotency
```

## Server

```console
$ cargo run -- migrate
$ cargo run -- serve
$ cargo run -- serve --bind 0.0.0.0:3000
$ cargo run -- serve --database /tmp/cadence.db
$ cargo run -- worker
$ cargo run -- worker --once
$ cargo run -- backup --output /tmp/cadence-backup.db
```

The database flag is global:

```console
$ cargo run -- --database /tmp/cadence.db serve
```

Environment variables:

```text
CADENCE_DATABASE   SQLite path or URL
CADENCE_PASSWORD   password for user create
CADENCE_SERVER     CLI server URL
CADENCE_TOKEN      CLI bearer token
RUST_LOG           tracing filter
```

## Accounts and tokens

```console
$ cargo run -- user create \
    --email you@example.com \
    --name "You" \
    --password "correct horse battery staple" \
    --verified

$ cargo run -- token create \
    --email you@example.com \
    --name CLI \
    --scopes read,write
```

The token command prints `id raw-token` once. The raw token is not stored in
plaintext and cannot be shown again.

```console
$ cargo run -- reminder create \
    --email you@example.com \
    --task-id <id> \
    --at 2026-12-01T09:00:00Z
```

## CLI client

```console
$ cargo run --bin cadence-cli -- --help
$ cargo run --bin cadence-cli -- --local local.db list
$ cargo run --bin cadence-cli -- --local local.db \
    --server http://127.0.0.1:3000 --token "$TOKEN" sync
$ cargo run --bin cadence-cli -- --local local.db rename <id> "new title"
$ cargo run --bin cadence-cli -- --local local.db done <id>
$ cargo run --bin cadence-cli -- --local local.db undo <id>
$ cargo run --bin cadence-cli -- --local local.db conflicts
$ cargo run --bin cadence-cli -- --local local.db resolve <conflict-id> --keep local
```

`list`, `rename`, `done`, `undo`, `conflicts`, and `resolve` work offline. `sync`
needs a server and token.

## HTTP

```console
$ curl -i http://127.0.0.1:3000/health
$ curl -s http://127.0.0.1:3000/metrics
$ curl -N -b cookies.txt http://127.0.0.1:3000/events
```

API:

```console
$ curl -s -H "Authorization: Bearer $TOKEN" \
    http://127.0.0.1:3000/api/v1/me

$ curl -s -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:3000/api/v1/workspaces/$WS/projects/$PROJECT/tasks?filter=due:today"

$ curl -s -X POST \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -H "Idempotency-Key: create-1" \
    -d '{"title":"buy milk"}' \
    "http://127.0.0.1:3000/api/v1/workspaces/$WS/projects/$PROJECT/tasks"

$ curl -s -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:3000/api/v1/sync?since=0&limit=100"

$ curl -s -X POST \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"operations":[{"op_id":"op-1","entity":"task","entity_id":"<id>","op":"update","base_version":1,"title":"new","completed":null}]}' \
    http://127.0.0.1:3000/api/v1/sync
```

Webhooks (admin scope and workspace admin):

```console
$ curl -s -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"url":"https://example.com/hook","events":["task.created"]}' \
    "http://127.0.0.1:3000/api/v1/workspaces/$WS/webhooks"

$ curl -s -X DELETE \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    "http://127.0.0.1:3000/api/v1/workspaces/$WS/webhooks/$HOOK"
```

## Database

The service uses SQLite in WAL mode. A SQLite client is optional.

```console
$ sqlite3 cadence.db ".tables"
$ sqlite3 cadence.db "SELECT id, title, completed, version FROM tasks LIMIT 5;"
$ sqlite3 cadence.db "EXPLAIN QUERY PLAN SELECT id FROM tasks WHERE project_id='...' ORDER BY completed, created_at DESC;"
$ sqlite3 cadence.db "PRAGMA integrity_check;"
```

Backup and restore:

```console
$ cargo run -- backup --output backup.db
$ cp backup.db restored.db
$ cargo run -- serve --database restored.db
```

Do not copy only `cadence.db` while the service is writing; use the backup command
or stop the writers and checkpoint.

## Docker

```console
$ docker compose build
$ docker compose up migrate
$ docker compose up -d cadence worker
$ docker compose logs -f cadence
$ docker compose down
```

The image exposes port 3000 and stores the database in the `cadence-data` volume.

## Formatting and linting

```console
$ cargo fmt
$ cargo clippy --all-targets --all-features -- -D warnings
```

The book’s house style is types-first, tests at the bottom, bound SQL, and no
`unwrap` in request paths.
