# 10. Reminders

**What you can do at the end.** Schedule a reminder and have a worker deliver
it, retry failures, and give up after five attempts.

**The Rust you learn.** Background workers, claiming rows, at-least-once
delivery, backoff, idempotency, and `tokio::time`.

**Build it.**

- Add a reminders table with `status`, `attempts`, `remind_at`, and `sent_at`.
- `create` verifies the actor can see the task and the target is a member.
- `claim_due` selects pending rows in a transaction and marks them `sending`,
  incrementing attempts before the network call.
- `mark_sent` is the only success transition.
- `mark_failed` retries with `attempts²` minutes of backoff, or marks `failed`
  at five attempts.
- Add `cadence worker --once` for a single batch and `cadence worker` for a
  loop.

A claim is a state transition:

```text
pending -> sending -> sent
                   -> pending (retry)
                   -> failed (after five)
```

**Checkpoint.**

```console
$ cargo test reminders
$ cargo run -- worker --once
```

The tests cover claiming once, marking sent, retry backoff, the five-attempt
limit, outsider rejection, and email plus in-app delivery together.

**Exercises.**

1. Add a test with paused tokio time.
2. Add quiet hours per user and test a reminder inside and outside them.
