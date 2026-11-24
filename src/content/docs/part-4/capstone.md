---
title: "Capstone"
description: "Run the whole service through one end-to-end scenario."
---

The previous chapters built pieces. This chapter runs one scenario through all of
them: a fresh database, a browser user, a workspace, a project, a task, a
recurring series, a reminder, an API token, a CLI sync, a conflict, a webhook,
metrics, a backup, a restore, and a graceful stop.

The capstone is not a new feature. It is the moment you stop trusting the unit
tests alone and prove the pieces compose.

By the end of this chapter you can:

- run the service from an empty database
- create a user and use the browser flows
- prove recurrence through the task tests
- deliver a reminder through the worker
- use the API with a scoped token and idempotency
- sync a CLI cache and resolve a conflict
- register a webhook and explain its delivery path
- scrape metrics, back up, restore, and stop cleanly
- write a short incident runbook for the failures you have seen

The Rust lesson is integration: the same services are called by the browser, the
API, the worker, and the CLI. The operational lesson is that “the tests pass” is
not “the service is ready.”

## The scenario

Start from a clean working directory and a database file that does not exist:

```console
$ cd cadence-app
$ rm -f /tmp/cadence-capstone.db
$ cargo run -- migrate --database /tmp/cadence-capstone.db
INFO cadence: migrations applied database=/tmp/cadence-capstone.db
```

Create a verified account:

```console
$ cargo run -- user create \
    --database /tmp/cadence-capstone.db \
    --email ada@example.com \
    --name Ada \
    --password "correct horse battery staple" \
    --verified
01J... ada@example.com
```

Start the server and the worker against the same file:

```console
$ cargo run -- serve --bind 127.0.0.1:3000 \
    --database /tmp/cadence-capstone.db
INFO cadence: cadence listening address=127.0.0.1:3000
```

In another terminal:

```console
$ cargo run -- worker --database /tmp/cadence-capstone.db
```

The worker loops every 30 seconds. For a deterministic demo, use `worker --once`.

## The browser path

Open `http://127.0.0.1:3000/`.

1. Log in as `ada@example.com`.
2. Create a workspace named `Home`.
3. Create a project named `Chores`.
4. Add a task `buy milk`.
5. Toggle it done.
6. Add `water plants`.

The page redirects after each POST and re-renders the selected workspace/project.
The session cookie is `HttpOnly`, and every form carries the session CSRF token.
The task list shows open tasks before done ones, newest first.

Disable JavaScript and repeat one add/toggle. It still works. The live script is
an enhancement, not the path.

## The recurring series

There is no recurrence form in the browser yet. The service is covered by tests:

```console
$ cargo test completing_a_recurring_task_creates_the_next_occurrence
$ cargo test completing_twice_does_not_duplicate_the_next_occurrence
```

The first test sets `FREQ=DAILY`, completes the task, and asserts two rows: the
completed occurrence and an open successor with the next day’s due time. The
second asserts a repeated completion does not duplicate the successor.

This is a capstone gap by design: the engine is present, the UI is not. A real
next step is a due-date/recurrence form that calls `set_recurrence`.

## The reminder

Create a reminder at a past instant so the worker picks it up immediately:

```console
$ cargo run -- reminder create \
    --database /tmp/cadence-capstone.db \
    --email ada@example.com \
    --task-id <task-id> \
    --at 2020-01-01T00:00:00Z
01J... 2020-01-01T00:00:00+00:00

$ cargo run -- worker --once --database /tmp/cadence-capstone.db
INFO cadence::mailer: email delivered to the log
    to=ada@example.com subject=Reminder: buy milk
sent 1 reminder(s) and 0 webhook(s)
```

The worker claimed the row, created an in-app notification, called the mailer, and
marked the reminder sent. The `LogMailer` writes the email to the log; a provider
adapter is the production replacement.

A second `worker --once` sends nothing; the reminder is `sent`.

## The API

Create a read/write token:

```console
$ cargo run -- token create \
    --database /tmp/cadence-capstone.db \
    --email ada@example.com \
    --name capstone \
    --scopes read,write
01J... cad_...
```

The raw token is shown once. Use it:

```console
$ export TOKEN='cad_...'
$ curl -s -H "Authorization: Bearer $TOKEN" \
    http://127.0.0.1:3000/api/v1/me
{"scopes":["read","write"],"user_id":"..."}
```

List the project’s tasks:

```console
$ curl -s -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:3000/api/v1/workspaces/<ws>/projects/<project>/tasks"
```

