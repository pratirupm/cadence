use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::tasks::{TaskError, TaskService};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPush {
    pub operations: Vec<SyncOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncOperation {
    pub op_id: String,
    pub entity: String,
    pub entity_id: String,
    pub op: String,
    pub base_version: i64,
    pub title: Option<String>,
    pub completed: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncResult {
    pub op_id: String,
    pub status: String,
    pub version: Option<i64>,
    pub conflict: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncPushResponse {
    pub results: Vec<SyncResult>,
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Task(#[from] TaskError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
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

    pub async fn push(&self, user_id: &str, push: SyncPush) -> Result<SyncPushResponse, SyncError> {
        let tasks = TaskService::new(self.pool.clone());
        let mut results = Vec::new();

        for operation in push.operations {
            if let Some(stored) = sqlx::query_scalar::<_, String>(
                "SELECT response FROM sync_operations WHERE op_id = ? AND user_id = ?",
            )
            .bind(&operation.op_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await?
            {
                results.push(serde_json::from_str(&stored)?);
                continue;
            }

            let result = self
                .apply_operation(&tasks, user_id, &operation)
                .await
                .unwrap_or_else(|error| SyncResult {
                    op_id: operation.op_id.clone(),
                    status: "error".to_owned(),
                    version: None,
                    conflict: None,
                    error: Some(error.to_string()),
                });

            sqlx::query(
                "INSERT INTO sync_operations (op_id, user_id, response, created_at)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(&result.op_id)
            .bind(user_id)
            .bind(serde_json::to_string(&result)?)
            .bind(Utc::now())
            .execute(&self.pool)
            .await?;

            results.push(result);
        }

        Ok(SyncPushResponse { results })
    }

    async fn apply_operation(
        &self,
        tasks: &TaskService,
        user_id: &str,
        operation: &SyncOperation,
    ) -> Result<SyncResult, SyncError> {
        if operation.entity != "task" {
            return Ok(SyncResult {
                op_id: operation.op_id.clone(),
                status: "error".to_owned(),
                version: None,
                conflict: None,
                error: Some("unsupported entity".to_owned()),
            });
        }

        if operation.op == "update" {
            match tasks
                .apply_sync_update(
                    user_id,
                    &operation.entity_id,
                    operation.title.as_deref(),
                    operation.completed,
                    operation.base_version,
                )
                .await
            {
                Ok(task) => Ok(SyncResult {
                    op_id: operation.op_id.clone(),
                    status: "ok".to_owned(),
                    version: Some(task.version),
                    conflict: None,
                    error: None,
                }),
                Err(TaskError::Conflict) => {
                    let current = tasks.get(user_id, &operation.entity_id).await?;
                    Ok(SyncResult {
                        op_id: operation.op_id.clone(),
                        status: "conflict".to_owned(),
                        version: Some(current.version),
                        conflict: Some(serde_json::to_value(current)?),
                        error: None,
                    })
                }
                Err(error) => Err(SyncError::Task(error)),
            }
        } else if operation.op == "delete" {
            match tasks.get(user_id, &operation.entity_id).await {
                Ok(task) if task.version == operation.base_version => {
                    tasks.delete(user_id, &operation.entity_id).await?;
                    Ok(SyncResult {
                        op_id: operation.op_id.clone(),
                        status: "ok".to_owned(),
                        version: Some(task.version + 1),
                        conflict: None,
                        error: None,
                    })
                }
                Ok(task) => Ok(SyncResult {
                    op_id: operation.op_id.clone(),
                    status: "conflict".to_owned(),
                    version: Some(task.version),
                    conflict: Some(serde_json::to_value(task)?),
                    error: None,
                }),
                Err(error) => Err(SyncError::Task(error)),
            }
        } else {
            Ok(SyncResult {
                op_id: operation.op_id.clone(),
                status: "error".to_owned(),
                version: None,
                conflict: None,
                error: Some("unsupported operation".to_owned()),
            })
        }
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
    async fn pushes_updates_and_reports_conflicts() {
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
        let push = SyncPush {
            operations: vec![SyncOperation {
                op_id: "op-1".to_owned(),
                entity: "task".to_owned(),
                entity_id: task.id.clone(),
                op: "update".to_owned(),
                base_version: 1,
                title: Some("buy oat milk".to_owned()),
                completed: None,
            }],
        };

        let response = fixture
            .sync
            .push(&fixture.owner, push.clone())
            .await
            .unwrap();
        assert_eq!(response.results[0].status, "ok");
        assert_eq!(response.results[0].version, Some(2));

        let replay = fixture.sync.push(&fixture.owner, push).await.unwrap();
        assert_eq!(replay.results[0], response.results[0]);

        let stale = fixture
            .sync
            .push(
                &fixture.owner,
                SyncPush {
                    operations: vec![SyncOperation {
                        op_id: "op-2".to_owned(),
                        entity: "task".to_owned(),
                        entity_id: task.id,
                        op: "update".to_owned(),
                        base_version: 1,
                        title: Some("stale".to_owned()),
                        completed: None,
                    }],
                },
            )
            .await
            .unwrap();
        assert_eq!(stale.results[0].status, "conflict");
        assert_eq!(stale.results[0].version, Some(2));
        assert!(stale.results[0].conflict.is_some());
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
