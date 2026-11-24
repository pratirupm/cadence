use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::mailer::{EmailMessage, LogMailer, MailError, Mailer};
use crate::notifications::{NotificationError, NotificationService};
use crate::tasks::{TaskError, TaskService};
use crate::workspaces::{WorkspaceError, WorkspaceService};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Reminder {
    pub id: String,
    pub task_id: String,
    pub user_id: String,
    pub remind_at: DateTime<Utc>,
    pub channel: String,
    pub sent_at: Option<DateTime<Utc>>,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum ReminderError {
    #[error("you do not have permission to do that")]
    Forbidden,
    #[error("reminder not found")]
    NotFound,
    #[error(transparent)]
    Task(#[from] TaskError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Mail(#[from] MailError),
    #[error(transparent)]
    Notification(#[from] NotificationError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct ReminderService {
    pool: SqlitePool,
}

impl ReminderService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

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
        if workspaces
            .role(&task.workspace_id, user_id)
            .await?
            .is_none()
        {
            return Err(ReminderError::Forbidden);
        }

        let reminder = Reminder {
            id: Ulid::new().to_string(),
            task_id: task.id,
            user_id: user_id.to_owned(),
            remind_at,
            channel: "email".to_owned(),
            sent_at: None,
            status: "pending".to_owned(),
            attempts: 0,
            last_error: None,
            created_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO reminders
             (id, task_id, user_id, remind_at, channel, sent_at, status, attempts, last_error, created_at)
             VALUES (?, ?, ?, ?, ?, NULL, 'pending', 0, NULL, ?)",
        )
        .bind(&reminder.id)
        .bind(&reminder.task_id)
        .bind(&reminder.user_id)
        .bind(reminder.remind_at)
        .bind(&reminder.channel)
        .bind(reminder.created_at)
        .execute(&self.pool)
        .await?;

        Ok(reminder)
    }

    pub async fn claim_due(
        &self,
        now: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<Reminder>, ReminderError> {
        let mut transaction = self.pool.begin().await?;
        let mut reminders = sqlx::query_as::<_, Reminder>(
            "SELECT id, task_id, user_id, remind_at, channel, sent_at, status, attempts,
                    last_error, created_at
             FROM reminders
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

    pub async fn mark_sent(&self, id: &str) -> Result<(), ReminderError> {
        let result = sqlx::query(
            "UPDATE reminders SET status = 'sent', sent_at = ? WHERE id = ? AND status = 'sending'",
        )
        .bind(Utc::now())
        .bind(id)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(ReminderError::NotFound);
        }

        Ok(())
    }

    pub async fn mark_failed(
        &self,
        id: &str,
        now: DateTime<Utc>,
        error: &str,
    ) -> Result<(), ReminderError> {
        let reminder = sqlx::query_as::<_, Reminder>(
            "SELECT id, task_id, user_id, remind_at, channel, sent_at, status, attempts,
                    last_error, created_at
             FROM reminders WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(ReminderError::NotFound)?;

        if reminder.attempts >= 5 {
            sqlx::query("UPDATE reminders SET status = 'failed', last_error = ? WHERE id = ?")
                .bind(error)
                .bind(id)
                .execute(&self.pool)
                .await?;
        } else {
            let delay = Duration::minutes(reminder.attempts * reminder.attempts);
            sqlx::query(
                "UPDATE reminders
                 SET status = 'pending', remind_at = ?, last_error = ?
                 WHERE id = ?",
            )
            .bind(now + delay)
            .bind(error)
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }

    pub async fn run_once(&self, now: DateTime<Utc>, limit: i64) -> Result<usize, ReminderError> {
        let mailer = LogMailer;
        let notifications = NotificationService::new(self.pool.clone());
        self.run_once_with(&mailer, &notifications, now, limit)
            .await
    }

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
                self.mark_failed(&reminder.id, now, "task or user disappeared")
                    .await?;
                continue;
            };

            tracing::info!(
                reminder_id = %reminder.id,
                task_id = %reminder.task_id,
                user_id = %reminder.user_id,
                "reminder due"
            );

            let payload = serde_json::json!({
                "task_id": reminder.task_id,
                "title": title,
            })
            .to_string();
            notifications
                .create(&reminder.user_id, "reminder", &payload)
                .await?;

            let message = EmailMessage {
                to: email,
                subject: format!("Reminder: {title}"),
                text: format!("Your task \"{title}\" is due for a reminder."),
            };

            if let Err(error) = mailer.send(&message).await {
                self.mark_failed(&reminder.id, now, &error.to_string())
                    .await?;
            } else {
                self.mark_sent(&reminder.id).await?;
            }
        }

        Ok(count)
    }
}

pub async fn run_forever(pool: SqlitePool) -> Result<(), ReminderError> {
    let service = ReminderService::new(pool);
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(30));

    loop {
        ticker.tick().await;
        if let Err(error) = service.run_once(Utc::now(), 50).await {
            tracing::error!(%error, "reminder worker failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;
    use crate::workspaces::ProjectVisibility;

    const PASSWORD: &str = "correct horse battery staple";

    struct Fixture {
        reminders: ReminderService,
        owner: String,
        task_id: String,
    }

    async fn fixture() -> Fixture {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone());
        let workspaces = WorkspaceService::new(pool.clone());
        let tasks = TaskService::new(pool.clone());
        let reminders = ReminderService::new(pool);
        let owner = auth
            .create_user("owner@example.com", "Owner", PASSWORD, true)
            .await
            .unwrap()
            .id;
        let workspace = workspaces.create_workspace(&owner, "Home").await.unwrap();
        let project = workspaces
            .create_project(
                &owner,
                &workspace.id,
                "Chores",
                ProjectVisibility::Workspace,
            )
            .await
            .unwrap();
        let task = tasks
            .create(&owner, &workspace.id, &project.id, "water plants")
            .await
            .unwrap();

        Fixture {
            reminders,
            owner,
            task_id: task.id,
        }
    }

    #[tokio::test]
    async fn claims_due_reminders_once() {
        let fixture = fixture().await;
        let now = Utc::now();
        let reminder = fixture
            .reminders
            .create(&fixture.owner, &fixture.task_id, &fixture.owner, now)
            .await
            .unwrap();

        let due = fixture.reminders.claim_due(now, 10).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, reminder.id);
        assert_eq!(due[0].attempts, 1);
        assert_eq!(due[0].status, "sending");

        assert!(
            fixture
                .reminders
                .claim_due(now, 10)
                .await
                .unwrap()
                .is_empty()
        );

        fixture.reminders.mark_sent(&reminder.id).await.unwrap();
        assert!(
            fixture
                .reminders
                .claim_due(now, 10)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn retries_with_backoff_then_gives_up() {
        let fixture = fixture().await;
        let mut now = Utc::now();
        let reminder = fixture
            .reminders
            .create(&fixture.owner, &fixture.task_id, &fixture.owner, now)
            .await
            .unwrap();

        for attempt in 1..=5 {
            let due = fixture.reminders.claim_due(now, 10).await.unwrap();
            assert_eq!(due.len(), 1);
            fixture
                .reminders
                .mark_failed(&reminder.id, now, "smtp down")
                .await
                .unwrap();
            now += Duration::minutes(30);

            let row = sqlx::query_as::<_, Reminder>(
                "SELECT id, task_id, user_id, remind_at, channel, sent_at, status, attempts,
                        last_error, created_at
                 FROM reminders WHERE id = ?",
            )
            .bind(&reminder.id)
            .fetch_one(&fixture.reminders.pool)
            .await
            .unwrap();

            if attempt < 5 {
                assert_eq!(row.status, "pending");
            } else {
                assert_eq!(row.status, "failed");
            }
        }
    }

    #[tokio::test]
    async fn sends_email_and_in_app_notification_together() {
        let fixture = fixture().await;
        let now = Utc::now();
        fixture
            .reminders
            .create(&fixture.owner, &fixture.task_id, &fixture.owner, now)
            .await
            .unwrap();

        let mailer = crate::mailer::RecordingMailer::new();
        let notifications = NotificationService::new(fixture.reminders.pool.clone());
        let count = fixture
            .reminders
            .run_once_with(&mailer, &notifications, now, 10)
            .await
            .unwrap();

        assert_eq!(count, 1);
        assert_eq!(mailer.messages().len(), 1);
        assert_eq!(notifications.unread_count(&fixture.owner).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn outsiders_cannot_create_reminders() {
        let fixture = fixture().await;
        let pool = fixture.reminders.pool.clone();
        let auth = AuthService::new(pool);
        let outsider = auth
            .create_user("outsider@example.com", "Outsider", PASSWORD, true)
            .await
            .unwrap()
            .id;

        assert!(matches!(
            fixture
                .reminders
                .create(&outsider, &fixture.task_id, &outsider, Utc::now())
                .await,
            Err(ReminderError::Task(_))
        ));
    }
}
