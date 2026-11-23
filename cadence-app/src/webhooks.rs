use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use reqwest::Url;
use sha2::Sha256;
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::auth::random_token;
use crate::workspaces::{Role, WorkspaceError, WorkspaceService};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct Webhook {
    pub id: String,
    pub workspace_id: String,
    pub url: String,
    pub secret: String,
    pub events: String,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("webhook URL must be HTTPS and public")]
    InvalidUrl,
    #[error("webhook not found")]
    NotFound,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct WebhookService {
    pool: SqlitePool,
}

impl WebhookService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        actor_id: &str,
        workspace_id: &str,
        url: &str,
        events: &[String],
    ) -> Result<Webhook, WebhookError> {
        let role = WorkspaceService::new(self.pool.clone())
            .role(workspace_id, actor_id)
            .await?
            .ok_or(WebhookError::InvalidUrl)?;
        if !role.can_manage() {
            return Err(WebhookError::Workspace(WorkspaceError::Forbidden));
        }
        validate_url(url)?;

        let webhook = Webhook {
            id: Ulid::new().to_string(),
            workspace_id: workspace_id.to_owned(),
            url: url.to_owned(),
            secret: random_token(),
            events: events.join(","),
            active: true,
            created_at: Utc::now(),
        };

        sqlx::query(
            "INSERT INTO webhooks (id, workspace_id, url, secret, events, active, created_at)
             VALUES (?, ?, ?, ?, ?, 1, ?)",
        )
        .bind(&webhook.id)
        .bind(&webhook.workspace_id)
        .bind(&webhook.url)
        .bind(&webhook.secret)
        .bind(&webhook.events)
        .bind(webhook.created_at)
        .execute(&self.pool)
        .await?;

        Ok(webhook)
    }

    pub async fn list(
        &self,
        actor_id: &str,
        workspace_id: &str,
    ) -> Result<Vec<Webhook>, WebhookError> {
        let role = WorkspaceService::new(self.pool.clone())
            .role(workspace_id, actor_id)
            .await?
            .ok_or(WebhookError::InvalidUrl)?;
        if role == Role::Viewer {
            return Err(WebhookError::Workspace(WorkspaceError::Forbidden));
        }

        let webhooks = sqlx::query_as::<_, Webhook>(
            "SELECT id, workspace_id, url, secret, events, active, created_at
             FROM webhooks WHERE workspace_id = ? AND active = 1
             ORDER BY created_at, id",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(webhooks)
    }

    pub async fn delete(
        &self,
        actor_id: &str,
        workspace_id: &str,
        id: &str,
    ) -> Result<bool, WebhookError> {
        let role = WorkspaceService::new(self.pool.clone())
            .role(workspace_id, actor_id)
            .await?
            .ok_or(WebhookError::InvalidUrl)?;
        if !role.can_manage() {
            return Err(WebhookError::Workspace(WorkspaceError::Forbidden));
        }

        let result = sqlx::query(
            "UPDATE webhooks SET active = 0 WHERE id = ? AND workspace_id = ? AND active = 1",
        )
        .bind(id)
        .bind(workspace_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn dispatch(
        &self,
        workspace_id: &str,
        event: &str,
        payload: &str,
    ) -> Result<usize, WebhookError> {
        let webhooks = sqlx::query_as::<_, Webhook>(
            "SELECT id, workspace_id, url, secret, events, active, created_at
             FROM webhooks WHERE workspace_id = ? AND active = 1",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await?;

        let now = Utc::now();
        let mut count = 0;
        for webhook in webhooks {
            if !webhook
                .events
                .split(',')
                .any(|subscribed| subscribed == event)
            {
                continue;
            }

            sqlx::query(
                "INSERT INTO webhook_deliveries
                 (id, webhook_id, event, payload, status, attempts, next_at, response_status,
                  response_body, created_at)
                 VALUES (?, ?, ?, ?, 'pending', 0, ?, NULL, NULL, ?)",
            )
            .bind(Ulid::new().to_string())
            .bind(&webhook.id)
            .bind(event)
            .bind(payload)
            .bind(now)
            .bind(now)
            .execute(&self.pool)
            .await?;
            count += 1;
        }

        Ok(count)
    }

    pub async fn deliver_due(&self, now: DateTime<Utc>, limit: i64) -> Result<usize, WebhookError> {
        let rows = sqlx::query_as::<_, (String, String, String, String, i64)>(
            "SELECT d.id, d.payload, w.url, w.secret, d.attempts
             FROM webhook_deliveries d
             JOIN webhooks w ON w.id = d.webhook_id
             WHERE d.status = 'pending' AND d.next_at <= ?
             ORDER BY d.next_at LIMIT ?",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        let client = reqwest::Client::new();
        let count = rows.len();
        for (id, payload, url, secret, attempts) in rows {
            let signature = sign(&secret, &payload);
            let result = client
                .post(&url)
                .header("content-type", "application/json")
                .header("x-cadence-signature", format!("sha256={signature}"))
                .body(payload)
                .send()
                .await;

            match result {
                Ok(response) if response.status().is_success() => {
                    sqlx::query(
                        "UPDATE webhook_deliveries
                         SET status = 'delivered', attempts = attempts + 1,
                             response_status = ?
                         WHERE id = ?",
                    )
                    .bind(i64::from(response.status().as_u16()))
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
                }
                Ok(response) => {
                    self.retry(&id, attempts, now, response.status().as_u16())
                        .await?;
                }
                Err(error) => {
                    self.retry(&id, attempts, now, 0).await?;
                    tracing::warn!(%error, delivery = %id, "webhook delivery failed");
                }
            }
        }

        Ok(count)
    }

    async fn retry(
        &self,
        id: &str,
        attempts: i64,
        now: DateTime<Utc>,
        status: u16,
    ) -> Result<(), WebhookError> {
        if attempts + 1 >= 5 {
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET status = 'failed', attempts = attempts + 1, response_status = ?
                 WHERE id = ?",
            )
            .bind(i64::from(status))
            .bind(id)
            .execute(&self.pool)
            .await?;
        } else {
            let delay = Duration::minutes((attempts + 1) * (attempts + 1));
            sqlx::query(
                "UPDATE webhook_deliveries
                 SET attempts = attempts + 1, next_at = ?, response_status = ?
                 WHERE id = ?",
            )
            .bind(now + delay)
            .bind(i64::from(status))
            .bind(id)
            .execute(&self.pool)
            .await?;
        }

        Ok(())
    }
}

pub async fn run_forever(pool: SqlitePool) -> Result<(), WebhookError> {
    let service = WebhookService::new(pool);
    let mut ticker = tokio::time::interval(tokio::time::Duration::from_secs(30));

    loop {
        ticker.tick().await;
        if let Err(error) = service.deliver_due(Utc::now(), 50).await {
            tracing::error!(%error, "webhook worker failed");
        }
    }
}

pub fn sign(secret: &str, payload: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

fn validate_url(url: &str) -> Result<(), WebhookError> {
    let url = Url::parse(url).map_err(|_| WebhookError::InvalidUrl)?;
    if url.scheme() != "https" {
        return Err(WebhookError::InvalidUrl);
    }
    let host = url.host_str().ok_or(WebhookError::InvalidUrl)?;
    if host == "localhost"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host == "::1"
        || host.ends_with(".local")
    {
        return Err(WebhookError::InvalidUrl);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;

    const PASSWORD: &str = "correct horse battery staple";

    async fn workspace() -> (WebhookService, String, String) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let user = AuthService::new(pool.clone())
            .create_user("owner@example.com", "Owner", PASSWORD, true)
            .await
            .unwrap()
            .id;
        let workspace = WorkspaceService::new(pool.clone())
            .create_workspace(&user, "Home")
            .await
            .unwrap();

        (WebhookService::new(pool), user, workspace.id)
    }

    #[tokio::test]
    async fn creates_lists_and_dispatches_webhooks() {
        let (webhooks, user, workspace) = workspace().await;
        let webhook = webhooks
            .create(
                &user,
                &workspace,
                "https://example.com/hook",
                &["task.created".to_owned()],
            )
            .await
            .unwrap();

        assert_eq!(webhooks.list(&user, &workspace).await.unwrap(), [webhook]);
        assert_eq!(
            webhooks
                .dispatch(&workspace, "task.created", "{\"id\":\"1\"}")
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            webhooks
                .dispatch(&workspace, "task.deleted", "{\"id\":\"1\"}")
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn rejects_private_and_non_https_urls() {
        let (webhooks, user, workspace) = workspace().await;

        for url in ["http://example.com/hook", "https://127.0.0.1/hook"] {
            assert!(matches!(
                webhooks.create(&user, &workspace, url, &[]).await,
                Err(WebhookError::InvalidUrl)
            ));
        }
    }

    #[test]
    fn signs_a_payload() {
        let signature = sign("secret", "{\"id\":\"1\"}");

        assert_eq!(signature.len(), 64);
        assert_eq!(signature, sign("secret", "{\"id\":\"1\"}"));
        assert_ne!(signature, sign("other", "{\"id\":\"1\"}"));
    }
}
