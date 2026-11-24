---
title: "The sequel"
description: "Start the Cadence service, add a health endpoint, and learn the process boundary."
---

Book one ended with a command-line program that kept a list in a file. It was
small on purpose. One failing test at a time, one behaviour at a time, one
commit at a time. The point was never the todo list. The point was the habit:
write the behaviour down, watch it fail, make it pass, clean up, save the step.

Book two keeps the habit and changes the boundary. The same task domain now has
to survive other people. It has to remember accounts, workspaces, permissions,
time zones, retries, and what happens when two clients edit the same task. A
program that runs once and exits becomes a service that keeps running while
nobody is watching.

That is a large jump, so this chapter takes the smallest possible step across
it. You will build a process that can start, report that it is alive, report a
clean error for everything else, and stop without dropping work on the floor.
No database. No login. No tasks yet. Just the front door.

By the end of this chapter you can:

- start a web service from a command
- ask it for its health and get a JSON answer
- get a JSON 404 for an unknown route
- see a readable error when the bind address is wrong
- stop the process with Ctrl+C or SIGTERM and see a shutdown message
- test the HTTP layer without opening a socket

The methods are `axum`, `tokio`, `tower`, and `tracing`. The Rust lessons are
smaller and more important: where a library ends and a binary begins, how a
`Result` crosses the process boundary, why errors become responses, and why a
test should not need a port.

## The new shape

Book one had one flow:

```text
argument -> command -> list -> print
```

Book two has a boundary in the middle:

```text
client -> HTTP request -> router -> handler -> response
```

The client can be a browser, `curl`, the CLI from book one, or a background
worker. The service cannot assume the caller is friendly, patient, or even
still connected. That is why the first checkpoint is not “add tasks.” It is
“prove the process can answer.”

A health endpoint looks trivial. It is the first place where deployment,
monitoring, and tests agree on a contract. Load balancers ask it. Container
orchestrators ask it. You will ask it every time a deployment goes wrong.

## Create the package

Create a package named `cadence` inside the repository:

```console
$ cargo new cadence --vcs none
$ cd cadence
```

The `--vcs none` flag is deliberate. The repository already has Git history;
Cargo should not create a second one. Book one did the same thing.

The checkpoint needs a small set of dependencies:

```toml
[package]
name = "cadence"
version = "0.1.0"
edition = "2024"
default-run = "cadence"

[dependencies]
axum = "0.8"
clap = { version = "4", features = ["derive", "env"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["macros", "net", "rt-multi-thread", "signal"] }
tower-http = { version = "0.6", features = ["trace"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
http-body-util = "0.1"
tower = { version = "0.5", features = ["util"] }
```

Two choices are worth explaining.

`edition = "2024"` matches the first book and the current compiler. Editions are
not a style preference; they change a few language defaults and tell Cargo which
rules the code expects.

`default-run = "cadence"` becomes useful later, when the package grows a second
binary for the sync client. At this checkpoint there is only one binary, so the
line is harmless and prevents a future ambiguity.

Do not add a database or a template engine yet. Dependencies are decisions, and
this chapter has only earned the decisions above.

## Write the first failing test

The first behaviour is a contract, not an implementation:

> `GET /health` returns `200 OK` with the JSON body `{"status":"ok"}`.

Write it before the router exists. Create `tests/health.rs`:

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use cadence::app;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json, serde_json::json!({ "status": "ok" }));
}
```

Run it:

```console
$ cargo test --test health
```

The first failure is a compile error, because `cadence::app` does not exist.
That is the same kind of failure as book one's missing `heading` function. The
compiler is telling you the shape of the next small step.

Three details in the test matter for the rest of the book.

`#[tokio::test]` starts a small async runtime. The test function is an async
function because the router is async. You do not need to understand the runtime
internals yet; you need to know that an async test is how you call async code.

`oneshot` comes from `tower`, the middleware library underneath axum. It sends
one request through a service and returns one future. It does not bind a socket,
so the test is fast and cannot fail because a port is busy. This is the testing
habit for the whole book: test the router, not the network, unless the network is
the behaviour.

`BodyExt::collect` drains the response body into bytes. `serde_json` then turns
those bytes into a `Value`. The test compares values, not formatted text, so
whitespace and key order cannot make a good response fail.

