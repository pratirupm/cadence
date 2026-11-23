# Cadence — full plan

**Book two of *Learn Rust with Tests***
**Subtitle:** *The Todo App Grows Up: build a multi-user task service in Rust*

## 1. Promise and audience

**Reader:** completed book one, or knows Rust ownership, structs/enums, traits,
`Result`, tests, and Cargo. No async, web, or database experience assumed.

**By the end, the reader can:**

- build and deploy an axum service with SQLite
- design sessions, CSRF, roles, and an authorization matrix
- write a query DSL, a recurrence engine, and a reminder scheduler
- run background workers with retries and idempotency
- expose a versioned API with tokens, rate limits, and webhooks
- build a CLI sync client with offline edits and conflict resolution
- stream updates with SSE
- test time, concurrency, permissions, and sync behaviour
- operate the service: migrations, backups, metrics, graceful shutdown

**Teaching style:** failure-first TDD, framework seams, production notes, no
reimplementation of tokio. Build the application engine, not a runtime.

## 2. Product scope

### MVP

| Area | Features |
|---|---|
| Accounts | Sign up, email verification, login/logout, password reset |
| Workspaces | Personal workspace, members, roles, invitations |
| Projects | Create, rename, archive, workspace or private visibility |
| Tasks | Title, notes, subtasks, labels, priority, due date, assignee, complete/reopen, delete |
| Views | Inbox, Today, Upcoming, project, label, search |
| Search/filter | Query DSL (`due:today & label:work`), FTS for text |
| Recurrence | Daily/weekly/monthly/yearly subset, time zones, next occurrence |
| Reminders | Scheduled email and in-app notifications, retries |
| Sharing | Workspace roles, private projects, task assignment |
| API | Token auth, scopes, pagination, idempotency |
| Webhooks | Signed deliveries, retries, dead letter |
| CLI | Login, add, list, complete, sync, conflicts, import book-one JSON |
| Real-time | SSE stream for task changes |
| Ops | Audit log, metrics, backups, graceful shutdown |

### Non-goals

Kanban board, calendar UI, mobile apps, attachments, billing, integrations
marketplace, AI, public link sharing, OAuth, 2FA, full offline browser/CRDT
sync, self-hosted SMTP.

### Personas

- **Personal user:** quick add, Today, recurring chores, reminders.
- **Team member:** shared workspace, assigned tasks, comments, activity.
- **CLI user:** offline edits, `cadence sync`, conflict inbox.
- **Workspace owner:** invites, roles, tokens, webhooks, audit.

## 3. Architecture

```text
cadence/
  crates/
    domain/        tasks, projects, permissions, recurrence, query AST
    store/         SQLx + SQLite, migrations, repositories
    web/           axum routers, Askama MPA, sessions, CSRF, errors
    api/           JSON API, tokens, idempotency, sync, SSE
    worker/        reminders, email, webhooks, recurrence rollover
    cli/           local SQLite, sync client, commands
    mailer/        SMTP + provider adapters, templates
    test-support/  temp DB, fake clock, mock HTTP, fixtures
  migrations/
  templates/
  static/
```

**Implementation order:** start with one `cadence` package in `cadence-app/`.
Split the library and binary when route tests need them, then split workspace
crates when the CLI and worker arrive. The tree above is the target shape, not
the day-one scaffold.

**Runtime modes:** one binary, subcommands `serve`, `worker`, `migrate`,
`user`, `backup`. Small deployments can run workers in-process.

**Request shape:** axum → auth/session → authorization → handler →
repository → SQLite. A mutation and its change-log row are one transaction.

**UI:** server-rendered MPA with Askama. No HTMX, no frontend framework, no
npm. One optional hand-written `live.js` for SSE refresh and optimistic
checkbox toggles; every core flow works without JavaScript.

**Database:** SQLite only, WAL mode. Postgres is an appendix, not a supported
second path.

**Email:** SMTP in dev (Mailpit), provider API in production.

## 4. Stack

| Concern | Choice |
|---|---|
| HTTP | axum + tower-http |
| Async | tokio |
| DB | SQLx + SQLite (WAL, FTS5) |
| Templates | Askama |
| IDs | ULID |
| Time | `time` or `chrono` + `chrono-tz` |
| Recurrence | hand-written RRULE subset, differential-tested against `rrule` |
| Passwords | Argon2id |
| Email | lettre (dev) + provider adapter (prod) |
| HTTP client | reqwest |
| Errors | thiserror + custom `IntoResponse` |
| Observability | tracing, metrics |
| Tests | tower::ServiceExt, insta, proptest, criterion, wiremock/httpmock, paused tokio time |
| CLI | clap + SQLx/SQLite + reqwest |

