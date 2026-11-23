use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::query::{self, Param, QueryError};
use crate::recurrence::{Recurrence, RecurrenceError};
use crate::workspaces::{Project, Role, WorkspaceError, WorkspaceService};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct Task {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub title: String,
    pub notes: String,
    pub completed: bool,
    pub priority: i64,
    pub due_at: Option<DateTime<Utc>>,
    pub recurrence: Option<String>,
    pub recurrence_timezone: Option<String>,
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
    Query(#[from] QueryError),
    #[error(transparent)]
    Recurrence(#[from] RecurrenceError),
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
        self.require_project_access(actor_id, workspace_id, project_id)
            .await?;

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
            recurrence: None,
            recurrence_timezone: None,
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
              recurrence, recurrence_timezone, created_by, assigned_to, version, created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, '', 0, 0, NULL, NULL, NULL, ?, NULL, 1, ?, ?, NULL)",
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
        self.list_filtered(actor_id, project_id, "").await
    }

    pub async fn list_filtered(
        &self,
        actor_id: &str,
        project_id: &str,
        filter: &str,
    ) -> Result<Vec<Task>, TaskError> {
        let project = self.find_project(project_id).await?;
        self.require_member(&project.workspace_id, actor_id).await?;
        self.require_project_access(actor_id, &project.workspace_id, project_id)
            .await?;

        let filter = filter.trim();
        let (sql, params) = if filter.is_empty() {
            (
                "SELECT id, workspace_id, project_id, title, notes, completed, priority, due_at,
                        recurrence, recurrence_timezone, created_by, assigned_to, version, created_at, updated_at, deleted_at
                 FROM tasks
                 WHERE project_id = ? AND deleted_at IS NULL
                 ORDER BY completed, created_at DESC, id DESC"
                    .to_owned(),
                Vec::new(),
            )
        } else {
            let compiled = query::parse(filter)?.to_sql()?;
            (
                format!(
                    "SELECT id, workspace_id, project_id, title, notes, completed, priority, due_at,
                            recurrence, recurrence_timezone, created_by, assigned_to, version, created_at, updated_at, deleted_at
                     FROM tasks
                     WHERE project_id = ? AND deleted_at IS NULL AND ({})
                     ORDER BY completed, created_at DESC, id DESC",
                    compiled.clause
                ),
                compiled.params,
            )
        };

        let mut request = sqlx::query_as::<_, Task>(&sql).bind(project_id);
        for param in params {
            request = match param {
                Param::Text(value) => request.bind(value),
                Param::Integer(value) => request.bind(value),
            };
        }

        Ok(request.fetch_all(&self.pool).await?)
    }

    pub async fn get(&self, actor_id: &str, id: &str) -> Result<Task, TaskError> {
        let task = self.find(id).await?;
        self.require_member(&task.workspace_id, actor_id).await?;
        self.require_project_access(actor_id, &task.workspace_id, &task.project_id)
            .await?;
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
        self.require_project_access(actor_id, &task.workspace_id, &task.project_id)
            .await?;

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

        let task = self.find(id).await?;
        if completed && task.recurrence.is_some() {
            self.create_next_occurrence(&task).await?;
        }

        Ok(task)
    }

    pub async fn set_recurrence(
        &self,
        actor_id: &str,
        id: &str,
        rrule: &str,
        timezone: &str,
        starts_at: DateTime<Utc>,
    ) -> Result<Task, TaskError> {
        Recurrence::parse(rrule, timezone, starts_at)?;
        let task = self.find(id).await?;
        self.require_write(&task.workspace_id, actor_id).await?;
        self.require_project_access(actor_id, &task.workspace_id, &task.project_id)
            .await?;

        let result = sqlx::query(
            "UPDATE tasks
             SET recurrence = ?, recurrence_timezone = ?, due_at = ?,
                 version = version + 1, updated_at = ?
             WHERE id = ? AND version = ? AND deleted_at IS NULL",
        )
        .bind(rrule)
        .bind(timezone)
        .bind(starts_at)
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

    async fn create_next_occurrence(&self, task: &Task) -> Result<Option<Task>, TaskError> {
        let Some(rrule) = task.recurrence.as_deref() else {
            return Ok(None);
        };
        let timezone = task.recurrence_timezone.as_deref().unwrap_or("UTC");
        let starts_at = task.due_at.unwrap_or(task.created_at);
        let recurrence = Recurrence::parse(rrule, timezone, starts_at)?;
        let after = Utc::now().max(starts_at);
        let Some(due_at) = recurrence.next_after(after) else {
            return Ok(None);
        };

        let now = Utc::now();
        let next = Task {
            id: Ulid::new().to_string(),
            workspace_id: task.workspace_id.clone(),
            project_id: task.project_id.clone(),
            title: task.title.clone(),
            notes: task.notes.clone(),
            completed: false,
            priority: task.priority,
            due_at: Some(due_at),
            recurrence: Some(rrule.to_owned()),
            recurrence_timezone: Some(timezone.to_owned()),
            created_by: task.created_by.clone(),
            assigned_to: task.assigned_to.clone(),
            version: 1,
            created_at: now,
            updated_at: now,
            deleted_at: None,
        };

        sqlx::query(
            "INSERT INTO tasks
             (id, workspace_id, project_id, title, notes, completed, priority, due_at,
              recurrence, recurrence_timezone, created_by, assigned_to, version,
              created_at, updated_at, deleted_at)
             VALUES (?, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, 1, ?, ?, NULL)",
        )
        .bind(&next.id)
        .bind(&next.workspace_id)
        .bind(&next.project_id)
        .bind(&next.title)
        .bind(&next.notes)
        .bind(next.priority)
        .bind(next.due_at)
        .bind(&next.recurrence)
        .bind(&next.recurrence_timezone)
        .bind(&next.created_by)
        .bind(&next.assigned_to)
        .bind(next.created_at)
        .bind(next.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(Some(next))
    }

    pub async fn rename(&self, actor_id: &str, id: &str, title: &str) -> Result<Task, TaskError> {
        let title = normalize_title(title)?;
        let task = self.find(id).await?;
        self.require_write(&task.workspace_id, actor_id).await?;
        self.require_project_access(actor_id, &task.workspace_id, &task.project_id)
            .await?;

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
        self.require_project_access(actor_id, &task.workspace_id, &task.project_id)
            .await?;

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
                    recurrence, recurrence_timezone, created_by, assigned_to, version, created_at, updated_at, deleted_at
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

    async fn require_project_access(
        &self,
        actor_id: &str,
        workspace_id: &str,
        project_id: &str,
    ) -> Result<(), TaskError> {
        self.require_project(workspace_id, project_id).await?;
        if self
            .workspaces()
            .can_access_project(actor_id, project_id)
            .await?
        {
            Ok(())
        } else {
            Err(TaskError::Forbidden)
        }
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
    async fn completing_a_recurring_task_creates_the_next_occurrence() {
        let fixture = fixture().await;
        let task = fixture
            .tasks
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "water plants",
            )
            .await
            .unwrap();
        let starts_at = Utc::now() + chrono::Duration::hours(1);
        fixture
            .tasks
            .set_recurrence(&fixture.owner, &task.id, "FREQ=DAILY", "UTC", starts_at)
            .await
            .unwrap();

        fixture
            .tasks
            .set_completed(&fixture.owner, &task.id, true)
            .await
            .unwrap();

        let tasks = fixture
            .tasks
            .list_for_project(&fixture.owner, &fixture.project_id)
            .await
            .unwrap();
        assert_eq!(tasks.len(), 2);
        assert!(tasks.iter().any(|task| task.completed));
        let next = tasks.iter().find(|task| !task.completed).unwrap();
        assert_eq!(next.title, "water plants");
        assert_eq!(next.due_at.unwrap(), starts_at + chrono::Duration::days(1));
    }

    #[tokio::test]
    async fn private_projects_require_project_membership() {
        let fixture = fixture().await;
        let member = fixture
            .auth
            .create_user("member@example.com", "Member", PASSWORD, true)
            .await
            .unwrap()
            .id;
        fixture
            .workspaces
            .add_member(&fixture.owner, &fixture.workspace_id, &member, Role::Member)
            .await
            .unwrap();
        let private = fixture
            .workspaces
            .create_project(
                &fixture.owner,
                &fixture.workspace_id,
                "Private",
                crate::workspaces::ProjectVisibility::Private,
            )
            .await
            .unwrap();
        let task = fixture
            .tasks
            .create(&fixture.owner, &fixture.workspace_id, &private.id, "secret")
            .await
            .unwrap();

        assert!(matches!(
            fixture.tasks.get(&member, &task.id).await,
            Err(TaskError::Forbidden)
        ));

        fixture
            .workspaces
            .add_project_member(
                &fixture.owner,
                &private.id,
                &member,
                crate::workspaces::ProjectRole::Viewer,
            )
            .await
            .unwrap();

        assert!(fixture.tasks.get(&member, &task.id).await.is_ok());
        assert!(
            fixture
                .tasks
                .list_for_project(&member, &private.id)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn filters_tasks_with_the_query_language() {
        let fixture = fixture().await;
        let milk = fixture
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
            .create(
                &fixture.owner,
                &fixture.workspace_id,
                &fixture.project_id,
                "call dentist",
            )
            .await
            .unwrap();

        let open = fixture
            .tasks
            .list_filtered(&fixture.owner, &fixture.project_id, "status:open")
            .await
            .unwrap();
        assert_eq!(open.len(), 2);

        fixture
            .tasks
            .set_completed(&fixture.owner, &milk.id, true)
            .await
            .unwrap();
        let done = fixture
            .tasks
            .list_filtered(&fixture.owner, &fixture.project_id, "status:done")
            .await
            .unwrap();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].title, "buy milk");

        let milk_only = fixture
            .tasks
            .list_filtered(&fixture.owner, &fixture.project_id, "text:milk")
            .await
            .unwrap();
        assert_eq!(milk_only.len(), 1);

        assert!(matches!(
            fixture
                .tasks
                .list_filtered(&fixture.owner, &fixture.project_id, "owner:ada")
                .await,
            Err(TaskError::Query(_))
        ));
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
