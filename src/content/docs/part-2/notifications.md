---
title: "Notifications"
description: "Add the mailer seam, email templates, and an in-app inbox."
---

The reminder scheduler can claim a row and call a mailer, but the default mailer
only writes a log line. That is useful in development and useless in production.
This chapter builds the delivery seam: a mailer interface, a recording fake, a
log implementation, email templates, and an in-app notification store.

The point is not to hide SMTP behind a trait and call it architecture. The point
is to keep the reminder state machine independent of a transport that can be
slow, rate-limited, or down. The worker should be testable without a network,
and the transport should be replaceable without touching the scheduler.

By the end of this chapter you can:

- define an async mailer interface that is object-safe
- send a message through a log mailer in development
- capture messages in a recording mailer for tests
- build verification and reset emails with the right links
- create, list, count, and mark in-app notifications read
- run a reminder batch with a fake mailer and assert exactly one email and one
  unread notification
- explain where a retry can produce a duplicate notification

The Rust lessons are async traits, trait objects, `Arc<Mutex<_>>` in a test fake,
and dependency injection through a function argument. The product lesson is that
email is a transport, not a domain rule.

## The mailer interface

Create `src/mailer.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    pub to: String,
    pub subject: String,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("could not send email: {0}")]
    Send(String),
}

#[async_trait]
pub trait Mailer: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError>;
}
```

`EmailMessage` is intentionally small: recipient, subject, and plain text. No
HTML, attachments, CC, or headers yet. Every field is a `String` because the
message crosses a process boundary and may be queued. A borrowed `&str` would
force the caller to keep the message alive across an await.

`MailError` carries a string, not a transport-specific error. The worker only
needs “failed and a readable reason”; it should not match on SMTP response codes.
A provider adapter can log the details and return the summary.

`Mailer` is `Send + Sync` because it is held by a worker and may be shared across
tasks. The async method is declared with `#[async_trait]`; without the macro, a
trait with an async method is not object-safe in a way that lets the worker hold
`&dyn Mailer`. The macro boxes the future. That allocation is irrelevant next to
an SMTP round trip.

## Two implementations

The log mailer is the development default:

```rust
#[derive(Debug, Default, Clone)]
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError> {
        tracing::info!(
            to = %message.to,
            subject = %message.subject,
            "email delivered to the log"
        );
        Ok(())
    }
}
```

It does not log the body. The body can contain a reset token or a task title, and
logs are often less protected than the database. Log the recipient and subject,
not the content.

The recording mailer is the test double:

```rust
#[derive(Debug, Default, Clone)]
pub struct RecordingMailer {
    messages: Arc<Mutex<Vec<EmailMessage>>>,
}

impl RecordingMailer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> Vec<EmailMessage> {
        self.messages.lock().expect("mailbox lock").clone()
    }
}

#[async_trait]
impl Mailer for RecordingMailer {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError> {
        self.messages
            .lock()
            .expect("mailbox lock")
            .push(message.clone());
        Ok(())
    }
}
```

`Arc` lets the fake be cloned into a worker while the test keeps a handle. The
mutex is a `std::sync::Mutex`, not a `tokio::sync::Mutex`, because the critical
section is a push and does not await. Holding a std mutex across an await is a
bug; here there is no await inside the lock.

The `expect` on the lock is acceptable in a test fake. A poisoned lock means
another test thread panicked while holding it; the test is already failing.

## Email templates

The templates are functions that return an `EmailMessage`:

```rust
pub fn verification_email(to: &str, token: &str) -> EmailMessage {
    EmailMessage {
        to: to.to_owned(),
        subject: "Verify your Cadence email".to_owned(),
        text: format!("Open /verify/{token} to verify your email address."),
    }
}

pub fn password_reset_email(to: &str, token: &str) -> EmailMessage {
    EmailMessage {
        to: to.to_owned(),
        subject: "Reset your Cadence password".to_owned(),
        text: format!(
            "Open /reset/{token} to choose a new password. The link expires in one hour."
        ),
    }
}
```

The text is a relative link in the book’s local development. A production
message needs an absolute URL from configuration, a plain-text and HTML version,
and a footer with the sending domain. The template function is the seam; the
transport is not the place to decide branding.

Do not put the token in the subject. Subjects appear in notifications, logs, and
previews. The body is the narrowest place for a secret that still reaches the
user.

The reset email says “one hour,” which matches the `TokenKind::Reset` TTL in
chapter 3. If you change the TTL, change the text or compute it from the same
constant. A stale sentence is a support ticket.

## In-app notifications

Email is not the only channel. A reminder should also appear in the app, even if
the user is already looking at it. Create `migrations/0007_notifications.sql`:

