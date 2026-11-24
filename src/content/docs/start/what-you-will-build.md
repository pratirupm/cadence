---
title: What you will build
description: The Cadence feature set, architecture, and the checkpoints you reach.
---

Cadence is a multi-user task service. The domain is a todo app; the product is a
service with accounts, workspaces, recurring tasks, reminders, an API, a sync
client, and production operations.

## The product

A person can:

- sign up, verify an email, log in, and reset a password
- create a workspace, invite members, and assign roles
- create projects, including private projects with explicit members
- add tasks with notes, priority, due dates, and recurrence
- filter with a small query language: `due:today & priority:>2`
- complete a recurring task and get the next occurrence
- schedule a reminder and receive an email and in-app notification
- use a scoped token to call a JSON API idempotently
- sync a CLI cache, edit offline, and resolve a conflict
- see live updates in the browser
- register a signed webhook for task events
- operate the service with metrics, backups, migrations, and graceful shutdown

## The architecture

```text
browser / CLI / API client
        |
        v
   axum HTTP layer
   sessions, CSRF, tokens, SSE
        |
        v
   domain services
   auth, workspaces, tasks, query,
   recurrence, reminders, sync, webhooks
        |
        v
   SQLite (WAL, migrations, triggers)
        |
        +--> worker: reminders, email, webhooks
```

The code is a single package with modules, not a microservice fleet. That is a
deliberate teaching choice: one binary, one database, one place to see the
boundaries. The modules can become crates later.

## The checkpoints

| Part | You can |
| --- | --- |
| I | Serve health, store tasks, add accounts, workspaces, roles, tasks, and the browser MPA |
| II | Filter, recur, remind, notify, share private projects, and call the API |
| III | Pull changes, sync a CLI, resolve conflicts, and stream live events |
| IV | Sign webhooks, observe metrics, harden security, measure performance, deploy, and run the capstone |

Each chapter ends with a runnable checkpoint. The repository’s reference
implementation is the final state; the chapter shows the step that got there.

## What you need

- Rust stable (the book uses edition 2024)
- Cargo
- Git
- a terminal and a text editor
- `curl` for API and SSE checks
- a SQLite client is optional but useful for query plans

No database server, Docker, or cloud account is required for the chapters. Docker
appears in the deployment chapter as an optional packaging step.

## What you will not build

- a native mobile app
- a rich block editor
- a plugin marketplace
- a CRDT or a full offline browser
- a production email provider account
- a multi-region deployment

Those are named as next steps where they fit. The book’s scope is a complete,
tested, single-node service.
