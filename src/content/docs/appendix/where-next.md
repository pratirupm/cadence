---
title: Where next
description: What the book covered, what it deliberately left out, and what to build next.
---

Cadence is complete as a single-node task service. That is not the same as a
finished commercial product or a complete Rust education. This appendix is the
honest map of what remains.

## What the book covered

- axum, tokio, SQLx and SQLite
- sessions, CSRF, password hashing, API tokens
- workspaces, roles, private projects, invitations
- task versions, soft deletes, a filter DSL
- recurrence across time zones and DST
- reminders, retries, notifications, a mailer seam
- an API with scopes and idempotency
- a change log, a CLI sync client, conflict resolution
- SSE and a tiny live UI
- signed webhooks, metrics, security headers, backups, Docker

The app is a reference for “how these pieces fit,” not a library to copy
unchanged.

## What it deliberately did not cover

- generics and lifetimes in depth
- custom iterators and iterator-heavy code
- `Rc`/`Arc` trade-offs and `Rc<RefCell<_>>` patterns
- async internals, executors, `Pin`
- `unsafe`, FFI, and low-level memory
- macro authorship beyond a mention
- a full standard-library tour
- SQLite at multi-writer scale
- a production email provider and deliverability
- a real SSRF-hardened webhook sender
- a browser offline/CRDT client
- multi-region deployment

Those are the “missing half” from book one’s appendix. A third book can go
underneath this application.

## Production gaps in Cadence

These are named in chapters 23–24 and repeated here:

- no HSTS or `Secure` cookie flag in development
- no `/ready` database check
- no shared login/API rate limiter
- no lease/recovery for in-flight reminders or webhooks
- no external log/metrics backend or alerting
- no automated backup retention
- no request IDs in logs
- no task-list pagination
- no mock HTTP test for webhook delivery
- Docker image not built in the author’s environment
- SQLite single-writer scaling limit

Each is a bounded project. Do not call the service production-ready until the
ones that affect your threat model are closed.

## Projects to build next

1. **A proper offline client.** Add create/delete, local IDs, a browser service
   worker, and a sync protocol with tombstones. Compare entity-level conflict
   handling with a CRDT.
2. **A multi-tenant SaaS shell.** Add billing, plans, invitations, email
   verification delivery, and a hosted admin. Keep the tenant checks.
3. **A search service.** Build an inverted index, ranking, and a query language
   without SQLite FTS. This is the language-depth project from the planning
   conversation.
4. **A job platform.** Build the scheduler, workers, retries, and observability
   as the product, then expose an API.
5. **A local-first tool.** Files, Git, and a sync engine, no server required.
6. **The missing half.** Write the generics/lifetimes/iterators/unsafe/macros
   book with a parser, interpreter, or storage engine as the vehicle.

## How to keep improving this codebase

- Add a pagination test before adding pagination.
- Add a webhook mock server before changing retries.
- Add reminder and webhook lease recovery before scaling the worker.
- Add `/ready` before trusting a container orchestrator.
- Add an authorization matrix test when adding a route.
- Add a provider mailer and a test double, not a real network call in tests.
- Measure the query plan before adding an index.

## A last note on scope

A book is finished when a reader can reproduce the checkpoints, not when every
feature is built. Cadence reaches that bar. The remaining work is either a
different product (scale, billing, mobile) or a different book (language depth,
systems internals).

Keep the habit from book one: one behavior, one test, one commit.