```sql
CREATE TABLE notifications (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL,
    read_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX notifications_user_id_idx
    ON notifications (user_id, created_at DESC);
```

`kind` is a string such as `reminder`, `task_assigned`, or `invitation`. The
payload is JSON text. A separate table per kind would be more typed and more
tedious; this table is a small inbox. If the payload shape becomes important,
add a typed view or a `payload_version` column.

`read_at` is nullable. `NULL` means unread. Marking read sets the timestamp; it
does not delete the row. Users expect an inbox history, and a deleted row cannot
be undeleted.

The service:

```rust
pub async fn create(
    &self,
    user_id: &str,
    kind: &str,
    payload: &str,
) -> Result<Notification, NotificationError> {
    let notification = Notification {
        id: Ulid::new().to_string(),
        user_id: user_id.to_owned(),
        kind: kind.to_owned(),
        payload: payload.to_owned(),
        read_at: None,
        created_at: Utc::now(),
    };

    // INSERT ...
    Ok(notification)
}
```

`list_for_user` orders newest first and takes a limit:

```sql
SELECT id, user_id, kind, payload, read_at, created_at
FROM notifications
WHERE user_id = ?
ORDER BY created_at DESC, id DESC
LIMIT ?
```

The limit is required, not optional. An inbox with no bound is a denial-of-service
waiting for a busy account. The caller chooses 50 or 100; the service does not
return the whole table.

`unread_count` is a `COUNT(*)` with `read_at IS NULL`. The index on
`(user_id, created_at DESC)` helps the list; the count may still scan the user’s
rows. For a large inbox, add a partial index on unread rows or maintain a counter.
Measure before adding either.

`mark_read` is a compare-and-set:

```sql
UPDATE notifications SET read_at = ?
WHERE id = ? AND user_id = ? AND read_at IS NULL
```

The `user_id` condition prevents one user from marking another user’s
notification. The `read_at IS NULL` condition makes the operation idempotent:
the first call returns `true`, the second returns `false`. Returning `false` for
“already read” is not an error; the UI can treat it as success.

## Wiring the reminder worker

`ReminderService::run_once_with` takes both dependencies:

```rust
pub async fn run_once_with(
    &self,
    mailer: &dyn Mailer,
    notifications: &NotificationService,
    now: DateTime<Utc>,
    limit: i64,
) -> Result<usize, ReminderError> {
    let reminders = self.claim_due(now, limit).await?;
    // for each reminder:
    //   load task title and user email
    //   create notification
    //   send email
    //   mark sent or failed
}
```

The default `run_once` constructs `LogMailer` and a live notification service.
Tests call `run_once_with(&RecordingMailer::new(), &notifications, now, 10)`.

The order is notification first, then email. That means a failed email still
leaves a notification, and a retry creates a second notification. That is a
deliberate at-least-once trade-off, not a hidden bug. Chapter 10 explains the
duplicate; this chapter names the fix:

- create the notification only after the email succeeds, or
- key the notification by `(reminder_id, channel, attempt)` and upsert.

Both choices are exercises. Do not “fix” it by catching the notification error
and pretending it succeeded; that loses the only in-app record.

## Worked delivery flow

A reminder delivery moves through two channels and four states:

```text
worker claims reminder
  -> notification row (unread)
  -> email message (to, subject, text)
  -> mailer returns Ok
  -> reminder sent
```

If the mailer returns an error:

```text
worker claims reminder
  -> notification row (unread)
  -> mailer returns Err
  -> reminder pending with backoff
```

The user now has a notification even though the email failed. On retry, another
notification row is created. That is the duplicate window described above. A
delivery key fixes it by making the second insert an upsert instead of a new row.

If the task or user disappeared between claim and delivery, the worker marks the
reminder failed with “task or user disappeared” and does not create a
notification. A deleted task should not produce a live reminder.

The batch returns the number of claimed rows, not the number of emails sent. A
log line “sent 3 reminders” means “claimed 3 and attempted them”; use the
`last_error` column or a richer report if you need exact counts.

## Payload shape

The payload is JSON text. Two examples:

```json
{"task_id":"01J...","title":"water plants"}
{"invitation_id":"01J...","workspace":"Home"}
```

The notification table does not enforce a schema for this string. The producer
does. Keep payloads small and stable: an ID the client can resolve, plus one or
two display fields. Do not copy the whole task into the notification; it will go
stale and it may contain fields the user should not see.

The `kind` string is the discriminator. A UI can render a reminder differently
from an invitation without parsing the payload first. If the set of kinds grows,
add a Rust enum and a conversion at the boundary, but keep the database column
as text for forward compatibility.

Unread is `read_at IS NULL`. The count is per user. The list is ordered by
creation time and ID, so two notifications created in the same millisecond still
have a stable order. Marking read is a one-way transition for the UI; the row is
not deleted.

