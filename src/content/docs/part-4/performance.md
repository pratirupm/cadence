---
title: "Performance"
description: "Read query plans, add indexes, and measure a real workload."
---

The service works. The next question is whether it works at the size the product
claims. A task list with ten rows is not evidence; a list with ten thousand rows
and a filter is. Performance work starts with a measurement, not a hunch.

This chapter is about the measurement you can do with the code you have: indexes,
query plans, pagination, connection pools, and a small load test. It does not
promise a benchmark number. It shows how to get one and what to change when the
plan is wrong.

By the end of this chapter you can:

- read the performance indexes and explain each one
- run `EXPLAIN QUERY PLAN` for the task and reminder queries
- identify an N+1 query and the three-query page load
- explain keyset versus offset pagination and the limits already in place
- run a simple HTTP load test against a local server
- choose a target and compare before/after
- identify the costs of SQLite’s single writer and the metrics scrape

The Rust lessons are query plans, indexes, and the difference between a correct
query and a fast one. The systems lesson is that performance is a property of a
workload, not of a line of code.

## Measure first

A useful loop:

1. Pick one user-visible operation: task list, task create, sync page, reminder
   batch.
2. Define the workload: rows, concurrency, filter, page size.
3. Record a baseline: latency, throughput, query plan.
4. Change one thing.
5. Record the same baseline.
6. Keep the change only if it helps the chosen operation.

Without step 1, “make it faster” becomes a tour of micro-optimizations. Without
step 5, the change is a belief.

The repository does not ship a benchmark result. Machine speed, SQLite version,
and row distribution change the number. It ships the schema and the commands to
produce one on your machine. Put your numbers in the chapter or the runbook; do
not copy someone else’s.

## The indexes

`migrations/0012_performance_indexes.sql`:

```sql
CREATE INDEX IF NOT EXISTS tasks_workspace_status_due_idx
    ON tasks (workspace_id, completed, due_at);

CREATE INDEX IF NOT EXISTS tasks_project_status_idx
    ON tasks (project_id, completed, created_at);

CREATE INDEX IF NOT EXISTS sessions_expires_at_idx
    ON sessions (expires_at);

CREATE INDEX IF NOT EXISTS webhook_deliveries_webhook_idx
    ON webhook_deliveries (webhook_id, created_at);
```

Each index serves a query shape:

- `tasks_project_status_idx` supports “open tasks in this project, newest first.”
  The list query filters `project_id` and `deleted_at IS NULL`, orders by
  `completed, created_at DESC`. The index covers the first two columns and the
  order prefix.
- `tasks_workspace_status_due_idx` supports workspace-wide “what is due” views.
  The current UI scopes to a project; the index is for the upcoming cross-project
  view and for due reminders.
- `sessions_expires_at_idx` supports cleanup and expiry scans. A session lookup
  uses the primary key, not this index.
- `webhook_deliveries_webhook_idx` supports “deliveries for this webhook” in a
  log view. The worker uses the `(status, next_at)` index from chapter 19.

`IF NOT EXISTS` makes the migration idempotent but hides a duplicate. The
`sessions_expires_at_idx` was already created in chapter 3 (accounts); this migration’s
line is a no-op. That is harmless, but it is also a sign that a later reader
cannot tell which index is authoritative. Prefer naming the original migration
or using a new index name if the definition changes.

An index is not free. It costs writes: every insert/update maintains the B-tree.
Do not add an index for a query you do not run. The exercise asks you to remove
one and measure the write cost.

## Reading a plan

SQLite explains a query with:

```sql
EXPLAIN QUERY PLAN
SELECT id, title, completed
FROM tasks
WHERE project_id = 'p1' AND deleted_at IS NULL
ORDER BY completed, created_at DESC, id DESC
LIMIT 100;
```

The output names the access path. A useful plan looks like:

```text
SEARCH tasks USING INDEX tasks_project_status_idx (project_id=?)
```

The exact text varies by version and statistics. What matters:

- `SCAN tasks` means a full table scan.
- `SEARCH ... USING INDEX` means an index was chosen.
- `USE TEMP B-TREE FOR ORDER BY` means SQLite is sorting; an index that matches
  the order can avoid it.
- A plan that uses an index for the filter but still sorts may be worse than a
  covering index for small pages.

