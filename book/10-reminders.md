# 10. Reminders

The task knows when it is due. The recurrence engine can create the next
occurrence. Neither of them can wake up at 08:55 and send an email. A reminder is
a delivery obligation with its own state: it must be claimed once, attempted,
and either marked sent or retried.

This chapter builds the scheduler. It is deliberately separate from the task and
from the recurrence engine. A task can exist without a reminder. A reminder can
point at a task that is already done. A scheduler can run in a different process
from the web server. Keeping these concerns separate is what makes each one
testable.

By the end of this chapter you can:

- create a reminder only for a task the actor can see and a user in the workspace
- claim due reminders without two workers claiming the same row
- send an email and write an in-app notification
- retry a failed delivery with backoff and stop after five attempts
- run one batch with `worker --once` and a loop with `worker`
- explain at-least-once delivery and where duplicate notifications can happen
- identify the “stuck in sending” failure after a crash

The Rust lessons are transactions, state transitions, claim protocols, backoff
arithmetic, and dependency injection through a mailer trait. The systems lesson
is the difference between “due,” “claimed,” and “sent.”

## The reminder row

Create `migrations/0006_reminders.sql`:

```sql
CREATE TABLE reminders (
    id TEXT PRIMARY KEY NOT NULL,
    task_id TEXT NOT NULL REFERENCES tasks (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    remind_at TEXT NOT NULL,
    channel TEXT NOT NULL DEFAULT 'email' CHECK (channel IN ('email', 'in_app')),
    sent_at TEXT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sending', 'sent', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX reminders_due_idx ON reminders (status, remind_at);
CREATE INDEX reminders_task_id_idx ON reminders (task_id);
```

A reminder belongs to one task and one user. It has its own instant, channel,
status, attempt count, and error. The `remind_at` column is what the worker
scans; the task’s `due_at` is separate. A user may want a reminder ten minutes
before the due time, on the due time, or at a custom time.

`status` is the state machine:

```text
pending -> sending -> sent
                   -> pending   (retry)
                   -> failed    (after five attempts)
```

`pending` means eligible. `sending` means a worker claimed it and is responsible
for a terminal transition. `sent` and `failed` are terminal. The worker never
picks up `sending` rows; a crash while sending therefore requires a separate
recovery policy, which this chapter does not implement. That gap is real and is
called out in the failure modes.

`attempts` increments when the row is claimed, not when it fails. That makes the
claim itself the record of an attempt. `mark_failed` uses the count to decide
between retry and terminal failure.

The index `(status, remind_at)` supports the due query. `(task_id)` supports
“what reminders does this task have?” in a future UI.

## Creating a reminder

```rust
pub async fn create(
    &self,
    actor_id: &str,
    task_id: &str,
    user_id: &str,
    remind_at: DateTime<Utc>,
) -> Result<Reminder, ReminderError> {
    let task = TaskService::new(self.pool.clone())
        .get(actor_id, task_id)
        .await?;
    let workspaces = WorkspaceService::new(self.pool.clone());
    if workspaces.role(&task.workspace_id, user_id).await?.is_none() {
        return Err(ReminderError::Forbidden);
    }

    // insert pending reminder
}
```

There are two authorization checks:

- `TaskService::get(actor_id, task_id)` ensures the actor can see the task and
  its project.
- `WorkspaceService::role(...).is_none()` ensures the target user is a member of
  the workspace.

Why can the actor and the target differ? A workspace admin might schedule a
reminder for a member. The current UI creates reminders for yourself, but the
service allows the admin case. If you do not want that, require `actor_id ==
user_id` and test it.

The reminder is always `channel = "email"` in the current service. The schema
allows `in_app`, but the sender always writes both an email and an in-app
notification. A real product would read the channel and the user’s preferences;
that is chapter 11’s seam.