## Why a trait, not a concrete client

A concrete `SmtpMailer` would make the reminder service depend on SMTP types,
configuration, and a network client. Tests would need a server or a mock at the
socket level. A trait keeps the service’s dependency at “something that can send
this message,” and the test double is a value with a vector.

This is not an argument for a trait for every dependency. The task repository is
a concrete SQLite service because the database is the domain’s storage, and its
SQL is part of the product. Email is an external transport with many providers
and a retry policy that belongs to the worker. The seam is justified by the
boundary, not by a desire for abstraction.

The provider adapter belongs at the edge:

```text
ReminderService -> &dyn Mailer -> ProviderMailer -> HTTPS API
```

The reminder service does not know the provider’s response codes. The adapter
maps them to `MailError::Send` and logs the details. Chapter 19 applies the same
pattern to webhooks; provider selection and credentials remain a production
exercise, not part of this chapter’s code.

## Tests

The mailer has two tests:

- `records_messages_for_tests`: the fake captures the exact message.
- `reset_email_contains_the_token`: the reset body contains `/reset/<token>`.

The notification service has one module test:

- `creates_lists_and_reads_notifications`: two creates, unread count 2, list
  length 2, first mark read returns true, unread count 1, second mark read
  returns false.

The combined reminder test from chapter 10 also proves the delivery path:

- `sends_email_and_in_app_notification_together`: one email captured, one unread
  notification.

Run:

```console
$ cargo test mailer
$ cargo test notifications
$ cargo test reminders
```

The tests do not open a network connection. The transport is a trait; the test
double is a value.

## Checkpoint

```console
$ cargo test mailer && cargo test notifications && cargo test reminders
$ cargo run -- worker --once
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

`worker --once` prints `email delivered to the log` through `LogMailer`. That is
the expected development behavior, not a missing feature.

## Failure modes you will meet

**`worker --once` logs an email but nothing arrives.** `LogMailer` is the
default. Replace it with a provider adapter in production. Do not add SMTP
credentials to the repository.

**A user gets two notifications for one reminder.** The email failed and the
reminder retried, but the notification was created before the send. Add a
delivery key and upsert, or create the notification after success. Decide which
loss is worse.

**`mark_read` returns false on the first call.** The row belongs to another user,
is already read, or does not exist. The `user_id` condition is intentional; do
not remove it to make the test pass.

**The inbox query is slow.** The limit helps, but the `ORDER BY` still needs the
index. Check `EXPLAIN QUERY PLAN`; a missing `(user_id, created_at DESC)` index
is the usual cause.

**The email contains a relative link.** Local development uses a relative path;
production needs an absolute base URL. Put the base URL in configuration and
build the link at the edge, not inside the transport.

**The reset token appears in a log.** The log mailer should never log the body.
If a provider logs full messages, treat the provider as part of the secret
surface and review its retention settings.

**The notification payload is malformed JSON.** `create` accepts a `&str` and
does not parse it. That keeps the service flexible and moves the validation to
the producer. If the payload becomes a contract, validate it before insert.

## Under the hood

`#[async_trait]` boxes the future returned by `send`. The trait remains
object-safe, so the worker can accept `&dyn Mailer` without knowing the concrete
type. The cost is one allocation per send; the network cost is larger.

`Arc<Mutex<Vec<_>>>` is a test-only shared mailbox. It is not a production queue.
A real provider has its own retry and rate limits; the worker’s retry is the
local safety net, not a replacement for the provider’s.

A notification is a row, not an event. The same event can create one row and
send one email; the two channels have independent failure modes. Treating them
as one transaction is possible but couples the inbox to the transport. The book
keeps them separate and documents the duplicate window.

## Exercises

1. **Add a delivery key.** Add `reminder_id` and `channel` to notifications with
   a unique constraint, and upsert on retry. Test that a failed email retry does
   not create a second notification.
2. **Add preferences.** Store `email_notifications` and `in_app_notifications`
   per user. Skip the channel that is disabled, and test both combinations.
3. **Paginate the inbox.** Add a cursor or offset and a test with more rows than
   the page size. Keep the newest-first order stable.
4. **Add a provider adapter.** Implement `Mailer` for a fake HTTP provider with a
   `reqwest` client. Test success, a 500, and a timeout. Do not call the real
   network in tests.
5. **Render HTML and text.** Add an `html: Option<String>` field and a template
   that includes both. Test that the text fallback still contains the link.

## What comes next

The worker can deliver email and in-app notifications, but a workspace member
cannot yet invite another person from the browser. Chapter 12 goes deeper on
sharing: private projects, project roles, and the audit log that records who
changed whom.
