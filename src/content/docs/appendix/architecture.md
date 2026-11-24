---
title: Architecture map
description: Module responsibilities, database tables, and request flows.
---

Cadence is a single Rust package with modules, one SQLite database, and two
processes: `serve` and `worker`. The map is for orientation; the chapters explain
why each boundary exists.

## Modules

| Module | Responsibility |
| --- | --- |
| `auth` | users, Argon2 passwords, sessions, verification/reset tokens |
| `workspaces` | workspaces, roles, projects, invitations, project access |
| `tasks` | task CRUD, versions, soft deletes, filter/sync updates |
| `query` | filter lexer/parser/AST and SQL compiler |
| `recurrence` | RRULE subset, time zones, next occurrence |
| `reminders` | reminder queue, claim, retry, email/notification delivery |
| `notifications` | in-app inbox, unread count, mark read |
| `mailer` | `Mailer` trait, log/recording implementations, templates |
| `audit` | append-only workspace audit log |
| `tokens` | API tokens, scopes, revocation, authentication |
| `idempotency` | stored responses and request hashes |
| `sync` | change pull, push operations, conflicts, operation replay |
| `events` | in-process broadcast bus for SSE |
| `webhooks` | webhook registry, delivery queue, HMAC, retries |
| `metrics` | request/login counters and Prometheus text |
| `security` | headers middleware and login limiter |
| `web` | MPA routes, templates, sessions, CSRF, SSE |
| `api` | JSON API, bearer extractor, idempotent create |
| `db` | SQLite pool, WAL, migrations |
| `cli` | command-line structure for the server binary |
| `telemetry` | tracing subscriber |
| `error` | `AppError` and HTTP/JSON mappings |

The binary `src/main.rs` dispatches subcommands. `src/bin/cadence-cli.rs` is the
separate sync client.

## Data model

| Table | Purpose |
| --- | --- |
| `users` | account identity and password hash |
| `sessions` | hashed session IDs, CSRF token, expiry |
| `auth_tokens` | verification/reset tokens, single use |
| `workspaces` | tenant, slug, owner |
| `workspace_members` | user role in a workspace |
| `projects` | project container, visibility |
| `project_members` | editor/viewer for private projects |
| `invitations` | email invitation token |
| `tasks` | task state, versions, soft delete, recurrence |
| `reminders` | delivery queue and retry state |
| `notifications` | in-app inbox |
| `audit_log` | append-only membership/project actions |
| `api_tokens` | hashed bearer tokens and scopes |
| `idempotency_keys` | stored create responses |
| `changes` | workspace change log |
| `sync_operations` | per-operation push responses |
| `webhooks` | endpoint, secret, event filter |
| `webhook_deliveries` | outbound delivery queue and retry state |

Migrations `0001`–`0012` build these in order. Migrations are append-only; do not
edit an applied file.

## Request flow: MPA

```text
browser
  -> GET /app (cookie)
  -> session lookup
  -> workspace/project selection
  -> task list query
  -> Askama render
  -> HTML

form POST
  -> CSRF check
  -> service authorization
  -> SQLite write + change trigger
  -> publish event
  -> queue webhook
  -> 303 redirect
```

The browser never gets a client-side model. The server is the renderer.

## Request flow: API

```text
client
  -> Authorization: Bearer <token>
  -> ApiUser extractor (hash lookup, scope)
  -> handler scope check
  -> service authorization
  -> SQLite
  -> JSON
```

The API scope is a coarse gate; the service role is the tenant gate. A `write`
token for a viewer is still 403.

## Sync flow

```text
client cursor
  -> GET /api/v1/sync?since=
  -> changes joined to workspace_members
  -> apply in seq order
  -> store cursor

local edits (dirty)
  -> POST /api/v1/sync with base_version
  -> compare-and-set
  -> ok | conflict | error
```

The change log is written by SQLite triggers on tasks, so it is atomic with the
task write. The pull is scoped by membership. The push is per-operation
idempotent through `sync_operations`.

## Worker flow

```text
serve --migrate-- SQLite
worker
  -> claim due reminders
  -> create notification
  -> send email (log/provider)
  -> mark sent or retry
  -> claim due webhook deliveries
  -> POST signed payload
  -> delivered or retry/fail
```

The worker is a separate process. It shares the SQLite file and the queue tables.
Keep its transactions short; SQLite has one writer.

## Deployment shape

```text
container
  cadence serve  -> :3000
  cadence worker -> no port
  /data/cadence.db
reverse proxy -> TLS, HSTS, /metrics restricted
```

The binary embeds templates, CSS, scripts, and migrations. The database and any
future media are the only persistent state.

## Where to read code

When a chapter refers to a behavior, read the module and its tests together:

- `src/<module>.rs` for the implementation and unit tests
- `tests/` for HTTP/API integration
- `migrations/` for the schema
- `templates/` for the MPA
- `src/bin/cadence-cli.rs` for sync

The tests are part of the map. A module without a test is a place to be careful.