`remind_at` is accepted as an instant. The service does not reject a past time;
the row is immediately due. For a user-facing form, validate “future” at the
edge, because a reminder in the past is usually a mistake. Tests deliberately use
`now` so the claim path runs without waiting.

## Claiming due reminders

`claim_due` is the heart of the scheduler:

```rust
pub async fn claim_due(
    &self,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<Reminder>, ReminderError> {
    let mut transaction = self.pool.begin().await?;
    let mut reminders = sqlx::query_as::<_, Reminder>(
        "SELECT ... FROM reminders
         WHERE status = 'pending' AND remind_at <= ?
         ORDER BY remind_at, id
         LIMIT ?",
    )
    .bind(now)
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await?;

    for reminder in &mut reminders {
        sqlx::query(
            "UPDATE reminders
             SET status = 'sending', attempts = attempts + 1
             WHERE id = ?",
        )
        .bind(&reminder.id)
        .execute(&mut *transaction)
        .await?;
        reminder.status = "sending".to_owned();
        reminder.attempts += 1;
    }

    transaction.commit().await?;
    Ok(reminders)
}
```

The select and the updates are in one transaction. A second worker that starts
after the commit sees no `pending` rows. A second worker that starts before the
commit waits on SQLite’s writer lock, then sees the new statuses. This is the
claim protocol.

The returned rows have `status = "sending"` and `attempts` already incremented.
The caller must not treat the return as “sent”; it is “owned by this worker until
a terminal transition.”

The `ORDER BY remind_at, id` makes a backlog deterministic. `LIMIT` bounds the
batch so one worker cannot hold the transaction for a thousand rows. The worker
uses 50.

`claim_due` does not filter by channel or user preferences. Every due row is
claimed. A channel-aware scheduler would add `AND channel = ?` or claim by
channel; this chapter keeps one queue.

## Terminal transitions

`mark_sent` only accepts a row in `sending`:

```rust
UPDATE reminders SET status = 'sent', sent_at = ?
WHERE id = ? AND status = 'sending'
```

If zero rows change, it returns `NotFound`. A row that is already `sent`, still
`pending`, or `failed` cannot be marked sent by a late worker. The status
condition is a small compare-and-set.

`mark_failed` reads the row, then:

- if `attempts >= 5`, set `failed` and store the error;
- otherwise set `pending`, move `remind_at` forward by `attempts²` minutes, and
  store the error.

```rust
let delay = Duration::minutes(reminder.attempts * reminder.attempts);
```

The first failure waits 1 minute, the second 4, the third 9, the fourth 16. The
fifth attempt happens after the fourth backoff; `attempts` is already 5 when it
fails, so it becomes `failed`. The exact constants are a policy. What matters is
that the retry is bounded and the delay grows.

`mark_failed` does not check `status = 'sending'` in its `WHERE` clause. It reads
any row by ID and updates it. A stricter version adds the status condition, like
`mark_sent`. The current code assumes the caller is the claimant. That assumption
is worth tightening; the exercise asks for it.

## Running a batch

`run_once_with` takes the mailer and notification service as arguments:

```rust
pub async fn run_once_with(
    &self,
    mailer: &dyn Mailer,
    notifications: &NotificationService,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<usize, ReminderError> {
    let reminders = self.claim_due(now, limit).await?;
    let count = reminders.len();

    for reminder in reminders {
        let detail = sqlx::query_as::<_, (String, String)>(
            "SELECT t.title, u.email
             FROM reminders r
             JOIN tasks t ON t.id = r.task_id
             JOIN users u ON u.id = r.user_id
             WHERE r.id = ?",
        )
        .bind(&reminder.id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((title, email)) = detail else {
            self.mark_failed(&reminder.id, now, "task or user disappeared").await?;
            continue;
        };

        let payload = serde_json::json!({
            "task_id": reminder.task_id,
            "title": title,
        }).to_string();

        notifications.create(&reminder.user_id, "reminder", &payload).await?;

        let message = EmailMessage {
            to: email,
            subject: format!("Reminder: {title}"),
            text: format!("Your task \"{title}\" is due for a reminder."),
        };

        if let Err(error) = mailer.send(&message).await {
            self.mark_failed(&reminder.id, now, &error.to_string()).await?;
        } else {
            self.mark_sent(&reminder.id).await?;
        }
    }

    Ok(count)
}
```