Add the second behaviour before the implementation, because it is part of the
same contract:

```rust
#[tokio::test]
async fn unknown_route_returns_json_not_found() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/missing")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json, serde_json::json!({ "error": "not found" }));
}
```

Now run the test again. It still fails to compile, but for one reason: the
application does not exist yet.

## Configuration is a type

A command-line flag is a string. The server needs a socket address. Convert at
the edge, not deep inside the handler.

Create `src/config.rs`:

```rust
use std::net::SocketAddr;

use thiserror::Error;

/// Settings needed to start the HTTP server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
}

impl Config {
    /// Parse a bind address supplied by the command line.
    pub fn new(bind: &str) -> Result<Self, ConfigError> {
        let bind = bind
            .parse()
            .map_err(|_| ConfigError::InvalidBind(bind.to_owned()))?;
        Ok(Self { bind })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("invalid bind address: {0}")]
    InvalidBind(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_socket_address() {
        let config = Config::new("127.0.0.1:4000").unwrap();

        assert_eq!(config.bind, "127.0.0.1:4000".parse().unwrap());
    }

    #[test]
    fn rejects_an_invalid_bind_address() {
        let error = Config::new("not-an-address").unwrap_err();

        assert_eq!(error, ConfigError::InvalidBind("not-an-address".to_owned()));
    }
}
```

The important line is the `parse()` call. `SocketAddr` parsing accepts IPv4 and
IPv6, including `[::1]:3000`. The error keeps the original text so the process
can say exactly what was wrong.

This `Config` is deliberately tiny. It has no `Default` implementation, because
there is no defensible default for “where should this service listen?” The CLI
owns the default string; the type owns validity.

The unit tests run without a runtime. There is no async here. When a type is
pure, test it as a pure type.

## Errors become responses

A handler returns `Result`. The error side has to become an HTTP response. If
every handler invented its own JSON error shape, clients would need a different
parser for every route.

Create `src/error.rs`:

```rust
use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Errors that can become an HTTP response.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
        };
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
```

`IntoResponse` is the conversion trait axum uses. A handler can return a value
that implements it, or a `Result` whose error implements it. The framework does
the rest.

`ErrorBody` borrows a `&'static str`. The messages in this chapter are fixed
strings, so there is no allocation and no chance of leaking a database error to a
client. Later chapters add variants for validation, authentication, conflicts,
and internal errors. The public shape stays the same: a status code and a small
JSON body.

Notice what is missing. There is no `unwrap`, no panic, and no `String` error
message. A panic in a handler becomes a 500 with a generic body and a log line,
but it also aborts the normal shutdown path for that request. An explicit error
type keeps the failure at the boundary where you can test it.

## The router

Create `src/lib.rs`:

```rust
pub mod cli;
pub mod config;
pub mod error;
pub mod telemetry;

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::error::AppError;

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

/// Build the HTTP application.
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn not_found() -> AppError {
    AppError::NotFound
}
```

`Router::new()` is an empty routing table. `.route` adds one method-and-path pair.
`.fallback` handles every path that did not match. Without it, axum returns its
own plain-text 404; with it, the service has one error shape.

`Health` is a struct, not a `serde_json::json!` macro call. The struct is
smaller, typed, and reusable. `Serialize` derives the JSON encoding. A reference
to a `'static` string is enough here. If the health response later includes a
version or a database check, the struct grows and the tests stay the same shape.

`TraceLayer` is middleware from `tower-http`. It logs requests and responses and
adds timing information through `tracing`. It does not change the body or the
status. At this stage it is the first example of a layer: a value that wraps the
router and observes or modifies requests on the way through.

The `app` function returns `Router`, not a server. That separation is what makes
the test possible. A server needs a socket; a router needs only a request.

## The binary

Create `src/cli.rs`:

```rust
use clap::{Parser, Subcommand};

/// The Cadence task service.
#[derive(Debug, Parser)]
#[command(name = "cadence", version, about = "A multi-user task service")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Command {
    /// Start the HTTP server.
    Serve {
        /// Address to listen on.
        #[arg(long, default_value = "127.0.0.1:3000")]
        bind: String,
    },
}
```

