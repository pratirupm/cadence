use axum::Json;
use axum::Router;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::auth::hash_token;
use crate::error::AppError;
use crate::idempotency::StoredResponse;
use crate::tokens::{Scope, TokenError};

#[derive(Debug, Clone)]
pub struct ApiUser {
    pub user_id: String,
    pub scopes: Vec<Scope>,
}

impl ApiUser {
    fn require(&self, scope: Scope) -> Result<(), AppError> {
        if self.scopes.iter().any(|granted| match granted {
            Scope::Admin => true,
            Scope::Write => matches!(scope, Scope::Read | Scope::Write),
            Scope::Read => scope == Scope::Read,
        }) {
            Ok(())
        } else {
            Err(AppError::Forbidden)
        }
    }
}

impl FromRequestParts<AppState> for ApiUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(AppError::ApiUnauthorized)?;

        match state.tokens.authenticate(header, Scope::Read).await {
            Ok(user) => Ok(Self {
                user_id: user.user_id,
                scopes: user.scopes,
            }),
            Err(TokenError::Forbidden) => Err(AppError::Forbidden),
            Err(_) => Err(AppError::ApiUnauthorized),
        }
    }
}

#[derive(Debug, Deserialize)]
struct TaskQuery {
    filter: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateTaskRequest {
    title: String,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/v1/me", get(me)).route(
        "/api/v1/workspaces/{workspace_id}/projects/{project_id}/tasks",
        get(list_tasks).post(create_task),
    )
}

async fn me(user: ApiUser) -> Json<Value> {
    Json(json!({
        "user_id": user.user_id,
        "scopes": user.scopes.iter().map(|scope| scope.as_str()).collect::<Vec<_>>(),
    }))
}

async fn list_tasks(
    State(state): State<AppState>,
    user: ApiUser,
    Path((_workspace_id, project_id)): Path<(String, String)>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<crate::tasks::Task>>, AppError> {
    user.require(Scope::Read)?;
    let tasks = state
        .tasks
        .list_filtered(
            &user.user_id,
            &project_id,
            query.filter.as_deref().unwrap_or(""),
        )
        .await?;

    Ok(Json(tasks))
}

async fn create_task(
    State(state): State<AppState>,
    user: ApiUser,
    Path((workspace_id, project_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<CreateTaskRequest>,
) -> Result<Response, AppError> {
    user.require(Scope::Write)?;

    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let request_hash = hash_token(&format!("{workspace_id}:{project_id}:{}", body.title));
    let idempotency = crate::idempotency::IdempotencyService::new(state.pool.clone());

    if let Some(key) = &key
        && let Some(stored) = idempotency.find(&user.user_id, key, &request_hash).await?
    {
        return Ok(stored_response(stored));
    }

    let task = state
        .tasks
        .create(&user.user_id, &workspace_id, &project_id, &body.title)
        .await?;
    let body = serde_json::to_string(&task).map_err(|_| AppError::Internal)?;

    if let Some(key) = key {
        idempotency
            .store(
                &user.user_id,
                &key,
                &request_hash,
                &StoredResponse {
                    status: StatusCode::CREATED.as_u16(),
                    body: body.clone(),
                },
            )
            .await?;
    }

    Ok((
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response())
}

fn stored_response(stored: StoredResponse) -> Response {
    (
        StatusCode::from_u16(stored.status).unwrap_or(StatusCode::OK),
        [(header::CONTENT_TYPE, "application/json")],
        stored.body,
    )
        .into_response()
}
