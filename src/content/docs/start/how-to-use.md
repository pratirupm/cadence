---
title: How to use this book
description: The chapter shape, conventions, commands, and what to do when a step fails.
---

Every chapter follows the same shape:

1. **What you can do at the end.**
2. **The Rust you learn.** The concepts the chapter forces.
3. **Build it.** The smallest sequence that produces the behavior.
4. **Tests and evidence.** The contract the checkpoint must satisfy.
5. **Failure modes.** The errors you will meet and what they mean.
6. **Exercises.** One or two changes that make the idea yours.

## Read with the code open

The chapters show short excerpts, not the whole repository. Keep
`cadence-app/src/` open beside the page. When a snippet omits error handling or
imports, the reference has the full version. [The reference source](/start/reference/)
names the repository, the file map, and how to compare early chapters with the
final app.

The reference implementation is the final state of the app. Early chapters show
the checkpoint as it existed then; later chapters refactor it. If a snippet does
not match the final file, check the chapter’s context and the surrounding
explanation. The book is cumulative, not a file listing.

## Run the checkpoint

Each chapter ends with commands. Run them before moving on:

```console
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo test
```

A chapter is not done because you read it. It is done when the tests pass and the
manual check produces the described behavior.

## Code conventions

- Types and error enums come before impls and free functions.
- Tests live at the bottom of the module they test.
- Integration tests live in `tests/` and drive the HTTP router or binary.
- Public behavior has a test; timing and external transports have a named manual
  check.
- `unwrap` is allowed in tests and process setup, not in request paths.
- SQL values are bound; field names and operators are whitelisted.

## Commands

All Rust commands run from `cadence-app/`:

```console
$ cd cadence-app
$ cargo test
$ cargo run -- serve
$ cargo run -- migrate
$ cargo run -- worker --once
```

The CLI is a second binary:

```console
$ cargo run --bin cadence-cli -- --help
```

The book shows `cadence` and `cadence-cli` after an install to keep examples
short; `cargo run --` during development is equivalent.

## When a step fails

1. Read the first error, not the last.
2. Check the chapter’s failure-mode section.
3. Compare the exact command and file path.
4. Run the focused test.
5. If the book is wrong, write down the expected and actual output; that is a
   book bug, not a reader bug.

The reference implementation is tested against the checkpoints. If your error
differs from the printed one and the difference is not one the book allows
(line numbers, hashes, timings), treat it as a defect.

## Notation

- `...` in output means “some lines omitted.”
- Paths in output may differ; the shape should match.
- Hashes, timestamps, and IDs are examples.
- `INFO`, `WARN`, and `ERROR` lines are `tracing` output.
- Terminal blocks show commands with `$` and output without it.
- A line with `// ...` in Rust is an elision, not code to type.

## What not to do

- Do not copy the final repository and skip the chapters; the explanations are
  the point.
- Do not add a dependency before the chapter introduces it.
- Do not “fix” a deliberate failure before reading the next paragraph.
- Do not skip the tests because the code compiles.
- Do not deploy the service with the development defaults.
