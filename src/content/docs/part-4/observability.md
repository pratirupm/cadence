---
title: "Observability"
description: "Add tracing, counters, queue gauges, and a metrics endpoint."
---

A service that fails silently is not finished. The task API can be correct in
tests and still be impossible to operate: which route is slow, which login is
failing, how many reminders are stuck, which webhook receiver is down? Logs
answer some questions; counters answer others.

This chapter adds observability in the smallest useful form:

- structured `tracing` events for lifecycle and failures
- a request middleware that counts requests and 5xx responses
- login success/failure counters
- database gauges for pending and failed reminders and webhook deliveries
- a Prometheus text endpoint at `/metrics`

It does not add a tracing backend, a dashboard, or a distributed trace. Those
are deployment choices. The code emits the signals; the operator chooses where
they go.

By the end of this chapter you can:

- explain logs, metrics, and traces as different questions
- count requests and server errors without touching every handler
- count login outcomes at the point where the decision is made
- expose queue health as gauges instead of in-process counters
- scrape `/metrics` with `curl` and read the Prometheus format
- explain why per-process counters need a shared source
- identify the cardinality and privacy traps in metrics

The Rust lessons are atomics, middleware, `Arc` state, and async metric queries.
The operational lesson is that a metric is an aggregate with a stable name, not
a log line with a number in it.

## Three questions, three tools

| Tool | Question | Cost |
| --- | --- | --- |
| Logs | What happened to this request? | storage, search |
| Metrics | How many, how fast, how often? | cardinality, aggregation |
| Traces | Where did this request spend time? | sampling, propagation |

A log line is an event with fields. A metric is a number over time. A trace is a
tree of spans for one request. The service already has logs and a request span
from `TraceLayer`; this chapter adds metrics.

Do not turn a metric into a log: a counter named after a user ID is a
high-cardinality series, not an aggregate. Do not turn a log into a metric: a
message that says “slow” without a duration cannot be aggregated.

## The metrics type

`src/metrics.rs`:

```rust
#[derive(Debug, Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub logins: AtomicU64,
    pub login_failures: AtomicU64,
}

impl Metrics {
    pub fn inc(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render(&self) -> String {
        format!(
            concat!(
                "# HELP cadence_requests_total HTTP requests served.\n",
                "# TYPE cadence_requests_total counter\n",
                "cadence_requests_total {}\n",
                "# HELP cadence_errors_total HTTP errors served.\n",
                "# TYPE cadence_errors_total counter\n",
                "cadence_errors_total {}\n",
                "# HELP cadence_logins_total Successful logins.\n",
                "# TYPE cadence_logins_total counter\n",
                "cadence_logins_total {}\n",
                "# HELP cadence_login_failures_total Failed logins.\n",
                "# TYPE cadence_login_failures_total counter\n",
                "cadence_login_failures_total {}\n",
            ),
            self.requests.load(Ordering::Relaxed),
            self.errors.load(Ordering::Relaxed),
            self.logins.load(Ordering::Relaxed),
            self.login_failures.load(Ordering::Relaxed),
        )
    }
}
```

`AtomicU64` is enough for counters. `Relaxed` ordering is enough because the
only requirement is eventual visibility; no other data is synchronized by the
counter. Do not use a mutex for a counter; it serializes every request for no
reason.

`Metrics` is wrapped in `Arc` in `AppState` so clones share the same counters.
Without the `Arc`, each `AppState` clone would count separately.

The names are stable and prefixed with `cadence_`. A metric name is an API:
dashboards and alerts depend on it. Adding a label is easier than renaming a
series; changing a name silently breaks every query.

## Counting requests

The middleware is a layer around the router:

```rust
async fn count_requests(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: Next,
) -> Response {
    metrics.inc(&metrics.requests);
    let response = next.run(request).await;
    if response.status().is_server_error() {
        metrics.inc(&metrics.errors);
    }
    response
}
```

It runs for every request that reaches the router, including 404s. It counts
5xx, not 4xx: a 400 is a client error, not a server failure. A 401 burst is a
login problem, not an availability problem; the login counters cover it.

The layer is installed in `app`:

```rust
.layer(middleware::from_fn_with_state(
    state.metrics.clone(),
    count_requests,
))
```

`from_fn_with_state` gives the middleware its own state without making it part
of every handler. The clone is an `Arc`; all requests share the same counters.

The request count is incremented before `next.run`, so a panic or a timeout still
counts the attempt. The error count is after the response; a panic that aborts
the task may not return a response and may not increment it. A production
middleware uses a guard or a `Drop` implementation for the error path. The
exercise asks for a latency histogram, which needs the same start/end shape.

## Counting logins

The login handler increments at the decision:

