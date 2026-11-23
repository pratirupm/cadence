use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::workspaces::{Project, Role, WorkspaceError, WorkspaceService};

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub title: String,
    pub notes: String,
    pub completed: bool,
    pub priority: i64,
    pub due_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub assigned_to: Option<String>,
    pub version: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("task title cannot be empty")]
    EmptyTitle,
    #[error("task not found")]
    NotFound,
    #[error("project not found")]
    ProjectNotFound,
    #[error("you do not have permission to do that")]
    Forbidden,
    #[error("the task changed since you loaded it")]
    Conflict,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct TaskService {
    pool: SqlitePool,
}

impl TaskService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        actor_id: &str,
        workspace_id: &str,
        project_id: &str,
        title: &str,
    ) -> Result<Task, TaskError> {
        let title = normalize_title(title)?;
        self.require_write(workspace_id, actor_id).await?;
        self.require_project(workspace_id, project_id).await?;

        let now = Utc::now();
        let task = Task {
            id: Ulid::new().to_string(),
            workspace_id: workspace_id.to_owned(),
            project_id: project_id.to_owned(),
            title,
            notes: String::new(),
            completed: false,
            priority: 0,
            due_at: None,
            created_by: actor_id.to_owned(),
            assigned_to: None,
            version: 1,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };

        sqlx::query(
            "INSERT INTO tasks
             (id, workspace_id, project_id, title, notes, completed, priority, due_at,
              created_by, assigned_to, version, created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, '', 0, 0, NULL, ?, NULL, 1, ?, ?, NULL)",
        )
        .bind(&task.id)
        .bind(&task.workspace_id)
        .bind(&task.project_id)
        .bind(&task.title)
        .bind(&task.created_by)
        .bind(task.created_at)
        .bind(task.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(task)
    }

    pub async fn list_for_project(
        &self,
        actor_id: &str,
        project_id: &str,
    ) -> Result<Vec<Task>, TaskError> {
        let project = self.find_project(project_id).await?;
        self.require_member(&project.workspace_id, actor_id).await?;

        let tasks = sqlx::query_as::<_, Task>(
            "SELECT id, workspace_id, project_id, title, notes, completed, priority, due_at,
                    created_by, assigned_to, version, created_at, updated_at, deleted_at
             FROM tasks
             WHERE project_id = ? AND deleted_at IS NULL
             ORDER BY completed, created_at DESC, id DESC",
        )
        .bind(project_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(tasks)
    }

    pub async fn get(&self, actor_id: &str, id: &str) -> Result<Task, TaskError> {
        let task = self.find(id).await?;
        self.require_member(&task.workspace_id, actor_id).await?;
        Ok(task)
    }

    pub async fn set_completed(
        &self,
        actor_id: &str,
        id: &str,
        completed: bool,
    ) -> Result<Task, TaskError> {
        let task = self.find(id).await?;
        self.require_write(&task.workspace_id, actor_id).await?;

        let result = sqlx::query(
            "UPDATE tasks
             SET completed = ?, version = version + 1, updated_at = ?
             WHERE id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(completed)
        .bind(Utc::now())
        .bind(id)
        .bind(task.version)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(TaskError::Conflict);
        }

        self.find(id).await
    }

    pub async fn rename(&self, actor_id: &str, id: &str, title: &str) -> Result<Task, TaskError> {
        let title = normalize_title(title)?;
        let task = self.find(id).await?;
        self.require_write(&task.workspace_id, actor_id).await?;

        let result = sqlx::query(
            "UPDATE tasks
             SET title = ?, version = version + 1, updated_at = ?
             WHERE id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(title)
        .bind(Utc::now())
        .bind(id)
        .bind(task.version)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(TaskError::Conflict);
        }

        self.find(id).await
    }

    pub async fn delete(&self, actor_id: &str, id: &str) -> Result<(), TaskError> {
        let task = self.find(id).await?;
        self.require_write(&task.workspace_id, actor_id).await?;

        let result = sqlx::query(
            "UPDATE tasks
             SET deleted_at = ?, version = version + 1, updated_at = ?
             WHERE id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(Utc::now())
        .bind(Utc::now())
        .bind(id)
        .bind(task.version)
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(TaskError::Conflict);
        }

        Ok(())
    }

    async fn find(&self, id: &str) -> Result<Task, TaskError> {
        sqlx::query_as::<_, Task>(
            "SELECT id, workspace_id, project_id, title, notes, completed, priority, due_at,
                    created_by, assigned_to, version, created_at, updated_at, deleted_at
             FROM tasks WHERE id = ? AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(TaskError::NotFound)
    }

    async fn find_project(&self, project_id: &str) -> Result<Project, TaskError> {
        sqlx::query_as::<_, Project>(
            "SELECT id, workspace_id, name, visibility, archived_at, created_at
             FROM projects WHERE id = ? AND archived_at IS NULL",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(TaskError::ProjectNotFound)
    }

    async fn require_project(&self, workspace_id: &str, project_id: &str) -> Result<(), TaskError> {
        let project = self.find_project(project_id).await?;
        if project.workspace_id != workspace_id {
            return Err(TaskError::ProjectNotFound);
        }
        Ok(())
    }

    async fn require_member(&self, workspace_id: &str, user_id: &str) -> Result<Role, TaskError> {
        self.workspaces()
            .role(workspace_id, user_id)
            .await?
            .ok_or(TaskError::Forbidden)
    }

    async fn require_write(&self, workspace_id: &str, user_id: &str) -> Result<(), TaskError> {
        let role = self.require_member(workspace_id, user_id).await?;
        if role.can_write() {
            Ok(())
        } else {
            Err(TaskError::Forbidden)
        }
    }

    fn workspaces(&self) -> WorkspaceService {
        WorkspaceService::new(self.pool.clone())
    }
}

fn normalize_title(title: &str) -> Result<String, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        Err(TaskError::EmptyTitle)
    } else {
        Ok(title.to_owned())
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
        auth: AuthService,
        workspaces: WorkspaceService,
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
        let tasks = TaskService::new(pool);
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
            auth,
            workspaces,
            tasks,
            owner,
            workspace_id: workspace.id,
            project_id: project.id,
        }
    }

    #[tokio::test]
    async fn creates_and_lists_tasks_in_a_project() {
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
        let tasks = fixture
            .tasks
            .list_for_project(&fixture.owner, &fixture.project_id)
            .await
            .unwrap();

        assert_eq!(tasks, [task]);
        assert!(!tasks[0].completed);
        assert_eq!(tasks[0].version, 1);
    }

    #[tokio::test]
    async fn trims_titles_and_rejects_empty_ones() {
        let fixture = fixture().await;

        let task = fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "  buy milk  ",
            )
            .await
            .unwrap();
        assert_eq!(task.title, "buy milk");

        assert!(matches!(
            fixture
                .tasks
                .create(
                    &fixture.owner,
                    &fixture.workspace_id,
                    &fixture.project_id,
                    "   "
                )
                .await,
            Err(TaskError::EmptyTitle)
        ));
    }

    #[tokio::test]
    async fn completes_reopens_and_renames_a_task() {
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

        let completed = fixture
            .tasks
            .set_completed(&fixture.owner, &task.id, true)
            .await
            .unwrap();
        assert!(completed.completed);
        assert_eq!(completed.version, 2);

        let renamed = fixture
            .tasks
            .rename(&fixture.owner, &task.id, "buy oat milk")
            .await
            .unwrap();
        assert_eq!(renamed.title, "buy oat milk");
        assert_eq!(renamed.version, 3);
    }

    #[tokio::test]
    async fn soft_deletes_a_task() {
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
            .delete(&fixture.owner, &task.id)
            .await
            .unwrap();

        assert!(matches!(
            fixture.tasks.get(&fixture.owner, &task.id).await,
            Err(TaskError::NotFound)
        ));
        assert!(
            fixture
                .tasks
                .list_for_project(&fixture.owner, &fixture.project_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn viewers_cannot_change_tasks() {
        let fixture = fixture().await;
        let viewer = fixture
            .auth
            .create_user("viewer@example.com", "Viewer", PASSWORD, true)
            .await
            .unwrap()
            .id;
        fixture
            .workspaces
            .add_member(&fixture.owner, &fixture.workspace_id, &viewer, Role::Viewer)
            .await
            .unwrap();
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

        assert!(matches!(
            fixture.tasks.set_completed(&viewer, &task.id, true).await,
            Err(TaskError::Forbidden)
        ));
        assert!(
            fixture
                .tasks
                .list_for_project(&viewer, &fixture.project_id)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn outsiders_cannot_read_the_project() {
        let fixture = fixture().await;
        let outsider = fixture
            .auth
            .create_user("outsider@example.com", "Outsider", PASSWORD, true)
            .await
            .unwrap()
            .id;

        assert!(matches!(
            fixture
                .tasks
                .list_for_project(&outsider, &fixture.project_id)
                .await,
            Err(TaskError::Forbidden)
        ));
    }
}
