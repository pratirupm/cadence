# 22. Performance

**What you can do at the end.** Measure the feed and task list, add the indexes
the queries need, and explain why each index exists.

**The Rust you learn.** Query plans, N+1 detection, `EXPLAIN QUERY PLAN`,
criterion-style thinking, and measuring before changing.

**Build it.**

- Add indexes for `(workspace_id, completed, due_at)`,
  `(project_id, completed, created_at)`, session expiry, and webhook delivery.
- Run `EXPLAIN QUERY PLAN` for the task list and filter queries.
- Remove an N+1 by joining tags or loading them in one query.
- Add a small benchmark or load script for the list endpoint.
- Record p95 latency and compare before and after.
- Do not optimize the recurrence loop before measuring it.

A useful plan names the index it uses:

```text
SEARCH tasks USING INDEX tasks_project_status_idx (project_id=?)
```

**Checkpoint.**

```console
$ cargo test
$ cargo run -- serve
$ oha --duration 10s http://127.0.0.1:3000/health
```

The checkpoint is a measured baseline, not a claim. Put the numbers in the
chapter or the runbook.

**Exercises.**

1. Add a paginated feed query and show its plan.
2. Add a benchmark for the filter DSL with 10,000 tasks.