## 5. Data model

| Table | Key fields |
|---|---|
| `users` | id, email, password_hash, name, email_verified_at, disabled_at |
| `sessions` | id_hash, user_id, csrf_token, expires_at, ip, user_agent |
| `api_tokens` | id, user_id, name, token_hash, scopes, expires_at, revoked_at |
| `workspaces` | id, name, slug, owner_id |
| `workspace_members` | workspace_id, user_id, role |
| `invitations` | id, workspace_id, email, role, token_hash, expires_at, accepted_at |
| `projects` | id, workspace_id, name, visibility, archived_at |
| `project_members` | project_id, user_id, role |
| `tasks` | id, workspace_id, project_id, parent_id, series_id, title, notes, status, priority, due_at, due_timezone, assigned_to, created_by, completed_at, version, deleted_at |
| `recurrence_rules` | id, task_id, rrule, timezone, next_at |
| `labels` / `task_labels` | workspace-scoped labels, join table |
| `reminders` | id, task_id, user_id, remind_at, channel, sent_at, status, attempts |
| `comments` | id, task_id, author_id, body, deleted_at |
| `activity` | workspace_id, actor_id, entity, entity_id, action, metadata |
| `notifications` | user_id, kind, payload, read_at |
| `changes` | seq INTEGER PK AUTOINCREMENT, workspace_id, entity, entity_id, op, version, payload |
| `idempotency_keys` | key, user_id, request_hash, response_status, response_body, expires_at |
| `webhooks` | id, workspace_id, url, secret, events, active |
| `webhook_deliveries` | id, webhook_id, event, payload, status, attempts, next_at, response |
| `jobs` | id, kind, payload, run_at, attempts, status, locked_by |
| `audit_log` | workspace_id, actor_id, action, target, metadata, ip, created_at |

**Invariants:**

- Task and project share a workspace; parent/child tasks share a project.
- Subtask depth is limited; cycles are rejected.
- Version increments on every mutation; mutation and change-log row are atomic.
- Recurring task completion creates the next occurrence and preserves series history.
- Private projects require explicit membership; workspace visibility requires workspace membership.
- A workspace always has at least one owner.
- Deleted entities become tombstones in the change log.
- Idempotent replay returns the stored response.

## 6. API and sync

### REST API (`/api/v1`)

| Method | Path | Purpose |
|---|---|---|
| POST | `/auth/token` | Exchange credentials for an API token |
| GET | `/me` | Current user and workspaces |
| GET/POST | `/workspaces` | List/create |
| GET/POST | `/workspaces/{id}/projects` | List/create |
| GET/POST | `/projects/{id}/tasks` | List/create with `filter`, `cursor`, `limit` |
| GET/PATCH/DELETE | `/tasks/{id}` | Read/update/delete |
| POST | `/tasks/{id}/complete` | Complete; handles recurrence |
| GET/POST | `/tasks/{id}/comments` | Threaded comments |
| GET | `/sync?since=<seq>&limit=` | Pull changes |
| POST | `/sync` | Batch push with per-op IDs and base versions |
| GET | `/events` | SSE stream |
| GET/POST/DELETE | `/tokens`, `/webhooks` | Management |

**Conventions:** cursor pagination, JSON problem details, `Idempotency-Key` on
mutations, token scopes, rate limits per token/IP, UTC timestamps plus time
zone fields.

### Sync protocol

- Every mutation appends to `changes` with a monotonic workspace sequence.
- Clients pull after their last sequence, then push local operations.
- Each pushed operation carries `op_id`, entity, `base_version`, and payload.
- The server applies operations in a transaction, returns new versions, and
  reports conflicts with the current entity.
- Conflict policy is entity-level optimistic concurrency, not CRDT: server
  wins until the client resolves and retries.
- Tombstones propagate deletes; replay of `op_id` is idempotent.
- Property tests: no lost updates, convergence after replay, duplicate and
  out-of-order handling.

## 7. Web app

### Routes

