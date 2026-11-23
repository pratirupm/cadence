use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use serde::Deserialize;

use crate::AppState;
use crate::auth::{AuthError, AuthSession, AuthenticatedUser};
use crate::error::AppError;
use crate::tasks::Task;

const SESSION_COOKIE: &str = "cadence_session";

#[derive(Template)]
#[template(path = "landing.html")]
struct LandingTemplate;

#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: String,
}

#[derive(Template)]
#[template(path = "signup.html")]
struct SignupTemplate {
    error: String,
}

#[derive(Template)]
#[template(path = "app.html")]
struct AppTemplate {
    user_name: String,
    csrf: String,
    workspaces: Vec<WorkspaceView>,
    projects: Vec<ProjectView>,
    tasks: Vec<TaskView>,
    selected_workspace: String,
    selected_project: String,
    filter: String,
    error: String,
}

struct WorkspaceView {
    id: String,
    name: String,
    selected: bool,
}

struct ProjectView {
    id: String,
    name: String,
    selected: bool,
}

struct TaskView {
    id: String,
    workspace_id: String,
    project_id: String,
    title: String,
    completed: bool,
}

#[derive(Debug, Deserialize)]
struct SignupForm {
    name: String,
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct LoginForm {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct AppQuery {
    workspace: Option<String>,
    project: Option<String>,
    filter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateWorkspaceForm {
    csrf: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateProjectForm {
    csrf: String,
    workspace_id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateTaskForm {
    csrf: String,
    workspace_id: String,
    project_id: String,
    title: String,
}

#[derive(Debug, Deserialize)]
struct ToggleTaskForm {
    csrf: String,
    completed: bool,
    workspace_id: String,
    project_id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteTaskForm {
    csrf: String,
    workspace_id: String,
    project_id: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(landing))
        .route("/signup", get(signup_form).post(signup))
        .route("/login", get(login_form).post(login))
        .route("/logout", post(logout))
        .route("/app", get(app_page))
        .route("/workspaces", post(create_workspace))
        .route("/projects", post(create_project))
        .route("/tasks", post(create_task))
        .route("/tasks/{id}/toggle", post(toggle_task))
        .route("/tasks/{id}/delete", post(delete_task))
        .route("/static/app.css", get(styles))
}

async fn landing(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    if current_user(&state, &headers).await?.is_some() {
        return Ok(Redirect::to("/app").into_response());
    }

    render(LandingTemplate)
}

async fn login_form() -> Result<Response, AppError> {
    render(LoginTemplate {
        error: String::new(),
    })
}

async fn signup_form() -> Result<Response, AppError> {
    render(SignupTemplate {
        error: String::new(),
    })
}

async fn signup(
    State(state): State<AppState>,
    Form(form): Form<SignupForm>,
) -> Result<Response, AppError> {
    let result = state
        .auth
        .signup(&form.email, &form.name, &form.password)
        .await;

    match result {
        Ok(signup) => {
            tracing::info!(user_id = %signup.session.user.id, "account created");
            with_session(signup.session, "/app")
        }
        Err(error) => {
            let message = match error {
                AuthError::InvalidEmail => "Enter a valid email address.",
                AuthError::InvalidName => "Enter your name.",
                AuthError::WeakPassword => "Use a password with at least 12 characters.",
                AuthError::EmailTaken => "That email address is already registered.",
                _ => "Could not create the account.",
            };

            render(SignupTemplate {
                error: message.to_owned(),
            })
        }
    }
}

async fn login(
    State(state): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Result<Response, AppError> {
    match state.auth.login(&form.email, &form.password).await {
        Ok(session) => with_session(session, "/app"),
        Err(_) => render(LoginTemplate {
            error: "Invalid email or password.".to_owned(),
        }),
    }
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, AppError> {
    if let Some(token) = session_token(&headers) {
        state.auth.logout(&token).await?;
    }

    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("cadence_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"),
    );
    Ok(response)
}

async fn app_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AppQuery>,
) -> Result<Response, AppError> {
    let Some(user) = current_user(&state, &headers).await? else {
        return Ok(Redirect::to("/login").into_response());
    };

    let workspaces = state.workspaces.list_workspaces(&user.user.id).await?;
    let selected_workspace = query
        .workspace
        .filter(|id| workspaces.iter().any(|workspace| &workspace.id == id))
        .or_else(|| workspaces.first().map(|workspace| workspace.id.clone()))
        .unwrap_or_default();

    let projects = if selected_workspace.is_empty() {
        Vec::new()
    } else {
        state
            .workspaces
            .list_projects(&user.user.id, &selected_workspace)
            .await?
    };
    let selected_project = query
        .project
        .filter(|id| projects.iter().any(|project| &project.id == id))
        .or_else(|| projects.first().map(|project| project.id.clone()))
        .unwrap_or_default();

    let filter = query.filter.unwrap_or_default();
    let tasks = if selected_project.is_empty() {
        Vec::new()
    } else {
        state
            .tasks
            .list_filtered(&user.user.id, &selected_project, &filter)
            .await?
    };

    render(AppTemplate {
        user_name: user.user.name,
        csrf: user.csrf_token,
        workspaces: workspaces
            .into_iter()
            .map(|workspace| WorkspaceView {
                selected: workspace.id == selected_workspace,
                id: workspace.id,
                name: workspace.name,
            })
            .collect(),
        projects: projects
            .into_iter()
            .map(|project| ProjectView {
                selected: project.id == selected_project,
                id: project.id,
                name: project.name,
            })
            .collect(),
        tasks: tasks.into_iter().map(TaskView::from).collect(),
        selected_workspace,
        selected_project,
        filter,
        error: String::new(),
    })
}

async fn create_workspace(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CreateWorkspaceForm>,
) -> Result<Response, AppError> {
    let user = require_user(&state, &headers).await?;
    verify_csrf(&user, &form.csrf)?;

    let workspace = state
        .workspaces
        .create_workspace(&user.user.id, &form.name)
        .await?;

    Ok(Redirect::to(&format!("/app?workspace={}", workspace.id)).into_response())
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CreateProjectForm>,
) -> Result<Response, AppError> {
    let user = require_user(&state, &headers).await?;
    verify_csrf(&user, &form.csrf)?;

    let project = state
        .workspaces
        .create_project(
            &user.user.id,
            &form.workspace_id,
            &form.name,
            crate::workspaces::ProjectVisibility::Workspace,
        )
        .await?;

    Ok(Redirect::to(&format!(
        "/app?workspace={}&project={}",
        project.workspace_id, project.id
    ))
    .into_response())
}

async fn create_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CreateTaskForm>,
) -> Result<Response, AppError> {
    let user = require_user(&state, &headers).await?;
    verify_csrf(&user, &form.csrf)?;

    let task = state
        .tasks
        .create(
            &user.user.id,
            &form.workspace_id,
            &form.project_id,
            &form.title,
        )
        .await?;

    Ok(Redirect::to(&format!(
        "/app?workspace={}&project={}",
        task.workspace_id, task.project_id
    ))
    .into_response())
}

async fn toggle_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<ToggleTaskForm>,
) -> Result<Response, AppError> {
    let user = require_user(&state, &headers).await?;
    verify_csrf(&user, &form.csrf)?;

    state
        .tasks
        .set_completed(&user.user.id, &id, form.completed)
        .await?;

    Ok(Redirect::to(&format!(
        "/app?workspace={}&project={}",
        form.workspace_id, form.project_id
    ))
    .into_response())
}

async fn delete_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<DeleteTaskForm>,
) -> Result<Response, AppError> {
    let user = require_user(&state, &headers).await?;
    verify_csrf(&user, &form.csrf)?;

    state.tasks.delete(&user.user.id, &id).await?;

    Ok(Redirect::to(&format!(
        "/app?workspace={}&project={}",
        form.workspace_id, form.project_id
    ))
    .into_response())
}

async fn styles() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/app.css"),
    )
}