```rust
match state.auth.login(&form.email, &form.password).await {
    Ok(session) => {
        state.login_limiter.clear(&form.email);
        state.metrics.inc(&state.metrics.logins);
        with_session(session, "/app")
    }
    Err(_) => {
        state.login_limiter.record_failure(&form.email);
        state.metrics.inc(&state.metrics.login_failures);
        render(LoginTemplate { error: "Invalid email or password.".to_owned() })
    }
}
```

Incrementing after the response is impossible; the handler owns the outcome. Do
not count “login attempts” by counting requests to `/login`; a GET is a form, a
POST is an attempt, and a rate-limited attempt is a third state.

The counter is not labeled by email. That would create one series per address and
leak user data into a metrics system. If you need per-user debugging, use a log
with an internal user ID and a retention policy, not a metric label.

## Queue gauges

In-process counters cannot see the worker process. `worker --once` runs in a
different process from `serve`; its counters are not in the server’s `Arc`.
Incrementing a `reminders` counter inside the worker would update a different
memory space.

The `/metrics` handler queries the database for queue health:

```rust
async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    let pending_reminders = count_rows(
        &state,
        "SELECT COUNT(*) FROM reminders WHERE status = 'pending'",
    ).await;
    let failed_reminders = count_rows(
        &state,
        "SELECT COUNT(*) FROM reminders WHERE status = 'failed'",
    ).await;
    let pending_webhooks = count_rows(
        &state,
        "SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'pending'",
    ).await;
    let failed_webhooks = count_rows(
        &state,
        "SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'failed'",
    ).await;

    let mut body = state.metrics.render();
    body.push_str(&format!(/* gauges */));
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}
```

`count_rows` logs a warning and returns 0 on a query error. A metrics scrape
should not fail because the database is briefly busy; a zero is misleading, but a
500 makes the whole scrape disappear. A production implementation exports a
`scrape_error` counter as well.

The gauges are:

```text
cadence_reminders_pending
cadence_reminders_failed
cadence_webhook_deliveries_pending
cadence_webhook_deliveries_failed
```

A gauge is a current value, not a cumulative count. `pending` can go down;
`failed` should only grow until a human retries or purges. Use the right type.

The queries run on every scrape. Four `COUNT(*)`s on small tables are cheap; on a
large table they need indexes. `reminders(status, remind_at)` and
`webhook_deliveries(status, next_at)` help. A busy production system might
maintain counters in the database or use a metrics table updated by workers.

## The endpoint

`GET /metrics` returns `text/plain; version=0.0.4`:

```console
$ curl -s http://127.0.0.1:3000/metrics
# HELP cadence_requests_total HTTP requests served.
# TYPE cadence_requests_total counter
cadence_requests_total 9
...
cadence_reminders_pending 0
cadence_reminders_failed 0
cadence_webhook_deliveries_pending 0
cadence_webhook_deliveries_failed 0
```

The endpoint is currently public. In production, restrict it to the private
network or put it behind the reverse proxy with a separate listener. Metrics can
reveal traffic patterns and queue health; they are not a public API.

The `HELP` and `TYPE` lines are part of the format. A scraper tolerates their
absence, but dashboards and documentation benefit from them. Keep the names and
help text stable.

## Logs and spans

`telemetry::init` installs a subscriber with an `EnvFilter` from `RUST_LOG`.
`TraceLayer` creates a span per request. The handlers log lifecycle events:

- startup address and shutdown
- account created
- email delivered to the log
- reminder due
- webhook delivery warning
- metrics query warning

The logs are structured fields, not formatted strings:

```rust
tracing::info!(address = %config.bind, "cadence listening");
tracing::warn!(%error, delivery = %id, "webhook delivery failed");
```

A log aggregator can index `address`, `error`, and `delivery` separately. A
formatted string cannot be filtered without a regex. Use fields for anything you
might query.

Do not log passwords, raw session tokens, API tokens, reset tokens, email bodies,
or full Authorization headers. The log mailer logs recipient and subject only.

A request ID is not implemented. `TraceLayer` gives a span, but no ID is returned
to the client or included in every log. Chapter 21’s security and production
work is the natural place to add it. The exercise asks for a UUID or ULID per
request.

## Metric catalog

| Name | Type | Source | Use |
| --- | --- | --- | --- |
| `cadence_requests_total` | counter | middleware | traffic rate, saturation |
| `cadence_errors_total` | counter | middleware | 5xx rate |
| `cadence_logins_total` | counter | login handler | auth success trend |
| `cadence_login_failures_total` | counter | login handler | attack or broken client |
| `cadence_reminders_pending` | gauge | DB query | backlog |
| `cadence_reminders_failed` | gauge | DB query | dead letters |
| `cadence_webhook_deliveries_pending` | gauge | DB query | receiver outage |
| `cadence_webhook_deliveries_failed` | gauge | DB query | dead letters |

