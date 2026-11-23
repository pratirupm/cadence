use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use serde::Serialize;
use thiserror::Error;

use crate::auth::AuthError;
use crate::idempotency::IdempotencyError;
use crate::sync::SyncError;
use crate::tasks::TaskError;
use crate::workspaces::WorkspaceError;

/// Errors that can become an HTTP response.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("authentication required")]
    Unauthorized,
    #[error("API authentication required")]
    ApiUnauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("internal error")]
    Internal,
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Task(#[from] TaskError),
    #[error(transparent)]
    Idempotency(#[from] IdempotencyError),
    #[error(transparent)]
    Sync(#[from] SyncError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl AppError {
    fn status_and_message(&self) -> (StatusCode, &'static str) {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "authentication required"),
            Self::ApiUnauthorized => (StatusCode::UNAUTHORIZED, "API authentication required"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
            Self::Auth(error) => match error {
                AuthError::InvalidEmail => (StatusCode::BAD_REQUEST, "invalid email address"),
                AuthError::InvalidName => (StatusCode::BAD_REQUEST, "invalid name"),
                AuthError::WeakPassword => (StatusCode::BAD_REQUEST, "weak password"),
                AuthError::EmailTaken => (StatusCode::CONFLICT, "email already registered"),
                AuthError::InvalidCredentials => (StatusCode::UNAUTHORIZED, "invalid credentials"),
                AuthError::InvalidToken => (StatusCode::BAD_REQUEST, "invalid token"),
                AuthError::Hash | AuthError::Database(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
                }
            },
            Self::Workspace(error) => match error {
                WorkspaceError::WorkspaceNotFound => (StatusCode::NOT_FOUND, "workspace not found"),
                WorkspaceError::ProjectNotFound => (StatusCode::NOT_FOUND, "project not found"),
                WorkspaceError::NotMember | WorkspaceError::Forbidden => {
                    (StatusCode::FORBIDDEN, "forbidden")
                }
                WorkspaceError::InvalidName => (StatusCode::BAD_REQUEST, "invalid name"),
                WorkspaceError::InvalidInvitation | WorkspaceError::EmailMismatch => {
                    (StatusCode::BAD_REQUEST, "invalid invitation")
                }
                WorkspaceError::InvalidRole => (StatusCode::BAD_REQUEST, "invalid role"),
                WorkspaceError::Audit(_) | WorkspaceError::Database(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
                }
            },
            Self::Task(error) => match error {
                TaskError::NotFound => (StatusCode::NOT_FOUND, "task not found"),
                TaskError::ProjectNotFound => (StatusCode::NOT_FOUND, "project not found"),
                TaskError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
                TaskError::EmptyTitle => (StatusCode::BAD_REQUEST, "task title cannot be empty"),
                TaskError::Conflict => (StatusCode::CONFLICT, "task changed since it was loaded"),
                TaskError::Query(_) => (StatusCode::BAD_REQUEST, "invalid task filter"),
                TaskError::Recurrence(_) => (StatusCode::BAD_REQUEST, "invalid recurrence rule"),
                TaskError::Workspace(error) => match error {
                    WorkspaceError::NotMember | WorkspaceError::Forbidden => {
                        (StatusCode::FORBIDDEN, "forbidden")
                    }
                    _ => (StatusCode::BAD_REQUEST, "workspace error"),
                },
                TaskError::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
            },
            Self::Idempotency(error) => match error {
                IdempotencyError::Conflict => (
                    StatusCode::CONFLICT,
                    "idempotency key reused for a different request",
                ),
                IdempotencyError::Database(_) => {
                    (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
                }
            },
            Self::Sync(SyncError::Database(_) | SyncError::Json(_)) => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error")
            }
            Self::Sync(SyncError::Task(error)) => match error {
                TaskError::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
                TaskError::NotFound => (StatusCode::NOT_FOUND, "task not found"),
                TaskError::Conflict => (StatusCode::CONFLICT, "task changed since it was loaded"),
                _ => (StatusCode::BAD_REQUEST, "sync error"),
            },
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if matches!(self, Self::Unauthorized) {
            return Redirect::to("/login").into_response();
        }

        let (status, message) = self.status_and_message();
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