Create a task with an idempotency key:

```console
$ curl -s -X POST \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -H "Idempotency-Key: capstone-1" \
    -d '{"title":"buy oat milk"}' \
    "http://127.0.0.1:3000/api/v1/workspaces/<ws>/projects/<project>/tasks"
```

Repeat the exact request. The response body is identical and the list still has
one task. Change the title with the same key: it returns `409`.

Use a read-only token for the same POST: it returns `403`. The scope is a gate;
the workspace role is still checked.

## The CLI sync

Create a local cache and sync:

```console
$ cargo run --bin cadence-cli -- \
    --local /tmp/cadence-local.db \
    --server http://127.0.0.1:3000 \
    --token "$TOKEN" sync
pulled N change(s) to cursor N

$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db list
  01J...  buy milk  (v2)
```

Rename locally and push:

```console
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db \
    rename 01J... "buy oat milk"
$ cargo run --bin cadence-cli -- --local /tmp/cadence-local.db \
    --server http://127.0.0.1:3000 --token "$TOKEN" sync
pulled 0 change(s) to cursor N
pushed task-01J...-v2-...
```

A second cache pulls the new title. To force a conflict, edit the same task in the
browser while the first cache is dirty, then sync. The CLI prints a conflict and
`conflicts` lists both sides. `resolve <id> --keep local` rebases the local edit;
the next sync pushes it as a new version.

## The webhook

Register a webhook with an admin-scoped token and a public HTTPS endpoint:

```console
$ curl -s -X POST \
    -H "Authorization: Bearer $ADMIN_TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"url":"https://example.com/cadence","events":["task.created"]}' \
    "http://127.0.0.1:3000/api/v1/workspaces/<ws>/webhooks"
{"id":"...","url":"https://example.com/cadence","secret":"...","events":["task.created"]}
```

The secret is shown once. The list endpoint omits it. Create a task through the
web/API path; `dispatch` queues a delivery, and the worker posts it with
`x-cadence-signature: sha256=...`.

A public example endpoint will not verify the signature; it will still receive
the request. For a deterministic check, use a receiver you control and verify the
HMAC over the raw body. The delivery path has no automated mock test yet; this is
a known gap.

## Metrics

```console
$ curl -s http://127.0.0.1:3000/metrics | grep cadence_
cadence_requests_total ...
cadence_errors_total ...
cadence_logins_total ...
cadence_login_failures_total ...
cadence_reminders_pending ...
cadence_reminders_failed ...
cadence_webhook_deliveries_pending ...
cadence_webhook_deliveries_failed ...
```

Make a request and scrape again. The request counter increases. Create a reminder
and do not run the worker; `cadence_reminders_pending` increases. Run the worker;
it decreases.

The endpoint is public in this deployment. Restrict it at the proxy.

## Backup and restore

```console
$ cargo run -- backup \
    --database /tmp/cadence-capstone.db \
    --output /tmp/cadence-capstone-backup.db
backup written to /tmp/cadence-capstone-backup.db
```

Restore into a fresh file and start a second server on another port:

```console
$ cp /tmp/cadence-capstone-backup.db /tmp/cadence-restored.db
$ cargo run -- serve --bind 127.0.0.1:3001 \
    --database /tmp/cadence-restored.db
```

Log in and confirm the task list is present. The backup is a consistent snapshot;
the WAL files are not part of it.

## Shutdown

Send SIGTERM to the server:

```console
$ kill -TERM <pid>
INFO cadence: shutdown signal received
```

The listener stops accepting; in-flight requests finish; the process exits with
success. The worker has the same signal path. A container stop uses the same
mechanism; set a stop timeout longer than a normal request.

## Verification checklist

| Step | Expected |
| --- | --- |
| migrate | “migrations applied” |
| user create | ID + email |
| browser login | app page, session cookie |
| create workspace/project/task | redirect, row visible |
| recurrence tests | 2 tests pass |
| reminder create | ID + instant |
| worker --once | 1 reminder, email log, notification |
| API /me | user + scopes |
| idempotent create | same body, one row |
| read-only write | 403 |
| CLI sync | cursor, pulled/pushed counts |
| conflict resolve | new version accepted |
| webhook create | secret once, list omits it |
| metrics | counters and gauges |
| backup | non-empty file |
| restore | login + tasks visible |
| SIGTERM | graceful log, exit 0 |

If a row fails, stop and fix it before declaring the service ready. The checklist
is the capstone.

## Observed run