| Area | Pages |
|---|---|
| Public | `/`, `/signup`, `/login`, `/verify/{token}`, `/reset/{token}`, `/invite/{token}` |
| App | `/app`, `/app/today`, `/app/upcoming`, `/app/inbox`, `/app/projects/{id}`, `/app/tasks/{id}`, `/app/search` |
| Settings | `/app/settings/profile`, `/notifications`, `/tokens`, `/workspace/members`, `/workspace/webhooks`, `/workspace/audit` |

### MPA behaviour

- `GET` renders; `POST` validates; success `303` redirects; failure re-renders
  with `422`.
- Every form has a CSRF token; every destructive action has a confirmation page.
- Field errors plus a top summary; preserved input; `autofocus` on the first
  invalid field.
- Flash messages; pagination links preserve filters; sort columns are whitelisted.
- Quick-add form on every list page; task completion is a POST form.
- Optional `live.js`: SSE-driven refresh and optimistic checkbox toggles.
  Disabled JavaScript leaves the app fully usable.
- Accessibility: semantic landmarks, labels, focus styles, keyboard operation,
  contrast, no color-only status.
- One hand-written CSS file; system fonts; responsive forms and tables.

## 8. Auth and security

- Argon2id passwords; email verification; reset and invite tokens.
- Web sessions: opaque IDs stored hashed; `HttpOnly`, `Secure`, `SameSite=Lax`;
  rotation on login; expiry and cleanup.
- API tokens: hashed, scoped, revocable, last-used tracking; shown once.
- CSRF synchronizer tokens on all POSTs.
- Roles: workspace owner/admin/member/viewer; project editor/viewer for private
  projects.
- Authorization resolver combines workspace role, project visibility, and
  membership. Test matrix: role × route × resource.
- Login rate limits by IP + email; token/IP rate limits; upload and webhook limits.
- Security headers: CSP with no inline scripts, `frame-ancestors 'none'`,
  `nosniff`, HSTS in production.
- Webhooks: HTTPS only, HMAC signature, timestamp, timeout, retries, block
  private IPs and DNS rebinding (SSRF).
- Audit log for membership, roles, tokens, webhooks, and destructive actions.
- Privacy: account export and delete; no PII in logs; retention policy for
  sessions, change log, deliveries.

## 9. The task engine

### Query DSL

Grammar: `field:value`, comparisons, `&`, `|`, `!`, parentheses, quoted strings,
relative dates (`today`, `tomorrow`, `overdue`, `none`), sort directives.

Fields: project, label, status, priority, due, assigned, created, updated,
text. Compile to parameterized SQL with a whitelist; reject unknown fields.
Used by web search and CLI. Property tests for parser round-trip and SQL
equivalence.

### Recurrence

RRULE subset: `FREQ` daily/weekly/monthly/yearly, `INTERVAL`, `BYDAY`,
`BYMONTHDAY`, `COUNT`, `UNTIL`, `WKST`. No `BYSETPOS`, no `BYYEARDAY`.

- Store rule plus time zone; expand in local wall time.
- DST: nonexistent local times shift forward; ambiguous times choose the first
  occurrence; document both.
- Completing a recurring task marks the occurrence done and creates the next
  occurrence with a new ULID, same series ID.
- Missed occurrences do not backfill; next occurrence is after now.
- Differential tests against the `rrule` crate for supported cases; property
  tests for monotonicity and limits.

### Reminders

- DB-backed reminders plus an in-memory heap for the next due batch.
- Worker polls, claims, sends, marks sent.
- At-least-once delivery; idempotency key per reminder; retries with
  exponential backoff; dead-letter visibility.
- Quiet hours and per-user preferences; time-zone aware.
- Tests use paused tokio time and a fake mailer; no real sleeps.

### Notifications

Email and in-app: assignment, mention, reminder, invitation, password reset.
Deduplicate, batch where sensible, respect preferences, no marketing.

### Webhooks

Workspace-level subscriptions: task created/updated/completed/deleted, comment
created. Signed payload, retries, delivery log, dead letter, manual replay.

## 10. CLI sync client

The book-one CLI grows up:

```text
cadence login
cadence add "buy milk" --due tomorrow --project home
cadence list --filter "due:today"
cadence done <id>
cadence sync
cadence conflicts
cadence import .todo.json
```

- Local SQLite with pending operations, last sequence, and a conflict inbox.
- Offline add/edit/complete; sync pull-then-push; idempotent operation IDs.
- Entity-level conflicts surface local vs server diff; user resolves; retry
  with new version.
- Token stored in a config file with `0600`; OS keyring is an appendix.
- Import maps book-one `.todo.json` records into a personal workspace.

