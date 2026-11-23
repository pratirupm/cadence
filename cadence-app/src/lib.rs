pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod mailer;
pub mod notifications;
pub mod query;
pub mod recurrence;
pub mod reminders;
pub mod tasks;
pub mod telemetry;
pub mod web;
pub mod workspaces;

use axum::Json;
use axum::Router;
use axum::routing::get;
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use crate::auth::AuthService;
use crate::error::AppError;
use crate::tasks::TaskService;
use crate::workspaces::WorkspaceService;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub tasks: TaskService,
}

impl AppState {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            auth: AuthService::new(pool.clone()),
            workspaces: WorkspaceService::new(pool.clone()),
            tasks: TaskService::new(pool.clone()),
            pool,
        }
    }
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
}

/// Build the HTTP application.
pub fn app(state: AppState) -> Router {
    Router::new()
        .merge(web::router())
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
