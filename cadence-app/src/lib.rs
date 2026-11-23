pub mod cli;
pub mod config;
pub mod error;
pub mod telemetry;

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Serialize;
use tower_http::trace::TraceLayer;

use crate::error::AppError;

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

/// Build the HTTP application.
pub fn app() -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn not_found() -> AppError {
    AppError::NotFound
}
