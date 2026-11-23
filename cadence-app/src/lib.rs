pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod tasks;
pub mod telemetry;

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct AppState {
    pub pool: SqlitePool,
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

/// Build the HTTP application.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .fallback(not_found)
        .with_state(state)
        .layer(TraceLayer::new_for_http())
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn not_found() -> AppError {
    AppError::NotFound
}
