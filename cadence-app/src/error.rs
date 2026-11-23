use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use thiserror::Error;

/// Errors that can become an HTTP response.
#[derive(Debug, Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl AppError {
    fn status_and_message(&self) -> (StatusCode, &'static str) {
        match self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = self.status_and_message();
        (status, Json(ErrorBody { error: message })).into_response()
    }
}