Run the plan with the same predicates as the query. A plan for
`WHERE project_id = ?` without the `ORDER BY` tells you less than the real
statement.

You can run the plan through `sqlx` in a test or a small `cadence plan` command;
the book uses a SQL client for clarity. The schema is the contract; the plan is
the evidence.

## The page load query count

`GET /app` performs:

1. session lookup (`sessions` by primary key + `users` by primary key)
2. workspaces for the user (join membership)
3. projects for the selected workspace (visibility + project membership)
4. tasks for the selected project (filter + order)
5. maybe a task count or a notification count, if added

That is a fixed number of queries, not N+1. The page does **not** query each
project for its tasks or each task for its labels. The view structs are built
from the lists already loaded.

An N+1 appears when a template calls a function that queries per row, or when a
handler loops over projects and loads tasks for each. Rust makes this less
accidental than an ORM lazy loader, but it is still possible. Watch for a query
inside a `for` loop over a list.

The audit and notification lists are bounded by a limit. They do not load the
whole table.

## Pagination

The task API takes `?filter=` but no limit; the sync endpoint and the notification
inbox take a limit. The task list is unpaginated in both the API and the MPA; it
renders every task in a project. That is fine for a small project and wrong for a
large one.

Two pagination shapes:

- **Offset:** `LIMIT ? OFFSET ?`. Simple, but a deep offset scans and discards
  rows, and inserts can shift the page.
- **Keyset:** `WHERE (created_at, id) < (?, ?) ORDER BY ... LIMIT ?`. Stable under
  inserts and uses the index, but the cursor must encode both columns.

The sync cursor is a keyset on `seq`. The task list should use a keyset on
`(created_at, id)` if it grows. The exercise asks for it.

Do not use `LIMIT` without an `ORDER BY` that the index supports; the “first 100”
may be arbitrary and change between requests.

## Connection pool and WAL

`db::connect` uses a pool of five connections for a file database, WAL mode,
foreign keys, and a busy timeout. WAL lets readers proceed during a write; it
does not allow two writers. SQLite serializes writers, so a long write
transaction blocks other writes.

Practical rules:

- keep write transactions short; do not hold one across a network call
- do not put a slow webhook delivery inside a task transaction
- use one writer for maintenance tasks (migrations, vacuum) or accept the wait
- set a busy timeout so a concurrent write waits instead of failing instantly
- monitor `database is locked` as a symptom, not a normal condition

The worker and the web process share the same file in this deployment. A busy
worker batch can delay a web write. The claim/dispatch transactions are short for
that reason.

## A small load test

With the server running:

```console
$ oha --duration 10s --concurrency 20 http://127.0.0.1:3000/health
$ oha --duration 10s --concurrency 20 \
    -H "Authorization: Bearer $TOKEN" \
    "http://127.0.0.1:3000/api/v1/workspaces/$WS/projects/$PROJECT/tasks"
```

`oha` is one option; `k6`, `wrk`, or a small Rust client work too. Record:

- requests per second
- p50/p95/p99 latency
- error rate
- CPU and memory
- database size and WAL growth

Health is not the task list. A health endpoint that returns a constant tells you
the runtime is up, not that the query path is fast. Test the route you care
about with realistic rows.

For the write path, a load test that creates tasks needs unique titles and a
token with write scope. A burst of writes will hit the single-writer limit;
that is the result, not a failure of the test.

## Targets

Pick targets before measuring:

- task list p95 under 200 ms on a small VM with 10,000 tasks in one project
- sync pull p95 under 300 ms for 100 changes
- reminder batch of 50 under 2 seconds excluding network delivery
- webhook batch of 50 under 2 seconds excluding receiver latency

These are starting points, not universal laws. The point is to write them down
and compare. A target with no workload is a wish.

## Query budget

A useful discipline is a per-request query *shape*:

| Request | Logical stages |
| --- | --- |
| `GET /health` | constant response, no query |
| `GET /metrics` | four queue gauge queries |
| `GET /app` | session, workspaces, projects, tasks |
| `GET /api/.../tasks` | token, project access, tasks |
| `POST /tasks` | token, workspace/project authorization, insert, event |
| `GET /sync` | token, membership-scoped changes |

The number of stages is fixed. The number of rows is not. If the query count
grows with the number of projects or tasks, you have an N+1.