The catalog is deliberately small. A dashboard with twenty metrics and no owner
is worse than four that someone checks. Add a metric when a decision depends on
it, not because the runtime exposes it.

`rate(cadence_requests_total[5m])` is traffic. `rate(cadence_errors_total[5m]) /
rate(cadence_requests_total[5m])` is the error ratio. The ratio is more useful
than the raw count because it survives a traffic spike.

## What not to measure

Do not put these in labels:

- user IDs, emails, task IDs, workspace IDs
- raw URLs with IDs
- error messages
- tokens or request bodies

Each unique combination is a time series. A per-task series is unbounded and
leaks data. Use logs for detail and metrics for aggregates.

Do not measure “number of tasks” as a request counter. If you need it, query the
database as a gauge with a clear cost. Do not turn a cheap log line into an
expensive scrape.

Do not alert on a gauge that is supposed to be zero only in a healthy system
without a duration. A single failed webhook is normal; a growing `failed` gauge
for an hour is not.

## What to alert on

A metric is only useful if someone acts on it. Reasonable first alerts:

- 5xx rate above a small threshold for five minutes
- login failure spike, which may be an attack or a broken client
- `cadence_reminders_failed` greater than zero for an hour
- `cadence_webhook_deliveries_failed` greater than zero
- `cadence_reminders_pending` growing without a matching send rate

Do not alert on `cadence_requests_total`; it only goes up. Alert on its rate or
on errors as a fraction of requests. Do not page on a single failed webhook; a
receiver outage is normal. Page on a sustained failure or a growing backlog.

## Tests and manual evidence

`renders_prometheus_text` increments requests and errors and asserts both series
appear with value 1. The test does not start a server; it tests the formatting.

The manual check starts the service, makes a request, and scrapes:

```console
$ curl -s http://127.0.0.1:3000/health > /dev/null
$ curl -s http://127.0.0.1:3000/metrics | grep cadence_requests_total
cadence_requests_total 1
```

The queue gauges are observed by creating a reminder or webhook delivery and
scraping again. The count changes; the label set does not.

## Checkpoint

```console
$ cargo test metrics
$ cargo run -- serve
$ curl -s http://127.0.0.1:3000/metrics | head
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

The checkpoint is a scrape with the expected names and a request count that
increases after a request.

## Failure modes you will meet

**Counters reset on restart.** Process-local counters start at zero. That is
normal for counters; dashboards use `rate()` and alerts tolerate a reset. Do not
store a counter in SQLite unless you need lifetime totals.

**Worker activity is invisible.** The worker is a separate process. Use database
gauges or a shared metrics service, not an in-process counter.

**Metrics endpoint is slow.** Four `COUNT(*)` queries per scrape can be costly.
Add indexes, cache the gauge for a few seconds, or maintain counters.

**A metric has one series per user.** A label such as `user_id` explodes
cardinality and leaks data. Aggregate; log the detail.

**A scrape returns 500.** A database error in the gauge query propagated. The
current handler returns 0 with a warning; a stricter handler might fail the
scrape. Decide and test.

**Logs contain a token.** The log mailer or a request log printed the body or the
Authorization header. Redact at the source; do not rely on a downstream filter.

**`/metrics` is public.** Restrict it at the proxy or bind a separate admin
listener. Public metrics are an information leak.

## Under the hood

`AtomicU64` with `Relaxed` is a single hardware atomic. On x86 it is a locked
add; on other architectures it is the cheapest atomic. A counter does not need
`SeqCst`; there is no ordering relationship with other data.

Middleware is a service that wraps the router. It sees the request before the
handler and the response after. The state is cloned per request; the `Arc` keeps
the counters shared.

Prometheus scrapes a text endpoint and stores samples. A counter is expected to
increase; a gauge goes up and down. The exporter does not compute rates; the
server does. Keep the endpoint cheap and stable.

## Exercises

1. **Add a latency histogram.** Measure from middleware entry to response and
   emit bucket counters for `cadence_request_duration_seconds`. Keep the label
   set small (method, route template, status class).
2. **Add request IDs.** Generate a ULID per request, include it in the tracing
   span, return it in a response header, and log it on errors.
3. **Add `/ready`.** Check the database with `SELECT 1` and return 503 with a
   short body when it fails. Keep `/health` as liveness.
4. **Add worker metrics.** Run the worker in the same process with a shared
   metrics registry, or write worker results to a `metrics` table that the
   endpoint reads. Compare the trade-offs.
5. **Define an SLO.** Pick one user-facing operation, a target (for example 99%
   under 300 ms), and an alert that fires when the error budget burns too fast.

## What comes next

Metrics tell you a request failed; they do not stop the next one. Chapter 21 adds
security headers, login throttling, CSRF review, and the authorization matrix
that keeps a failure from becoming a breach.