`clap` turns the struct into a parser. The doc comments become help text. The
`Serve` variant owns the flag value as a `String`; `Config` owns the parsed
`SocketAddr`. The CLI does not know what a socket is, and the config does not
know what a flag is. Each type stays at its own level.

`#[arg(long)]` means `--bind`. The default is loopback and port 3000. Loopback
is the safe default: the service is not reachable from another machine until you
choose a public address.

Create `src/telemetry.rs`:

```rust
use tracing_subscriber::EnvFilter;

/// Install the process-wide tracing subscriber.
pub fn init() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
```

`EnvFilter` reads `RUST_LOG`, so a deployment can raise or lower the level
without rebuilding. The fallback is `info`, which shows startup and shutdown but
not per-request detail.

`init` calls `.init()` once for the process. That is why tests do not call it:
two tests initializing the same global subscriber would race. The binary owns
process-wide setup; the library stays testable.

Create `src/main.rs`:

```rust
use std::process::ExitCode;

use cadence::cli::{Cli, Command};
use cadence::config::Config;
use cadence::{app, telemetry};
use clap::Parser;

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    telemetry::init();

    match cli.command {
        Command::Serve { bind } => {
            let config = Config::new(&bind)?;

            let listener = tokio::net::TcpListener::bind(config.bind).await?;
            tracing::info!(address = %config.bind, "cadence listening");

            axum::serve(listener, app())
                .with_graceful_shutdown(shutdown_signal())
                .await?;
        }
    }

    Ok(())
}
```

This is the first place the two error worlds meet. `main` returns an
`ExitCode`, which the operating system understands. `run` returns
`Box<dyn std::error::Error>`, because it may fail in several unrelated ways:
parsing, binding, or serving. The `?` operator converts each concrete error into
the boxed trait object. The caller prints one line and exits non-zero.

The `#[tokio::main]` attribute turns the async `main` into a small runtime
setup. Without it, you would have to build a runtime by hand. The attribute is
not magic to memorize; it expands to code that starts the runtime, blocks on the
future, and shuts the runtime down.

The listener is bound before the server starts. If the port is already in use,
the error appears before any request is accepted, and the process exits. That is
better than starting, logging “listening,” and failing on the first connection.

`axum::serve(listener, app())` takes ownership of the listener and the router.
`with_graceful_shutdown` wraps the server in a future that stops accepting new
connections when the signal resolves. In-flight requests get a chance to finish.
The exact drain timeout is a production concern; the hook is here from the first
chapter so it is never bolted on later.

`shutdown_signal` waits for Ctrl+C and, on Unix, SIGTERM:

```rust
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
```

`tokio::select!` waits for whichever future finishes first. The `cfg` attribute
keeps the Unix-only API out of a Windows build; on other platforms the second
future never completes. This is the first conditional compilation in the book,
and it is a small one: the shutdown contract is the same on every platform, but
the operating system’s signal name is not.

The two `expect` calls are deliberately allowed here. They run during process
setup, not inside a request. If the runtime cannot install a signal handler, the
process cannot honour its shutdown contract, and failing loudly is correct. In a
handler, the same call would be a bug.

## Run it

Start the service:

```console
$ cargo run -- serve
```

The log line contains the address. Ask for health:

```console
$ curl -i http://127.0.0.1:3000/health
HTTP/1.1 200 OK
content-type: application/json
content-length: 15

{"status":"ok"}
```

Ask for a route that does not exist:

```console
$ curl -i http://127.0.0.1:3000/missing
HTTP/1.1 404 Not Found
content-type: application/json
content-length: 21

{"error":"not found"}
```

Try a bad address:

```console
$ cargo run -- serve --bind not-an-address
error: invalid bind address: not-an-address
```

Stop the process with Ctrl+C. The final log line is:

```text
INFO cadence: shutdown signal received
```

## The tests, and what they do not prove

Run the full suite:

```console
$ cargo test
```

At this checkpoint there are seven tests: two for config, three for the CLI,
and two for HTTP. The CLI tests are worth adding even though `clap` is well
tested. They document the contract you own:

- `serve` defaults to `127.0.0.1:3000`
- `--bind 0.0.0.0:8080` is accepted
- an unknown subcommand is rejected

