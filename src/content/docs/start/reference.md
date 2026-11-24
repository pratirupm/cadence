---
title: The reference source
description: Where the complete Cadence app lives, and how to use it beside the chapters.
---

The chapters show the decisions and the code that carries them. They do not
print every import, template, or helper. The complete application does exist, and
it is the second half of how this book is meant to be read.

## Where it is

The complete app is the `cadence-app/` directory in the book’s repository. If
you are reading the published site without a clone, the same tree is packaged
with the book:

[cadence-app.tar.gz](/reference/cadence-app.tar.gz)

Unpack it beside your work:

```console
$ tar -xzf cadence-app.tar.gz
$ cd cadence-app
$ cargo test
```

The chapters tell you to create a package named `cadence`; the archive directory
is `cadence-app/` because the repository also holds the site. The layout inside
the package is the same.

```console
$ git clone https://github.com/pratirupm/cadence.git
$ cd cadence/cadence-app
$ cargo test
```

The reference is **the final state of the app**, after all twenty-four chapters.
It is not a snapshot of each checkpoint. Early chapters describe a smaller
service; later chapters refactor it. When a snippet and the final file differ,
the chapter’s surrounding explanation says what changed and why.

## What is in it

| Path | What it holds |
| --- | --- |
| `src/main.rs` | The binary, subcommand dispatch, shutdown signal |
| `src/cli.rs` | The clap types and their parsing tests |
| `src/config.rs` | Bind-address parsing |
| `src/db.rs` | SQLite connection, migrations, transactions |
| `src/auth.rs` | Accounts, passwords, sessions, verification, reset |
| `src/workspaces.rs` | Workspaces, roles, projects, invitations, project members |
| `src/tasks.rs` | Tasks, authorization, versions, recurrence hooks |
| `src/query.rs` | The filter parser and SQL compiler |
| `src/recurrence.rs` | The RRULE subset and next-occurrence engine |
| `src/reminders.rs` | The reminder queue and worker |
| `src/notifications.rs` | The mailer seam and notification records |
| `src/tokens.rs` | API tokens and scopes |
| `src/idempotency.rs` | Stored responses for retried creates |
| `src/webhooks.rs` | Registration, signing, delivery, retries |
| `src/sync.rs` | The change log and batch sync endpoint |
| `src/web.rs` | The server-rendered MPA |
| `src/api.rs` | The JSON API |
| `src/error.rs` | Domain errors to HTTP responses |
| `templates/` | Askama layouts and pages |
| `migrations/` | Append-only schema files |
| `tests/` | HTTP, API, and web integration tests |

## How to use it

1. Read the chapter and type the code it prints.
2. Run the chapter’s checkpoint.
3. If something does not compile, compare the file with the reference and read
   the explanation around the snippet. The missing piece is usually an import,
   a helper introduced earlier, or a template named in prose.
4. Do not copy the final file into an early chapter. It contains services that
   have not been introduced yet, and copying it skips the lesson.

The reference is also the answer key for the exercises. Try the exercise first;
then compare. A diff you understand is worth more than a file you pasted.

## What the reference is not

It is not a production deployment. It runs on SQLite, has no external mail
provider wired, and leaves the gaps named in
[Deployment](/part-4/deployment/). It is a complete teaching implementation, not
a service you should expose to the internet without doing that chapter’s work.