fn render<T: Template>(template: T) -> Result<Response, AppError> {
    let body = template.render().map_err(|error| {
        tracing::error!(%error, "template render failed");
        AppError::Internal
    })?;

    Ok(Html(body).into_response())
}

async fn current_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedUser>, AppError> {
    let Some(token) = session_token(headers) else {
        return Ok(None);
    };

    Ok(state.auth.authenticate(&token).await?)
}

async fn require_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedUser, AppError> {
    current_user(state, headers)
        .await?
        .ok_or(AppError::Unauthorized)
}

fn verify_csrf(user: &AuthenticatedUser, token: &str) -> Result<(), AppError> {
    if user.csrf_token == token {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;

    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_owned())
    })
}

fn with_session(session: AuthSession, location: &str) -> Result<Response, AppError> {
    let cookie = format!(
        "{SESSION_COOKIE}={}; Path=/; HttpOnly; SameSite=Lax",
        session.token
    );
    let mut response = Redirect::to(location).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| AppError::Internal)?,
    );

    Ok(response)
}

impl From<Task> for TaskView {
    fn from(task: Task) -> Self {
        Self {
            id: task.id,
            workspace_id: task.workspace_id,
            project_id: task.project_id,
            title: task.title,
            completed: task.completed,
        }
    }
}