A query budget is a review tool: a pull request that adds a query per task is
visible in the shape before it is visible in production. Count the queries with
tracing or a test wrapper if you want exact numbers; do not copy estimates from
a table.

## Baseline template

Record measurements in a small table so the comparison is honest:

```text
Machine: 4 vCPU, 8 GB, local NVMe
SQLite:  3.x
Rows:    10,000 tasks, 50 projects, 5 workspaces
Route:   GET /api/v1/workspaces/<ws>/projects/<p>/tasks
Command: oha --duration 10s --concurrency 20 -H "Authorization: Bearer ..."
Before:  p50 18 ms, p95 41 ms, 420 req/s
After:   p50 11 ms, p95 24 ms, 610 req/s
Change:  added tasks_project_status_idx
```

The numbers are illustrative; replace them with yours. The schema and command
make the result reproducible. A number without a workload is marketing.

## Tests and manual evidence

Performance tests are not part of `cargo test` in this book. The schema and
queries are covered functionally; the timing is not. That is a deliberate scope
choice: a CI timing test is flaky on shared runners, and a benchmark needs a
stable machine.

Manual evidence:

```console
$ cargo run -- serve
$ curl -s http://127.0.0.1:3000/metrics | grep cadence_requests_total
$ oha --duration 10s --concurrency 20 http://127.0.0.1:3000/health
```

The metrics counters show request volume; `oha` shows latency. The queue gauges
show whether a worker is keeping up.

Do not claim a performance result the repository does not contain. Put your
measurement in the runbook with the machine, row count, and command.

## Checkpoint

```console
$ cargo test
$ cargo run -- serve
$ oha --duration 10s --concurrency 20 http://127.0.0.1:3000/health
$ sqlite3 cadence.db "EXPLAIN QUERY PLAN SELECT ..."
```

The checkpoint is a baseline you can reproduce, not a number copied from a blog.

## Failure modes you will meet

**A full table scan on the task list.** The predicate or order does not match an
index. Check the plan; add or reorder an index deliberately.

**An index exists but is not used.** The leading column is wrong, the query uses
a function on the column, or the statistics are stale. Run `ANALYZE` after a
large data change and re-check.

**The page is slow only with many projects.** The handler loops and queries per
project. Load the projects and tasks in one query or batch.

**Deep pagination gets slower.** Offset scans. Switch to keyset.

**Writes time out.** A long transaction or too many writers. Keep transactions
short; do not hold one across a webhook or email.

**The metrics scrape is slow.** Four `COUNT(*)` queries on large tables. Add
indexes, cache, or maintain counters.

**The load test saturates before the database does.** The runtime or the Argon2
login path is the bottleneck. Measure the route separately; do not “fix” the
database for a login benchmark.

**A benchmark improves but p95 worsens.** The average hides a tail. Keep the
percentile you defined; do not switch metrics after the fact.

## Under the hood

SQLite uses B-tree indexes. A composite index is ordered by its columns; a query
can use a prefix of the index for equality and the next column for ordering. An
index on `(project_id, completed, created_at)` helps a query that filters on
`project_id` and orders by `completed, created_at`. An index on
`(created_at, project_id)` does not help the same query as much.

A covering index contains every column the query reads. SQLite can answer from
the index without visiting the table. It is wider and costs more writes; add it
only for a hot query.

WAL writes to a separate log and checkpoints into the main file. Readers see a
consistent snapshot. The `-wal` file can grow during a long read; monitor it.

## Exercises

1. **Measure the task list.** Create 10,000 tasks in one project, run the list,
   and record p50/p95 before and after adding a covering index.
2. **Keyset paginate the MPA.** Add `?after=created_at,id` and a “next” link.
   Test that inserts do not duplicate or skip a row.
3. **Find an N+1.** Add a label count to the task list first with a per-task
   query, measure it, then replace it with one grouped query. Compare.
4. **Remove an index.** Drop one index in a test database, measure the write
   improvement, and restore it. Which query got slower?
5. **Load test the write path.** Create tasks concurrently and record the
   single-writer behavior. Decide whether the product needs a queue or a
   different database.

## What comes next

Indexes and query plans are only useful if the service runs. Chapter 23 packages
the binary, migrates the database, backs it up, and restores it. A fast query on
a service that cannot start is not a performance win.
