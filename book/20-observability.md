# 20. Observability

**What you can do at the end.** See request counts, errors, login attempts,
reminder activity, and webhook deliveries in a Prometheus scrape.

**The Rust you learn.** Atomics, middleware, `tracing` spans, and exposing
operational state without leaking user data.

**Build it.**

- Add a `Metrics` struct with atomic counters for requests, errors, logins,
  login failures, reminders, and webhook deliveries.
- Add a `count_requests` middleware that increments requests and 5xx errors.
- Increment login success/failure in the login handler.
- Add `GET /metrics` returning Prometheus text.
- Keep `tracing` for structured logs and request IDs.
- Do not put email addresses or task titles in metrics.

The middleware sees every request:

```rust
async fn count_requests(State(metrics): State<Arc<Metrics>>, request: Request, next: Next) -> Response {
    metrics.inc(&metrics.requests);
    let response = next.run(request).await;
    if response.status().is_server_error() {
        metrics.inc(&metrics.errors);
    }
    response
}
```

**Checkpoint.**

```console
$ curl http://127.0.0.1:3000/metrics
$ cargo test metrics
```

The output contains `cadence_requests_total`, `cadence_errors_total`,
`cadence_logins_total`, `cadence_login_failures_total`,
`cadence_reminders_total`, and `cadence_webhook_deliveries_total`.

**Exercises.**

1. Add a latency histogram.
2. Add a build/version label to every metric.
