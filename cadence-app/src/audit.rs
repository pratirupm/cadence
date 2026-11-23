use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub action: String,
    pub target: String,
    pub metadata: String,
    pub ip: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct AuditService {
    pool: SqlitePool,
}

impl AuditService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn record(
        &self,
        workspace_id: &str,
        actor_id: &str,
        action: &str,
        target: &str,
        metadata: &str,
    ) -> Result<(), AuditError> {
        sqlx::query(
            "INSERT INTO audit_log
             (id, workspace_id, actor_id, action, target, metadata, ip, created_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(Ulid::new().to_string())
        .bind(workspace_id)
        .bind(actor_id)
        .bind(action)
        .bind(target)
        .bind(metadata)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn list_for_workspace(
        &self,
        workspace_id: &str,
        limit: i64,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let entries = sqlx::query_as::<_, AuditEntry>(
            "SELECT id, workspace_id, actor_id, action, target, metadata, ip, created_at
             FROM audit_log
             WHERE workspace_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?",
        )
        .bind(workspace_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn records_and_lists_audit_entries() {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let user = Ulid::new().to_string();
        let workspace = Ulid::new().to_string();
        sqlx::query(
            "INSERT INTO users (id, email, name, password_hash, email_verified_at, created_at)
             VALUES (?, ?, 'Person', 'hash', NULL, ?)",
        )
        .bind(&user)
        .bind(format!("{user}@example.com"))
        .bind(Utc::now())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, slug, owner_id, created_at)
             VALUES (?, 'Home', ?, ?, ?)",
        )
        .bind(&workspace)
        .bind(&workspace)
        .bind(&user)
        .bind(Utc::now())
        .execute(&pool)
        .await
        .unwrap();

        let audit = AuditService::new(pool);
        audit
            .record(&workspace, &user, "member.added", "user", "")
            .await
            .unwrap();

        let entries = audit.list_for_workspace(&workspace, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "member.added");
        assert_eq!(entries[0].actor_id, user);
    }
}
