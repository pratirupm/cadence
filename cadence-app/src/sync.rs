use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Change {
    pub seq: i64,
    pub workspace_id: String,
    pub entity: String,
    pub entity_id: String,
    pub op: String,
    pub version: i64,
    pub payload: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SyncPage {
    pub changes: Vec<Change>,
    pub cursor: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct SyncService {
    pool: SqlitePool,
}

impl SyncService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn pull(&self, user_id: &str, since: i64, limit: i64) -> Result<SyncPage, SyncError> {
        let changes = sqlx::query_as::<_, Change>(
            "SELECT c.seq, c.workspace_id, c.entity, c.entity_id, c.op, c.version,
                    c.payload, c.created_at
             FROM changes c
             JOIN workspace_members m ON m.workspace_id = c.workspace_id
             WHERE m.user_id = ? AND c.seq > ?
             ORDER BY c.seq
             LIMIT ?",
        )
        .bind(user_id)
        .bind(since)
        .bind(limit.clamp(1, 500))
        .fetch_all(&self.pool)
        .await?;

        let cursor = changes.last().map(|change| change.seq).unwrap_or(since);

        Ok(SyncPage { changes, cursor })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;
    use crate::tasks::TaskService;
    use crate::workspaces::{ProjectVisibility, WorkspaceService};

    const PASSWORD: &str = "correct horse battery staple";

    struct Fixture {
        sync: SyncService,
        tasks: TaskService,
        owner: String,
        workspace_id: String,
        project_id: String,
    }

    async fn fixture() -> Fixture {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let auth = AuthService::new(pool.clone());
        let workspaces = WorkspaceService::new(pool.clone());
        let tasks = TaskService::new(pool.clone());
        let sync = SyncService::new(pool);
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

        Fixture {
            sync,
            tasks,
            owner,
            workspace_id: workspace.id,
            project_id: project.id,
        }
    }

    #[tokio::test]
    async fn records_create_update_and_delete_changes() {
        let fixture = fixture().await;
        let task = fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "buy milk",
            )
            .await
            .unwrap();
        fixture
            .tasks
            .set_completed(&fixture.owner, &task.id, true)
            .await
            .unwrap();
        fixture
            .tasks
            .delete(&fixture.owner, &task.id)
            .await
            .unwrap();

        let page = fixture.sync.pull(&fixture.owner, 0, 100).await.unwrap();
        let ops: Vec<_> = page
            .changes
            .iter()
            .map(|change| change.op.as_str())
            .collect();

        assert_eq!(ops, ["create", "update", "delete"]);
        assert_eq!(page.changes[1].version, 2);
        assert_eq!(page.changes[2].version, 3);
        assert!(page.changes[0].payload.contains("buy milk"));
        assert_eq!(page.cursor, page.changes.last().unwrap().seq);
    }

    #[tokio::test]
    async fn only_returns_changes_from_member_workspaces() {
        let fixture = fixture().await;
        fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "buy milk",
            )
            .await
            .unwrap();
        let outsider = AuthService::new(fixture.sync.pool.clone())
            .create_user("outsider@example.com", "Outsider", PASSWORD, true)
            .await
            .unwrap()
            .id;

        let page = fixture.sync.pull(&outsider, 0, 100).await.unwrap();
        assert!(page.changes.is_empty());
        assert_eq!(page.cursor, 0);
    }

    #[tokio::test]
    async fn resumes_after_a_cursor() {
        let fixture = fixture().await;
        fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "first",
            )
            .await
            .unwrap();
        fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "second",
            )
            .await
            .unwrap();

        let first = fixture.sync.pull(&fixture.owner, 0, 1).await.unwrap();
        assert_eq!(first.changes.len(), 1);
        let second = fixture
            .sync
            .pull(&fixture.owner, first.cursor, 10)
            .await
            .unwrap();
        assert_eq!(second.changes.len(), 1);
        assert!(second.changes[0].payload.contains("second"));
    }
}
