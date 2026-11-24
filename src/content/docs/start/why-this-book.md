---
title: Why this book
description: Who the book is for, what it assumes, and what changes from book one.
---

Book one ended with a command-line program that kept a todo list in a JSON file.
It was deliberately small: one behavior, one test, one commit. The habit was the
point, not the feature list.

This book keeps the habit and changes the boundary. A program that runs for a
second becomes a service that runs for weeks. A list with one owner becomes a
list with accounts, workspaces, roles, reminders, and conflicts. The task domain
stays familiar; the engineering does not.

## Who it is for

You should be comfortable with:

- opening a terminal, running commands, and editing files
- Rust ownership, borrowing, structs, enums, `Result`, and tests
- Cargo, `cargo test`, and `cargo fmt`
- Git basics: init, add, commit, status

You do **not** need experience with:

- async Rust, tokio, or axum
- SQL, SQLite, or SQLx
- web sessions, CSRF, or HTML templating
- background workers, queues, or deployment

If you finished book one, you have all the prerequisites. If you did not, the
appendix [Book one recap](/appendix/book-one-recap/) gives you the domain and the
style in a few pages.

## What changes

Book one had one process and one file. Book two has:

- a server process and a worker process
- a SQLite database shared by both
- a browser client and a JSON API
- a separate CLI with its own cache
- external email and webhook transports

Every new boundary is a place where a value can be stale, a request can be
retried, a user can be unauthorized, or a process can crash. The book names those
boundaries and tests them.

## What stays

- **One behavior at a time.** A chapter adds a contract, not a pile of files.
- **Tests before confidence.** If a claim matters, it has a test or a manual
  check with output.
- **Git as a record.** Commit after a working checkpoint.
- **Plain code.** No framework magic that the book cannot explain.
- **Production honesty.** The gaps are listed, not hidden.

## The promise

By the end you can build, test, and operate a multi-user Rust service with:

- sessions, CSRF, roles, and invitations
- a filter language and a recurrence engine
- reminders, notifications, webhooks, and retries
- an API with scoped tokens and idempotency
- a change log, a sync client, and conflict resolution
- metrics, security headers, migrations, backups, and a deployment

You will not have implemented tokio or an HTTP framework, and you should not. You
will have used them the way a working application does, and built the parts that
belong to the product.
