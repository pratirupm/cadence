use std::str::FromStr;

use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::audit::{AuditError, AuditService};
use crate::auth::{hash_token, random_token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Admin,
    Member,
    Viewer,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }

    pub fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    pub fn can_write(self) -> bool {
        matches!(self, Self::Owner | Self::Admin | Self::Member)
    }
}

impl FromStr for Role {
    type Err = WorkspaceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "member" => Ok(Self::Member),
            "viewer" => Ok(Self::Viewer),
            _ => Err(WorkspaceError::InvalidRole),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectVisibility {
    Workspace,
    Private,
}

impl ProjectVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Private => "private",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectRole {
    Editor,
    Viewer,
}

impl ProjectRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::Viewer => "viewer",
        }
    }

    pub fn can_write(self) -> bool {
        self == Self::Editor
    }
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub owner_id: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Project {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub visibility: String,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub user_id: String,
    pub email: String,
    pub name: String,
    pub role: Role,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("workspace or project name cannot be empty")]
    InvalidName,
    #[error("workspace not found")]
    WorkspaceNotFound,
    #[error("project not found")]
    ProjectNotFound,
    #[error("not a member of this workspace")]
    NotMember,
    #[error("you do not have permission to do that")]
    Forbidden,
    #[error("invalid invitation")]
    InvalidInvitation,
    #[error("invitation belongs to a different email address")]
    EmailMismatch,
    #[error("invalid role")]
    InvalidRole,
    #[error(transparent)]
    Audit(#[from] AuditError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct WorkspaceService {
    pool: SqlitePool,
}

#[derive(Debug, sqlx::FromRow)]
struct MemberRow {
    user_id: String,
    email: String,
    name: String,
    role: String,
}