The return value is the number of claimed reminders, not the number successfully
sent. The caller can log “claimed 3” even if one failed and will retry. If you
want a precise sent/failed pair, return a small report struct. The CLI currently
prints the claimed count, which is honest enough for a batch log.

The default `run_once` wires `LogMailer` and a fresh `NotificationService`. Tests
call `run_once_with` with `RecordingMailer` so they can assert the message.

`run_forever` is the loop:

```rust
let mut ticker = tokio::time::interval(Duration::from_secs(30));
loop {
    ticker.tick().await;
    if let Err(error) = service.run_once(Utc::now(), 50).await {
        tracing::error!(%error, "reminder worker failed");
    }
}
```

The loop owns one service and one pool. It logs errors and keeps going. A
production worker adds graceful shutdown, a jittered interval, metrics, and a
lease timeout for `sending` rows. Chapter 23 covers shutdown for the main
binary; the worker uses the same signal path in `main`.

## At-least-once delivery

The sequence is:

```text
claim -> send -> mark sent
```

If the process crashes after `send` and before `mark sent`, the row is stuck in
`sending` and the user may have received the email. If a recovery job later
returns it to `pending`, the user may receive it twice. That is at-least-once
delivery. Exactly-once delivery across an email provider is not a property this
system can guarantee; the provider’s idempotency key might help, but the local
state can still be ambiguous.

The same issue appears with the in-app notification. `notifications.create`
happens before `mailer.send`. If the email fails and the reminder is retried, a
second notification is created. The user sees duplicates. A robust design uses a
delivery record keyed by `(reminder_id, channel, attempt)` and an upsert, or
creates the notification only after the email succeeds. Neither is free:
success-first loses the notification if the process dies after the email;
record-first duplicates on retry. Decide which failure you prefer and test it.

The current chapter is honest about at-least-once. It does not claim exactly-once.

## The worker command

`cadence worker --once` runs one batch and exits:

```console
$ cargo run -- worker --once
sent 1 reminder(s) and 0 webhook(s)
```

`cadence worker` runs the reminder loop and the webhook loop together under one
shutdown signal. The two loops are independent; a slow webhook does not block a
reminder because they are separate futures.

For local development, `--once` is the useful mode: create a reminder at a past
instant, run the batch, and inspect the log. Do not sleep in a test to wait for a
30-second tick.

## Worked retry trace

A reminder that keeps failing follows this path:

| Claim | Attempts after claim | `mark_failed` result | Next `remind_at` |
| --- | ---: | --- | --- |
| 1 | 1 | pending | now + 1 minute |
| 2 | 2 | pending | now + 4 minutes |
| 3 | 3 | pending | now + 9 minutes |
| 4 | 4 | pending | now + 16 minutes |
| 5 | 5 | failed | terminal |

The `attempts` value is incremented by the claim, so `mark_failed` sees the
attempt that just failed. The fifth failure is terminal because `attempts >= 5`.
The test advances the clock by 30 minutes before each claim so every backoff is
due; production would simply wait.

The first attempt is not delayed by a retry. A new reminder at `now` is due
immediately. That is what `--once` relies on.

## Why claim before send

It is tempting to send first and mark the row after. That has a simpler success
path but a worse failure path: two workers can both send the same reminder before
either marks it. Claiming first makes ownership explicit. The cost is the stuck
`sending` state after a crash.

A common production compromise is a short lease:

```text
claim -> sending with locked_until = now + 2 minutes
send
mark sent
```

