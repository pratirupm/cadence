# 1. The sequel

**What you can do at the end.** Start a service, ask for its health, and get a
clean error for anything else. The book-one app is now a network program.

**The Rust you learn.** Binaries, library roots, shared types, `Result` at the
process boundary, and `tracing` initialization. The health route is small; the
boundary is the lesson.

**Build it.**

- Create the `cadence` package with `cargo new cadence --vcs none`.
- Add `axum`, `tokio`, `serde`, `thiserror`, `tracing`, and `tracing-subscriber`.
- Split `lib.rs` from `main.rs` so the router can be tested without a socket.
- Add `Config` with a parsed `SocketAddr`, an `AppError` that renders JSON, and
  a `GET /health` route.
- Install a tracing subscriber in `main`, start a `TcpListener`, and wire
  graceful shutdown to Ctrl+C and SIGTERM.

The router is built in `AppState`-free form at this point:

```rust
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
}
```

**Checkpoint.**

```console
$ cargo run -- serve
$ curl -i http://127.0.0.1:3000/health
HTTP/1.1 200 OK
{"status":"ok"}
$ curl -i http://127.0.0.1:3000/missing
HTTP/1.1 404 Not Found
{"error":"not found"}
$ cargo test
```

The tests use `tower::ServiceExt::oneshot`, so they do not bind a port.

**Exercises.**

1. Make `--bind` accept an environment variable as well as a flag.
2. Add a `GET /version` route that returns the crate version, and test it.
