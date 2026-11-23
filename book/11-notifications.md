# 11. Notifications

**What you can do at the end.** A reminder creates an in-app notification and
sends an email through a swappable mailer.

**The Rust you learn.** Traits with async methods, dependency injection,
recording fakes, deduplication, and keeping transport out of domain code.

**Build it.**

- Add `mailer.rs` with `EmailMessage`, `Mailer`, `MailError`, and `LogMailer`.
- Add `RecordingMailer` for tests: it stores messages in `Arc<Mutex<Vec<_>>>`.
- Add a notifications table and `NotificationService`.
- Make `ReminderService::run_once_with(&dyn Mailer, &NotificationService, ...)`
  take both dependencies.
- On success, create the notification and send the email; on failure, retry.
- Keep the default `run_once` wired to `LogMailer` for local development.

The worker depends on the interface, not on SMTP:

```rust
pub trait Mailer: Send + Sync {
    async fn send(&self, message: &EmailMessage) -> Result<(), MailError>;
}
```

**Checkpoint.**

```console
$ cargo test mailer
$ cargo test notifications
$ cargo test reminders
```

The tests assert the fake captured one email, the user has one unread
notification, and marking it read is idempotent.

**Exercises.**

1. Add a `BulkMailer` that batches messages and flushes on drop.
2. Add notification preferences for email vs in-app.
