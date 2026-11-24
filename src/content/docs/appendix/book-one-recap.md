---
title: Book one recap
description: The CLI app, domain, test style, and Git habits that book two assumes.
---

Book one built a terminal todo app one failing test at a time. This appendix is
the minimum you need from it. If you finished the book, this is a reminder; if
you arrived from another Rust tutorial, this is the domain and the style.

## The app

The final CLI stored tasks in `.todo.json` and answered commands:

```text
add buy milk
list
done 1
undone 1
remove 2
edit 1 buy oat milk
find milk
priority 1 high
due 1 2026-12-01
help
quit
```

It split into a library (`src/lib.rs`) and a binary (`src/main.rs`) so the logic
could be tested without a terminal.

## The domain

The core types were a task and a list:

```rust
pub struct Todo {
    pub title: String,
    pub done: bool,
    pub priority: Priority,
    pub due: Option<NaiveDate>,
}

pub struct TodoList {
    todos: Vec<Todo>,
}
```

The exact fields grew across the book. The important shape is:

- a task owns its title
- completion is a boolean state
- priority is an enum, not an integer
- a due date is optional
- the list owns the tasks and exposes methods

Book two replaces the CLI file storage with SQLite and adds workspace/project IDs
and a `version`, but the data modeling habit is the same.

## The test style

A behavior starts as a failing test:

```rust
#[test]
fn adds_a_todo() {
    let mut list = TodoList::new();
    list.add("buy milk");
    assert_eq!(list.len(), 1);
}
```

Then the smallest implementation makes it pass. The test stays as the contract.
Tests live at the bottom of the module; integration tests drive the binary or the
router.

Book two keeps this style at every layer: unit tests for pure logic, repository
tests against an in-memory database, HTTP tests with `tower::ServiceExt::oneshot`,
and manual checks for transports.

## Git style

- commit after a working checkpoint
- message in the present tense, naming the behavior
- do not commit `target/`
- do not commit local data (`.todo.json`, `*.db`)
- use a branch when an idea is risky

Book two adds migrations and generated files to the ignore list, but the commit
habit is the same.

## Rust concepts assumed

You should recognize:

- ownership, moves, and borrows
- `&str` versus `String`
- `Option` and `Result`
- `match` and `if let`
- structs, enums, `impl`, and traits
- `Vec`, iteration, and closures
- `#[test]`, `assert_eq!`, and `#[cfg(test)]`
- modules, `pub`, and `use`
- Cargo dependencies and `cargo test`

You do **not** need:

- async/await or tokio
- SQL or SQLx
- HTML or web sessions
- background workers
- deployment

Those are introduced with a contract and a checkpoint in this book.

## What book two changes

| Book one | Book two |
| --- | --- |
| one process | server + worker |
| JSON file | SQLite + migrations |
| one user | accounts, workspaces, roles |
| in-memory list | repositories and versions |
| terminal output | MPA + JSON API |
| no network | HTTP, SSE, webhooks |
| local only | deployable container |

The domain is still a task list. The boundary is the new material.