The HTTP tests prove the router without a socket. They do not prove that the
server binds, that the process handles SIGTERM, or that the log line appears.
Those are integration concerns, and the manual run above covers them once. Do
not turn every manual check into a flaky test. Test the contract at the cheapest
layer that can express it.

There is one more thing the tests do not prove: that a client is authorized.
There is no auth yet. `/health` is public on purpose. Every other route will
have to decide.

## Failure modes you will meet

**`Address already in use`.** Another process owns the port. Choose another
`--bind`, or stop the old process. Do not “fix” this by retrying forever in a
loop; a port conflict is a configuration error, not a transient network error.

**`error: invalid bind address`.** The string is not an IP address and port.
IPv4 uses `127.0.0.1:3000`; IPv6 uses `[::1]:3000`. A hostname such as
`localhost:3000` is not a `SocketAddr`; resolve it to an address before binding,
or accept a hostname in a later config layer.

**The process starts but `curl` cannot connect.** Check the bind address. The
default is loopback, which is not reachable from another machine. `0.0.0.0:3000`
listens on every IPv4 interface, with the security consequences that implies.

**`404` has the wrong content type.** The fallback is the only 404 in this
chapter. If a later route returns its own error, it must use `AppError` or an
equivalent `IntoResponse` implementation. A raw string returns `text/plain`.

**Ctrl+C does nothing.** The process may be running an old binary, or the
signal may be handled by a supervisor. Check the build, then check the process
tree. Do not add a timeout before confirming which process received the signal.

**A panic in a handler.** Axum turns it into a 500 for that request, but the
panic is still a bug. The error type exists so ordinary failures never take that
path.

## Under the hood

You do not need to read these crates to finish the chapter, but knowing what
each layer owns prevents false mental models.

`tokio` owns the async runtime: the executor, timers, TCP, and signals. It does
not know what HTTP is.

`hyper` owns HTTP/1.1 and HTTP/2 parsing and connection management. It turns
bytes into requests and responses.

`axum` owns routing, extractors, and response conversion. It maps a request to
your handler and your return value back to a response.

`tower` owns the middleware abstraction. `TraceLayer` and the later metrics and
security layers are services that wrap other services.

`tracing` owns structured events and spans. `TraceLayer` emits spans; your
handlers emit events. A log line is an event with fields, not a formatted string.

The boundaries matter because each layer can be tested separately. The router
test exercises axum and your handler. It does not exercise hyper’s socket code
or tokio’s signal handling. That is why the manual run still exists.

## Exercises

1. **Add `/version`.** Return `{"version": env!("CARGO_PKG_VERSION")}`. Add a
   router test that asserts the exact version. Then add a test that checks the
   content type is `application/json`.
2. **Reject an empty bind.** Change `Config::new` so `""` returns a distinct
   error variant. Add a test. Decide whether the CLI should catch it before
   `Config` or let the type do it.
3. **Add a second route with a path parameter.** Add `GET /hello/{name}` and
   return `{"hello":"name"}`. Test a normal name and a name with a space. What
   does axum do with an unmatched path?
4. **Make shutdown testable.** Extract the signal future behind a trait or a
   channel, then write a test that sends a shutdown message and asserts the
   server future completes. Do not use a real signal in the test.
5. **Break the contract on purpose.** Change the health body to a plain string,
   run the tests, and read the failure. Restore the JSON. The point is to see
   which assertion catches the change: status, body, or content type.

## Checkpoint

You are done with chapter 1 when all of these are true:

```console
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
$ cargo test
$ cargo run -- serve
$ curl -i http://127.0.0.1:3000/health
$ curl -i http://127.0.0.1:3000/missing
```

The output matches the contract above, and Ctrl+C produces the shutdown log.

Commit the checkpoint with a message that names the behaviour, not the files:

```text
Add the service health check
```

## What comes next

The router currently has no state. Chapter 2 adds SQLite, migrations, and a task
repository. That changes `app()` to `app(state)`, because handlers need access to
the database pool. The health route stays small; the state travels beside it.

The habit does not change. Write the next contract, watch it fail, make it pass,
and commit the smallest step that is true.