The checklist is not theoretical. A clean run produced this shape:

```text
INFO cadence: migrations applied database=/tmp/capstone.db
01J... ada@example.com
login=303
01J... 2020-01-01T00:00:00+00:00
INFO cadence::mailer: email delivered to the log
    to=ada@example.com subject=Reminder: buy milk
sent 1 reminder(s) and 0 webhook(s)
{"scopes":["read","write"],"user_id":"01J..."}
cadence_requests_total 10
cadence_reminders_pending 0
cadence_reminders_failed 0
backup written to /tmp/capstone-backup.db
INFO cadence: shutdown signal received
```

Your IDs and counts differ. The lines that must match are the state transitions:
migration applied, login redirect, reminder sent, token scopes, gauges, backup,
shutdown.

The backup restored into a second database and served `/health`; the original
process exited on SIGTERM. The restore did not prove every row, but it proved the
file is a standalone database the binary can open.

## Failure injection drills

A capstone is also a chance to see failure deliberately:

| Drill | Action | Expected |
| --- | --- | --- |
| DB locked | hold a write transaction, create a task | request waits, then errors or succeeds within timeout |
| Worker down | stop worker, create reminder | pending gauge grows, no email |
| Duplicate create | same idempotency key twice | one row, same body |
| Stale edit | push old base version | conflict result, no overwrite |
| Webhook down | point at a 500 receiver | pending, retries, failed after five |
| Expired session | delete session row, use cookie | redirect to login |
| Bad CSRF | post form with wrong token | 403, no mutation |
| Disk full | fill the volume | writes fail with an error, no corruption |

Run one drill per feature, not all at once. The point is to see the failure mode
the chapter described, not to break the demo.

## Incident runbook

**Database is locked.** Check for a long worker transaction, a second process,
or a backup. Keep writes short; retry after the lock; do not delete the WAL.

**Reminder backlog grows.** Check `worker` is running, `cadence_reminders_pending`
is increasing, and the mailer/provider is healthy. A failed mailer retries with
backoff; a permanently failed row is a dead letter.

**Webhook receiver is down.** Deliveries stay pending and retry. After five
failures they are `failed`. The receiver should be idempotent; the sender may
duplicate on a lost response.

**Migrations fail.** The old binary is still running if you used a separate
migrate step. Restore the pre-migration backup if the schema is half-applied; do
not edit the migration checksum.

**Session key or token leaked.** Revoke API tokens, delete sessions, rotate the
database if a webhook secret leaked. The audit log names membership changes, not
tokens; use the token table for revocation.

**Disk full.** SQLite returns write errors; the service should log and fail
requests, not corrupt. Free space, checkpoint the WAL, and retry. A full disk can
also break the backup; monitor it.

## Postmortem template

After an incident, write:

- what users saw
- the timeline (first symptom, detection, mitigation, recovery)
- the root cause, not the proximate trigger
- which test or alert would have caught it
- the smallest fix and the follow-up work

A postmortem that says “human error” is incomplete. The system allowed the error
to reach users; find the missing check.

## What “finished” means

The service is finished as a book project when:

- the tests pass on a clean checkout
- `cargo fmt` and strict clippy pass
- the capstone checklist passes
- the known gaps are written down
- a backup restores into a fresh database
- a new reader can follow the chapters and reach the same checkpoints

It is not finished as a commercial product. The gaps in chapter 23 are real:
HSTS/Secure, readiness, shared rate limits, worker leases, external metrics,
provider email, pagination, and SQLite scale. Naming them is part of the work.

## Exercises

1. **Add the recurrence form.** Set a due date and an RRULE from the task page,
   call `set_recurrence`, and show the next occurrence after completion.
2. **Add a members page.** List workspace members, invite by email, change roles,
   and show the audit entries.
3. **Add a webhook test receiver.** A local test-only validator plus a mock server
   that verifies the HMAC and returns 500 once, then 200. Assert retry.
4. **Add readiness.** `/ready` checks `SELECT 1`; the Compose health check uses it.
5. **Run the capstone on a clean machine.** Use only the book and the repository;
   write down every place the instructions were ambiguous.

## The next book

If you continue, the missing half from the first book is still there: generics,
lifetimes in depth, custom iterators, `Rc`/`Arc` trade-offs, async internals,
`unsafe`, macros, and a broader standard library. This book gave you the
application shape. A third book can go underneath it.

For now, stop the servers, commit the capstone notes, and leave the working tree
clean. The habit from book one still applies: one behaviour, one test, one
commit.
