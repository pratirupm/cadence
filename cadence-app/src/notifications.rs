use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Notification {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub payload: String,
    pub read_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct NotificationService {
    pool: SqlitePool,
}

impl NotificationService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

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

        sqlx::query(
            "INSERT INTO notifications (id, user_id, kind, payload, read_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&notification.id)
        .bind(&notification.user_id)
        .bind(&notification.kind)
        .bind(&notification.payload)
        .bind(notification.created_at)
        .execute(&self.pool)
        .await?;

        Ok(notification)
    }

    pub async fn list_for_user(
        &self,
        user_id: &str,
        limit: i64,
    ) -> Result<Vec<Notification>, NotificationError> {
        let notifications = sqlx::query_as::<_, Notification>(
            "SELECT id, user_id, kind, payload, read_at, created_at
             FROM notifications
             WHERE user_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )
        .bind(user_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(notifications)
    }

    pub async fn unread_count(&self, user_id: &str) -> Result<i64, NotificationError> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM notifications WHERE user_id = ? AND read_at IS NULL",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(count)
    }

    pub async fn mark_read(&self, user_id: &str, id: &str) -> Result<bool, NotificationError> {
        let result = sqlx::query(
            "UPDATE notifications SET read_at = ? WHERE id = ? AND user_id = ? AND read_at IS NULL",
        )
        .bind(Utc::now())
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn service_and_user() -> (NotificationService, String) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let user_id = Ulid::new().to_string();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash, email_verified_at, created_at)
             VALUES (?, ?, 'Person', 'hash', NULL, ?)",
        )
        .bind(&user_id)
        .bind(format!("{user_id}@example.com"))
        .bind(Utc::now())
        .execute(&pool)
        .await
        .unwrap();

        (NotificationService::new(pool), user_id)
    }

    #[tokio::test]
    async fn creates_lists_and_reads_notifications() {
        let (notifications, user) = service_and_user().await;
        let first = notifications
            .create(&user, "task_assigned", "{\"task_id\":\"1\"}")
            .await
            .unwrap();
        notifications
            .create(&user, "reminder", "{\"task_id\":\"2\"}")
            .await
            .unwrap();

        assert_eq!(notifications.unread_count(&user).await.unwrap(), 2);

        let list = notifications.list_for_user(&user, 10).await.unwrap();
        assert_eq!(list.len(), 2);

        assert!(notifications.mark_read(&user, &first.id).await.unwrap());
        assert_eq!(notifications.unread_count(&user).await.unwrap(), 1);
        assert!(!notifications.mark_read(&user, &first.id).await.unwrap());
    }
}
