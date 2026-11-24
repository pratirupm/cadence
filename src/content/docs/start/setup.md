---
title: Set up your workbench
description: Install Rust, Git, an editor, and the command-line tools the book uses.
---

The book needs a Rust toolchain, Git, a terminal, and a text editor. The rest is
optional.

## Rust

Install Rust with `rustup` from <https://rustup.rs>. The stable toolchain is
enough; the book uses edition 2024.

Check the install:

```console
$ rustc --version
rustc 1.99.1 (...)
$ cargo --version
cargo 1.99.1 (...)
$ rustup component list --installed
cargo-x86_64-unknown-linux-gnu
clippy-x86_64-unknown-linux-gnu
rust-std-x86_64-unknown-linux-gnu
rustc-x86_64-unknown-linux-gnu
rustfmt-x86_64-unknown-linux-gnu
```

The exact version may be newer. You need `cargo`, `rustc`, `rustfmt`, and
`clippy`. If `clippy` or `rustfmt` is missing:

```console
$ rustup component add clippy rustfmt
```

## Git

Install Git from your package manager or <https://git-scm.com>. Set a name and
email so commits work:

```console
$ git config --global user.name "Your Name"
$ git config --global user.email "you@example.com"
$ git --version
git version 2.x
```

The book commits after each working checkpoint. A local repository is enough; no
remote account is required.

Create the repository and the files Git should ignore before writing any code:

```console
$ mkdir -p ~/code/cadence-book
$ cd ~/code/cadence-book
$ git init
$ printf 'target/\ncadence.db*\n.env\n' > .gitignore
```

The package you create in chapter 1 will live inside this directory. `target/`
matches at any depth, so the build directory is ignored wherever Cargo puts it.

## Editor

Any editor works. The book uses Zed in examples, but the commands are plain
`cargo` and `git`. If you use VS Code, install `rust-analyzer`; if you use Zed,
the Rust extension brings it in.

Useful editor behavior:

- format on save with `rustfmt`
- run clippy on save, or a clippy task
- show inlay type hints if they help
- a terminal pane for `cargo test`

The book’s code is plain text. Do not install a Rust framework extension that
generates code; the point is to type it.

## Command-line tools

You need:

- `curl` for API and SSE checks
- `git`
- `cargo`

Optional:

- a SQLite client (`sqlite3`) for `EXPLAIN QUERY PLAN` and inspecting the file
- `oha`, `wrk`, or `k6` for the load test in the performance chapter
- Docker for the deployment chapter
- `jq` for pretty-printing JSON

Check:

```console
$ curl --version | head -1
$ sqlite3 --version 2>/dev/null || echo "sqlite3 not installed; optional"
$ docker --version 2>/dev/null || echo "docker not installed; optional"
```

The chapters do not require Docker until deployment, and the deployment chapter
can be read without running the image.

## A scratch directory

Create a parent directory for the book’s project:

```console
$ mkdir -p ~/code/cadence-book
$ cd ~/code/cadence-book
```

The first chapter creates the `cadence` package here. The book’s repository is a
reference; do not clone it into the same folder and skip the steps.

## Verify the toolchain

Before chapter 1, run:

```console
$ rustc --version
$ cargo --version
$ cargo fmt --version
$ cargo clippy --version
$ git --version
$ curl --version | head -1
```

If any command is missing, install it now. A missing tool is easier to fix before
the first test than in the middle of a migration.

## Network and ports

The service binds `127.0.0.1:3000` by default. Check that the port is free:

```console
$ curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/health || true
```

A connection failure is expected before the server exists. If something already
answers on 3000, choose another port with `--bind 127.0.0.1:3001` in the
chapters.

The email chapter uses `LogMailer` by default; no SMTP account is needed. A real
provider is a production exercise, not a prerequisite.

## What not to install

- a database server: the book uses SQLite, a library
- a frontend framework or Node build: the UI is server-rendered
- a Rust web framework other than the one the chapter introduces
- an ORM: the book uses SQL directly

Keep the workbench small. The fewer moving parts, the easier it is to see the
Rust.
