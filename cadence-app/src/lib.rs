pub mod api;
pub mod audit;
pub mod auth;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod events;
pub mod idempotency;
pub mod mailer;
pub mod metrics;
pub mod notifications;
pub mod query;
pub mod recurrence;
pub mod reminders;
pub mod sync;
pub mod tasks;
pub mod telemetry;
pub mod tokens;
pub mod web;
pub mod webhooks;
pub mod workspaces;

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use serde::Serialize;
use sqlx::SqlitePool;
use tower_http::trace::TraceLayer;

use crate::auth::AuthService;
use crate::error::AppError;
use crate::events::EventBus;
use crate::metrics::Metrics;
use crate::tasks::TaskService;
use crate::tokens::TokenService;
use crate::webhooks::WebhookService;
use crate::workspaces::WorkspaceService;

#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub tasks: TaskService,
    pub tokens: TokenService,
    pub webhooks: WebhookService,
    pub events: EventBus,
    pub metrics: Arc<Metrics>,
}

impl AppState {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            auth: AuthService::new(pool.clone()),
            workspaces: WorkspaceService::new(pool.clone()),
            tasks: TaskService::new(pool.clone()),
            tokens: TokenService::new(pool.clone()),
            webhooks: WebhookService::new(pool.clone()),
            events: EventBus::new(256),
            metrics: Arc::new(Metrics::default()),
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
        .merge(api::router())
        .route("/health", get(health))
        .route("/metrics", get(metrics))
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn_with_state(
            state.metrics.clone(),
            count_requests,
        ))
        .with_state(state)
}

async fn count_requests(
    State(metrics): State<Arc<Metrics>>,
    request: Request,
    next: Next,
) -> Response {
    metrics.inc(&metrics.requests);
    let response = next.run(request).await;
    if response.status().is_server_error() {
        metrics.inc(&metrics.errors);
    }
    response
}

async fn metrics(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        state.metrics.render(),
    )
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn not_found() -> AppError {
    AppError::NotFound
}
