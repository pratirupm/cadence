use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task title cannot be empty")]
    EmptyTitle,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct TaskRepository {
    pool: SqlitePool,
}

impl TaskRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, title: &str) -> Result<Task, TaskError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(TaskError::EmptyTitle);
        }

        let now = Utc::now();
        let task = Task {
            id: Ulid::new().to_string(),
            title: title.to_owned(),
            completed: false,
            created_at: now,
            updated_at: now,
        };

        sqlx::query(
            "INSERT INTO tasks (id, title, completed, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&task.id)
        .bind(&task.title)
        .bind(task.completed)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(task)
    }

    pub async fn get(&self, id: &str) -> Result<Option<Task>, TaskError> {
        let task = sqlx::query_as::<_, Task>(
            "SELECT id, title, completed, created_at, updated_at FROM tasks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(task)
    }

    pub async fn list(&self) -> Result<Vec<Task>, TaskError> {
        let tasks = sqlx::query_as::<_, Task>(
            "SELECT id, title, completed, created_at, updated_at
             FROM tasks
             ORDER BY created_at DESC, id DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(tasks)
    }

    pub async fn set_completed(&self, id: &str, completed: bool) -> Result<bool, TaskError> {
        let result = sqlx::query("UPDATE tasks SET completed = ?, updated_at = ? WHERE id = ?")
            .bind(completed)
            .bind(Utc::now())
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn delete(&self, id: &str) -> Result<bool, TaskError> {
        let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    async fn repository() -> TaskRepository {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        TaskRepository::new(pool)
    }

    #[tokio::test]
    async fn creates_and_reads_a_task() {
        let repository = repository().await;

        let created = repository.create("buy milk").await.unwrap();
        let loaded = repository.get(&created.id).await.unwrap().unwrap();

        assert_eq!(loaded, created);
        assert_eq!(loaded.title, "buy milk");
        assert!(!loaded.completed);
    }

    #[tokio::test]
    async fn trims_the_title() {
        let repository = repository().await;

        let task = repository.create("  buy milk  ").await.unwrap();

        assert_eq!(task.title, "buy milk");
    }

    #[tokio::test]
    async fn rejects_an_empty_title() {
        let repository = repository().await;

        let error = repository.create("   ").await.unwrap_err();

        assert!(matches!(error, TaskError::EmptyTitle));
        assert!(repository.list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn lists_newest_first() {
        let repository = repository().await;

        repository.create("first").await.unwrap();
        repository.create("second").await.unwrap();

        let titles: Vec<_> = repository
            .list()
            .await
            .unwrap()
            .into_iter()
            .map(|task| task.title)
            .collect();

        assert_eq!(titles, ["second", "first"]);
    }

    #[tokio::test]
    async fn completes_and_reopens_a_task() {
        let repository = repository().await;
        let task = repository.create("buy milk").await.unwrap();

        assert!(repository.set_completed(&task.id, true).await.unwrap());
        assert!(repository.get(&task.id).await.unwrap().unwrap().completed);

        assert!(repository.set_completed(&task.id, false).await.unwrap());
        assert!(!repository.get(&task.id).await.unwrap().unwrap().completed);
    }

    #[tokio::test]
    async fn reports_a_missing_task() {
        let repository = repository().await;

        assert!(repository.get("missing").await.unwrap().is_none());
        assert!(!repository.set_completed("missing", true).await.unwrap());
        assert!(!repository.delete("missing").await.unwrap());
    }

    #[tokio::test]
    async fn deletes_a_task() {
        let repository = repository().await;
        let task = repository.create("buy milk").await.unwrap();

        assert!(repository.delete(&task.id).await.unwrap());
        assert!(repository.get(&task.id).await.unwrap().is_none());
    }
}