impl WorkspaceService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_workspace(
        &self,
        owner_id: &str,
        name: &str,
    ) -> Result<Workspace, WorkspaceError> {
        let name = normalize_name(name)?;
        let slug = self.unique_slug(&name).await?;
        let workspace = Workspace {
            id: Ulid::new().to_string(),
            name,
            slug,
            owner_id: owner_id.to_owned(),
            created_at: Utc::now(),
        };

        let mut transaction = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO workspaces (id, name, slug, owner_id, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&workspace.id)
        .bind(&workspace.name)
        .bind(&workspace.slug)
        .bind(&workspace.owner_id)
        .bind(workspace.created_at)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role, created_at)
             VALUES (?, ?, 'owner', ?)",
        )
        .bind(&workspace.id)
        .bind(owner_id)
        .bind(Utc::now())
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        self.audit()
            .record(
                &workspace.id,
                owner_id,
                "workspace.created",
                &workspace.id,
                "",
            )
            .await?;

        Ok(workspace)
    }

    pub async fn find_workspace(&self, id: &str) -> Result<Workspace, WorkspaceError> {
        sqlx::query_as::<_, Workspace>(
            "SELECT id, name, slug, owner_id, created_at FROM workspaces WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(WorkspaceError::WorkspaceNotFound)
    }

    pub async fn list_workspaces(&self, user_id: &str) -> Result<Vec<Workspace>, WorkspaceError> {
        let workspaces = sqlx::query_as::<_, Workspace>(
            "SELECT w.id, w.name, w.slug, w.owner_id, w.created_at
             FROM workspaces w
             JOIN workspace_members m ON m.workspace_id = w.id
             WHERE m.user_id = ?
             ORDER BY w.created_at, w.id",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(workspaces)
    }

    pub async fn role(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> Result<Option<Role>, WorkspaceError> {
        let role = sqlx::query_scalar::<_, String>(
            "SELECT role FROM workspace_members WHERE workspace_id = ? AND user_id = ?",
        )
        .bind(workspace_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        role.map(|role| role.parse()).transpose()
    }

    pub async fn create_project(
        &self,
        actor_id: &str,
        workspace_id: &str,
        name: &str,
        visibility: ProjectVisibility,
    ) -> Result<Project, WorkspaceError> {
        self.require_role(workspace_id, actor_id, Role::can_write)
            .await?;

        let name = normalize_name(name)?;
        let project = Project {
            id: Ulid::new().to_string(),
            workspace_id: workspace_id.to_owned(),
            name,
            visibility: visibility.as_str().to_owned(),
            archived_at: None,
            created_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO projects (id, workspace_id, name, visibility, archived_at, created_at)
             VALUES (?, ?, ?, ?, NULL, ?)",
        )
        .bind(&project.id)
        .bind(&project.workspace_id)
        .bind(&project.name)
        .bind(&project.visibility)
        .bind(project.created_at)
        .execute(&self.pool)
        .await?;
        self.audit()
            .record(
                workspace_id,
                actor_id,
                "project.created",
                &project.id,
                visibility.as_str(),
            )
            .await?;

        Ok(project)
    }

    pub async fn list_projects(
        &self,
        actor_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<Project>, WorkspaceError> {
        self.require_member(workspace_id, actor_id).await?;

        let projects = sqlx::query_as::<_, Project>(
            "SELECT p.id, p.workspace_id, p.name, p.visibility, p.archived_at, p.created_at
             FROM projects p
             WHERE p.workspace_id = ? AND p.archived_at IS NULL
               AND (
                   p.visibility = 'workspace'
                   OR EXISTS (
                       SELECT 1 FROM project_members pm
                       WHERE pm.project_id = p.id AND pm.user_id = ?
                   )
               )
             ORDER BY p.created_at, p.id",
        )
        .bind(workspace_id)
        .bind(actor_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(projects)
    }

    pub async fn add_member(
        &self,
        actor_id: &str,
        workspace_id: &str,
        user_id: &str,
        role: Role,
    ) -> Result<(), WorkspaceError> {
        let actor_role = self.require_member(workspace_id, actor_id).await?;
        if !actor_role.can_manage() || (role == Role::Owner && actor_role != Role::Owner) {
            return Err(WorkspaceError::Forbidden);
        }

        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(workspace_id)
        .bind(user_id)
        .bind(role.as_str())
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        self.audit()
            .record(
                workspace_id,
                actor_id,
                "member.added",
                user_id,
                role.as_str(),
            )
            .await?;

        Ok(())
    }

    pub async fn list_members(
        &self,
        actor_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<Member>, WorkspaceError> {
        self.require_member(workspace_id, actor_id).await?;

        let rows = sqlx::query_as::<_, MemberRow>(
            "SELECT u.id AS user_id, u.email, u.name, m.role
             FROM workspace_members m
             JOIN users u ON u.id = m.user_id
             WHERE m.workspace_id = ?
             ORDER BY u.name, u.id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                Ok(Member {
                    user_id: row.user_id,
                    email: row.email,
                    name: row.name,
                    role: row.role.parse()?,
                })
            })
            .collect()
    }

    pub async fn invite(
        &self,
        actor_id: &str,
        workspace_id: &str,
        email: &str,
        role: Role,
    ) -> Result<String, WorkspaceError> {
        let actor_role = self.require_member(workspace_id, actor_id).await?;
        if !actor_role.can_manage() || role == Role::Owner {
            return Err(WorkspaceError::Forbidden);
        }

        let email = normalize_email(email)?;
        let token = random_token();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO invitations
             (id, workspace_id, email, role, token_hash, expires_at, accepted_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(Ulid::new().to_string())
        .bind(workspace_id)
        .bind(&email)
        .bind(role.as_str())
        .bind(hash_token(&token))
        .bind(now + Duration::days(7))
        .bind(now)
        .execute(&self.pool)
        .await?;
        self.audit()
            .record(
                workspace_id,
                actor_id,
                "invitation.created",
                &email,
                role.as_str(),
            )
            .await?;

        Ok(token)
    }

    pub async fn accept_invitation(
        &self,
        token: &str,
        user_id: &str,
    ) -> Result<String, WorkspaceError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT id, workspace_id, email, role FROM invitations
             WHERE token_hash = ? AND accepted_at IS NULL AND expires_at > ?",
        )
        .bind(hash_token(token))
        .bind(Utc::now())
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(WorkspaceError::InvalidInvitation)?;

        let (invitation_id, workspace_id, email, role) = row;
        let user_email = sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or(WorkspaceError::InvalidInvitation)?;

        if user_email != email {
            return Err(WorkspaceError::EmailMismatch);
        }

        sqlx::query(
            "INSERT INTO workspace_members (workspace_id, user_id, role, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (workspace_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(&workspace_id)
        .bind(user_id)
        .bind(&role)
        .bind(Utc::now())
        .execute(&mut *transaction)
        .await?;

        sqlx::query("UPDATE invitations SET accepted_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(invitation_id)
            .execute(&mut *transaction)
            .await?;

        transaction.commit().await?;
        self.audit()
            .record(
                &workspace_id,
                user_id,
                "invitation.accepted",
                &workspace_id,
                &role,
            )
            .await?;

        Ok(workspace_id)
    }

    pub async fn add_project_member(
        &self,
        actor_id: &str,
        project_id: &str,
        user_id: &str,
        role: ProjectRole,
    ) -> Result<(), WorkspaceError> {
        let project = self.find_project(project_id).await?;
        self.require_role(&project.workspace_id, actor_id, Role::can_manage)
            .await?;
        if self.role(&project.workspace_id, user_id).await?.is_none() {
            return Err(WorkspaceError::NotMember);
        }

        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, role, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT (project_id, user_id) DO UPDATE SET role = excluded.role",
        )
        .bind(project_id)
        .bind(user_id)
        .bind(role.as_str())
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        self.audit()
            .record(
                &project.workspace_id,
                actor_id,
                "project_member.added",
                user_id,
                role.as_str(),
            )
            .await?;

        Ok(())
    }

    pub async fn can_access_project(
        &self,
        user_id: &str,
        project_id: &str,
    ) -> Result<bool, WorkspaceError> {
        let project = self.find_project(project_id).await?;
        let Some(role) = self.role(&project.workspace_id, user_id).await? else {
            return Ok(false);
        };
        if role.can_manage() || project.visibility == "workspace" {
            return Ok(true);
        }

        let member = sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM project_members WHERE project_id = ? AND user_id = ? LIMIT 1",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .is_some();

        Ok(member)
    }

    pub async fn project_role(
        &self,
        project_id: &str,
        user_id: &str,
    ) -> Result<Option<ProjectRole>, WorkspaceError> {
        let role = sqlx::query_scalar::<_, String>(
            "SELECT role FROM project_members WHERE project_id = ? AND user_id = ?",
        )
        .bind(project_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(role.and_then(|role| match role.as_str() {
            "editor" => Some(ProjectRole::Editor),
            "viewer" => Some(ProjectRole::Viewer),
            _ => None,
        }))
    }

    async fn find_project(&self, project_id: &str) -> Result<Project, WorkspaceError> {
        sqlx::query_as::<_, Project>(
            "SELECT id, workspace_id, name, visibility, archived_at, created_at
             FROM projects WHERE id = ? AND archived_at IS NULL",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(WorkspaceError::ProjectNotFound)
    }

    fn audit(&self) -> AuditService {
        AuditService::new(self.pool.clone())
    }

    async fn require_member(
        &self,
        workspace_id: &str,
        user_id: &str,
    ) -> Result<Role, WorkspaceError> {
        self.role(workspace_id, user_id)
            .await?
            .ok_or(WorkspaceError::NotMember)
    }

    async fn require_role(
        &self,
        workspace_id: &str,
        user_id: &str,
        allowed: fn(Role) -> bool,
    ) -> Result<Role, WorkspaceError> {
        let role = self.require_member(workspace_id, user_id).await?;
        if allowed(role) {
            Ok(role)
        } else {
            Err(WorkspaceError::Forbidden)
        }
    }

    async fn unique_slug(&self, name: &str) -> Result<String, WorkspaceError> {
        let base = slugify(name);

        for suffix in 1..=100 {
            let candidate = if suffix == 1 {
                base.clone()
            } else {
                format!("{base}-{suffix}")
            };
            let exists =
                sqlx::query_scalar::<_, i64>("SELECT 1 FROM workspaces WHERE slug = ? LIMIT 1")
                    .bind(&candidate)
                    .fetch_optional(&self.pool)
                    .await?
                    .is_some();

            if !exists {
                return Ok(candidate);
            }
        }

        Err(WorkspaceError::InvalidName)
    }
}

fn normalize_name(name: &str) -> Result<String, WorkspaceError> {
    let name = name.trim();
    if name.is_empty() {
        Err(WorkspaceError::InvalidName)
    } else {
        Ok(name.to_owned())
    }
}

fn normalize_email(email: &str) -> Result<String, WorkspaceError> {
    let email = email.trim().to_lowercase();
    if email.contains('@') {
        Ok(email)
    } else {
        Err(WorkspaceError::InvalidInvitation)
    }
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;

    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }

    while slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "workspace".to_owned()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;

    const PASSWORD: &str = "correct horse battery staple";

    async fn fixture() -> (AuthService, WorkspaceService) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        (AuthService::new(pool.clone()), WorkspaceService::new(pool))
    }

    async fn user(auth: &AuthService, email: &str) -> String {
        auth.create_user(email, "Person", PASSWORD, true)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn creates_a_workspace_with_an_owner() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;

        let workspace = workspaces
            .create_workspace(&owner, "Home Street")
            .await
            .unwrap();

        assert_eq!(workspace.slug, "home-street");
        assert_eq!(
            workspaces.role(&workspace.id, &owner).await.unwrap(),
            Some(Role::Owner)
        );
        assert_eq!(
            workspaces.list_workspaces(&owner).await.unwrap(),
            [workspace]
        );
    }

    #[tokio::test]
    async fn makes_duplicate_slugs_unique() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;

        let first = workspaces.create_workspace(&owner, "Home").await.unwrap();
        let second = workspaces.create_workspace(&owner, "Home").await.unwrap();

        assert_eq!(first.slug, "home");
        assert_eq!(second.slug, "home-2");
    }

    #[tokio::test]
    async fn members_can_create_projects_but_viewers_cannot() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;
        let member = user(&auth, "member@example.com").await;
        let viewer = user(&auth, "viewer@example.com").await;
        let workspace = workspaces.create_workspace(&owner, "Home").await.unwrap();

        workspaces
            .add_member(&owner, &workspace.id, &member, Role::Member)
            .await
            .unwrap();
        workspaces
            .add_member(&owner, &workspace.id, &viewer, Role::Viewer)
            .await
            .unwrap();

        let project = workspaces
            .create_project(
                &member,
                &workspace.id,
                "Chores",
                ProjectVisibility::Workspace,
            )
            .await
            .unwrap();
        assert_eq!(project.name, "Chores");

        assert!(matches!(
            workspaces
                .create_project(&viewer, &workspace.id, "Nope", ProjectVisibility::Workspace)
                .await,
            Err(WorkspaceError::Forbidden)
        ));
    }

    #[tokio::test]
    async fn hides_private_projects_from_non_members() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;
        let member = user(&auth, "member@example.com").await;
        let workspace = workspaces.create_workspace(&owner, "Home").await.unwrap();
        workspaces
            .add_member(&owner, &workspace.id, &member, Role::Member)
            .await
            .unwrap();

        let private = workspaces
            .create_project(&owner, &workspace.id, "Private", ProjectVisibility::Private)
            .await
            .unwrap();
        let shared = workspaces
            .create_project(
                &owner,
                &workspace.id,
                "Shared",
                ProjectVisibility::Workspace,
            )
            .await
            .unwrap();

        let visible = workspaces
            .list_projects(&member, &workspace.id)
            .await
            .unwrap();
        assert_eq!(visible, [shared]);

        sqlx::query(
            "INSERT INTO project_members (project_id, user_id, role, created_at)
             VALUES (?, ?, 'viewer', ?)",
        )
        .bind(&private.id)
        .bind(&member)
        .bind(Utc::now())
        .execute(&workspaces.pool)
        .await
        .unwrap();

        let visible = workspaces
            .list_projects(&member, &workspace.id)
            .await
            .unwrap();
        assert_eq!(visible.len(), 2);
    }

    #[tokio::test]
    async fn invitations_require_the_matching_email_and_work_once() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;
        let invited = user(&auth, "invited@example.com").await;
        let other = user(&auth, "other@example.com").await;
        let workspace = workspaces.create_workspace(&owner, "Home").await.unwrap();

        let token = workspaces
            .invite(&owner, &workspace.id, "invited@example.com", Role::Member)
            .await
            .unwrap();

        assert!(matches!(
            workspaces.accept_invitation(&token, &other).await,
            Err(WorkspaceError::EmailMismatch)
        ));

        let accepted = workspaces
            .accept_invitation(&token, &invited)
            .await
            .unwrap();
        assert_eq!(accepted, workspace.id);
        assert_eq!(
            workspaces.role(&workspace.id, &invited).await.unwrap(),
            Some(Role::Member)
        );

        assert!(matches!(
            workspaces.accept_invitation(&token, &invited).await,
            Err(WorkspaceError::InvalidInvitation)
        ));
    }

    #[tokio::test]
    async fn viewers_cannot_add_members() {
        let (auth, workspaces) = fixture().await;
        let owner = user(&auth, "owner@example.com").await;
        let viewer = user(&auth, "viewer@example.com").await;
        let target = user(&auth, "target@example.com").await;
        let workspace = workspaces.create_workspace(&owner, "Home").await.unwrap();
        workspaces
            .add_member(&owner, &workspace.id, &viewer, Role::Viewer)
            .await
            .unwrap();

        assert!(matches!(
            workspaces
                .add_member(&viewer, &workspace.id, &target, Role::Member)
                .await,
            Err(WorkspaceError::Forbidden)
        ));
    }
}