A recovery pass returns expired leases to pending. If the send took longer than
the lease, two workers can still race, so the provider should receive an
idempotency key. There is no free exactly-once.

## Tests

Four tests cover the scheduler:

- `claims_due_reminders_once`: first claim returns one row with status `sending`
  and attempts `1`; a second claim returns nothing; marking sent keeps it out.
- `retries_with_backoff_then_gives_up`: five claim/fail cycles, advancing the
  clock 30 minutes each time; statuses are `pending` for the first four and
  `failed` on the fifth.
- `sends_email_and_in_app_notification_together`: a `RecordingMailer` captures
  one message and the user has one unread notification.
- `outsiders_cannot_create_reminders`: a non-member gets a task access error.

The tests use an in-memory database and real services. They do not sleep; the
`now` value is passed in. `retries_with_backoff_then_gives_up` advances `now`
manually so each retry is due.

Run:

```console
$ cargo test reminders
```

## Checkpoint

```console
$ cargo test reminders
$ cargo run -- worker --once
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

The worker command needs a reminder in the database. Use the CLI from chapter 15
or a test fixture; this chapter’s checkpoint is the batch behavior.

## Failure modes you will meet

**A reminder is stuck in `sending` after a crash.** No recovery lease exists. Add
a `locked_at` column and return rows whose lease expired to `pending`, with a
bounded attempt count. Test the crash window.

**The same reminder is sent twice.** A recovery job or a duplicate worker
returned a `sending` row to `pending` after the first send. At-least-once is the
contract; use an idempotency key at the provider or a delivery record.

**The user gets two in-app notifications after one retry.** The notification was
created before the email failed. Create it after success, or record a delivery
key and upsert. Choose based on which loss is worse.

**`mark_sent` returns `NotFound`.** The row was not in `sending`; another worker
marked it, or the claim transaction did not commit. Check the status and the
worker identity.

**A retry never becomes due.** The backoff uses `attempts²` minutes and the test
clock did not advance. In production, a row with a far-future `remind_at` is
correctly not due.

**`worker --once` prints 1 but no email arrived.** The default `LogMailer` logs
the message instead of sending it. Chapter 11 adds a real provider adapter.

**The worker loops too fast.** `tokio::time::interval` ticks immediately once,
then every 30 seconds. The first immediate tick is intentional. Add jitter if
many workers start together.

## Under the hood

A claim is a lease without an expiry. The row says “a worker owns this” but does
not say until when. That is enough for one worker and a small deployment; it is
not enough for a fleet. A lease adds `locked_by` and `locked_until`, and a
recovery pass returns expired leases.

`attempts` is a delivery attempt count, not a user-visible retry count. The
backoff is deterministic, so tests can assert exact instants. Production often
adds jitter to avoid synchronized retries after an outage.

The scheduler is a pull model: the worker asks the database what is due. A push
model would keep a timer per reminder in memory. Pull is simpler to restart and
to scale horizontally; the cost is a periodic query and a claim transaction.

## Exercises

1. **Add a lease.** Add `locked_by` and `locked_until`; claim with a lease; add a
   recovery pass that returns expired leases to `pending`. Test a crash window.
2. **Test with paused time.** Use `tokio::time::pause` and `advance` to run the
   loop without real sleeps. Keep the explicit-`now` tests as the primary ones.
3. **Add quiet hours.** Store a user preference window and refuse to send inside
   it, moving the reminder to the window’s end. Test both sides of the boundary.
4. **Make failures status-checked.** Add `AND status = 'sending'` to
   `mark_failed`, and return `NotFound` if zero rows change. Update the tests.
5. **Report sent vs failed.** Return a `BatchReport { claimed, sent, failed }`
   from `run_once_with`. Keep the CLI output honest.

## What comes next

The worker can claim and send, but the default sender only logs. Chapter 11 adds
the mailer seam and the in-app notification service in full: an interface, a
recording fake, an email template, and the rule that a transport failure must not
lose the notification record.