## 11. Real-time

SSE at `/api/v1/events`:

- Per-user stream filtered to accessible workspaces/projects.
- `Last-Event-ID` replays from the change log.
- Heartbeats; slow-consumer detection; backpressure; cancellation on disconnect.
- Event types: task created/updated/completed/deleted, comment created,
  reminder due.
- Web UI uses the optional `live.js`; API clients can consume SSE directly.

## 12. Operations

- **Observability:** tracing spans with request ID, user ID (not email),
  workspace; metrics for latency, errors, login failures, reminder delay,
  webhook failures, queue depth, DB size.
- **Performance targets:** API p95 < 150 ms locally, 1,000-change sync < 500 ms,
  reminder delay < 60 s, 100 req/s on a small VM. Load tests with `oha`/`k6`;
  benchmarks with `criterion`.
- **Deployment:** single binary + SQLite + static assets; Docker and systemd
  examples; Caddy/Nginx TLS; explicit `migrate`; graceful shutdown; worker
  supervision.
- **Backups:** `VACUUM INTO` for the DB, file copy for static assets/config;
  retention; restore drill in the runbook.
- **SLOs and alerts:** error rate, p95 latency, reminder backlog, webhook dead
  letter, failed logins.
- **Migrations:** forward-only, tested from empty and previous schema; backup
  before migrate.

## 13. Book outline — 24 chapters

### Part I — From CLI to service (6)

| # | Chapter | Product | Rust focus |
|---|---|---|---|
| 1 | The sequel | Scope, workspace, health endpoint, import plan | axum, workspaces, errors |
| 2 | SQLite and migrations | Tasks schema, repositories, ULIDs | SQLx, transactions, traits |
| 3 | Accounts | Signup, login, verification, reset | Argon2, sessions, extractors |
| 4 | Workspaces and projects | Membership, roles, invitations | Authz matrix, joins |
| 5 | Tasks | CRUD, subtasks, labels, due dates, versions | Validation, optimistic concurrency |
| 6 | The MPA | Lists, forms, filters, pagination | Askama, CSRF, accessibility |

### Part II — The task engine (7)

| # | Chapter | Product | Rust focus |
|---|---|---|---|
| 7 | Query DSL | Parser, AST, SQL compilation | Parsing, lifetimes, property tests |
| 8 | Recurrence I | RRULE subset, expansion | Iterators, time zones |
| 9 | Recurrence II | DST, completion, series history | Edge cases, differential tests |
| 10 | Reminders | Heap, scheduler, worker, retries | Collections, timers, idempotency |
| 11 | Notifications | Email adapter, preferences, dedupe | Traits, async, templates |
| 12 | Sharing deep | Private projects, authz resolver, audit | Permissions, security tests |
| 13 | API | Tokens, scopes, idempotency, rate limits | Extractors, middleware, errors |

### Part III — Sync and real-time (5)

| # | Chapter | Product | Rust focus |
|---|---|---|---|
| 14 | Change log | Workspace sequence, tombstones | Transactions, event log design |
| 15 | CLI sync | Local SQLite, pull/push | CLI, HTTP client, idempotency |
| 16 | Conflicts | Entity versions, conflict inbox | State machines, property tests |
| 17 | SSE | Live events, replay, backpressure | Streams, `Pin`, cancellation |
| 18 | Live UI | Optional vanilla JS enhancement | Progressive enhancement, a11y |

### Part IV — Production (6)

| # | Chapter | Product | Rust focus |
|---|---|---|---|
| 19 | Webhooks | Signing, retries, SSRF, dead letter | HMAC, retries, security |
| 20 | Observability | Tracing, metrics, request IDs | Instrumentation, SLOs |
| 21 | Security hardening | IDOR, CSRF, XSS, rate limits, secrets | Threat modeling, test matrix |
| 22 | Performance | Indexes, N+1, FTS, benchmarks | Query plans, measurement |
| 23 | Deployment | Docker, backups, migrations, runbook | Ops, graceful shutdown |
| 24 | Capstone | Two users, shared workspace, recurrence, reminder, CLI sync, webhook | Integration |

**Appendices:** book-one recap/starter, macros (derive commands/events), unsafe
lab (arena/interval tree), advanced types (typestate, `Pin`, GATs), email
deliverability, CRDT future.

## 14. Coverage of the "missing half"

