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
pub mod security;
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
use crate::security::LoginLimiter;
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
    pub login_limiter: Arc<LoginLimiter>,
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
            login_limiter: Arc::new(LoginLimiter::default()),
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
        .layer(middleware::from_fn(security::headers))
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
    let pending_reminders = count_rows(
        &state,
        "SELECT COUNT(*) FROM reminders WHERE status = 'pending'",
    )
    .await;
    let failed_reminders = count_rows(
        &state,
        "SELECT COUNT(*) FROM reminders WHERE status = 'failed'",
    )
    .await;
    let pending_webhooks = count_rows(
        &state,
        "SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'pending'",
    )
    .await;
    let failed_webhooks = count_rows(
        &state,
        "SELECT COUNT(*) FROM webhook_deliveries WHERE status = 'failed'",
    )
    .await;

    let mut body = state.metrics.render();
    body.push_str(&format!(
        concat!(
            "# HELP cadence_reminders_pending Pending reminders.\n",
            "# TYPE cadence_reminders_pending gauge\n",
            "cadence_reminders_pending {}\n",
            "# HELP cadence_reminders_failed Permanently failed reminders.\n",
            "# TYPE cadence_reminders_failed gauge\n",
            "cadence_reminders_failed {}\n",
            "# HELP cadence_webhook_deliveries_pending Pending webhook deliveries.\n",
            "# TYPE cadence_webhook_deliveries_pending gauge\n",
            "cadence_webhook_deliveries_pending {}\n",
            "# HELP cadence_webhook_deliveries_failed Failed webhook deliveries.\n",
            "# TYPE cadence_webhook_deliveries_failed gauge\n",
            "cadence_webhook_deliveries_failed {}\n",
        ),
        pending_reminders, failed_reminders, pending_webhooks, failed_webhooks,
    ));

    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body)
}

async fn count_rows(state: &AppState, sql: &str) -> i64 {
    match sqlx::query_scalar::<_, i64>(sql)
        .fetch_one(&state.pool)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            tracing::warn!(%error, "metrics query failed");
            0
        }
    }
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok" })
}

async fn not_found() -> AppError {
    AppError::NotFound
}