| Area | Coverage | Where |
|---|---:|---|
| Generics/traits | high | repositories, commands, recurrence, notifiers, sync |
| Lifetimes | medium | filters, borrowed trees, SSE |
| Iterators/closures | high | filtering, recurrence expansion, sync diffs |
| Collections | high | reminder heap, sync maps, project trees, indexes |
| Smart pointers | high | `Arc`/`RwLock`, `Rc`/`RefCell` engine tests |
| Threads/sync | high | reminder/email workers, sync |
| Async/networking | high | axum, SSE, timers, webhooks |
| Unsafe/FFI | appendix | arena/interval tree lab |
| Macros | partial | derive commands/events, `macro_rules!` filters |
| Ecosystem/errors | high | SQLx, serde, thiserror, tracing, lettre |
| Workspaces/release | high | seven-crate workspace, deploy |
| Testing/tooling | high | property tests, paused time, golden HTML |

**Natural total: about 82%.** With macro and unsafe appendices as chapters:
about 85%.

## 15. Testing and quality

| Layer | What |
|---|---|
| Unit | recurrence, DSL, permissions, validation, idempotency |
| Property | recurrence vs oracle, DSL equivalence, sync convergence, pagination |
| Integration | API with temp SQLite; auth, tokens, sync, webhooks with mock server |
| CLI | spin server + CLI, offline edits, conflicts, import |
| Golden HTML | MPA pages, error pages |
| Time | paused tokio time, fake clock, deterministic reminders |
| Security | role × route × resource matrix, IDOR, CSRF, XSS, SSRF |
| Migration | empty → latest; previous schema → latest |
| Load | API, sync, reminder backlog; `criterion` for hot paths |
| Accessibility | keyboard-only, screen reader, contrast, no-JS pass |

CI: fmt, strict Clippy, tests, SQLx offline metadata, migrations,
`cargo audit`/`cargo deny`.

Book QA: docs-only reader kit, three clean-room passes, hidden reference app,
same workflow as book one.

## 16. Milestones

| Milestone | Exit criteria |
|---|---|
| M0 | Workspace, config, SQLite, health, CI, error pages |
| M1 | Auth, sessions, CSRF, workspaces, projects, roles |
| M2 | Tasks, MPA, filters, pagination, import book-one data |
| M3 | Query DSL, recurrence, reminders, email |
| M4 | Sharing, API tokens, idempotency, webhooks |
| M5 | Change log, CLI sync, conflicts, SSE |
| M6 | Hardening, load test, backup/restore, deploy, capstone |

**Feature freeze after M2.** Recurrence, sync, and webhooks are the planned
engine; everything else waits.

## 17. Risks and cuts

| Risk | Mitigation |
|---|---|
| Scope creep | Fixed MVP, feature freeze, non-goals |
| Recurrence/DST bugs | Strict subset, oracle differential tests, property tests |
| Sync complexity | Entity-level conflicts, no CRDT, conflict inbox |
| SQLite concurrency | WAL, one writer, short transactions, backups |
| Email deliverability | Provider adapter, no self-hosted SMTP |
| UI expectations | Deliberately plain MPA, optional tiny JS, no drag/drop/calendar |
| Multi-tenant leaks | Workspace/project scoping in every query; authz matrix tests |
| Time zones | Store UTC + zone; test DST boundaries |
| Webhook abuse | HTTPS, SSRF guard, signatures, retries, dead letter |
| Book length | 24 chapters, cut attachments/OAuth/2FA/billing |

## 18. Decisions to confirm

1. **Repo:** book repo; reference app hidden in `cadence-app/`.
2. **Crate strategy:** one `cadence` package first; split lib/bin, then workspace
   crates as chapters need them.
3. **Signup:** open with email verification; self-host can disable.
4. **Database:** SQLite only; Postgres appendix.
5. **UI:** MPA with no framework; one optional vanilla JS file.
6. **Sync:** entity-level optimistic concurrency, not CRDT.
7. **Comments:** plain-text task comments + activity feed in v1.
8. **Private projects:** yes, with project membership; adds authz depth.
9. **Attachments/OAuth/2FA:** appendices or future work.
10. **Length:** 24 chapters + 5 appendices.

## 19. Definition of done

Every route authenticated where required, authorized, CSRF-protected,
validated, tested, accessible, and free of `unwrap` in request paths. Every
mutation writes an audit/activity record. Every background job is idempotent
and observable. Every backup is restorable. Every chapter ends with a runnable
demo and a failing test that proves the next behavior.
