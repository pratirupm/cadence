use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredResponse {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum IdempotencyError {
    #[error("idempotency key was reused with a different request")]
    Conflict,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct IdempotencyService {
    pool: SqlitePool,
}

impl IdempotencyService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn find(
        &self,
        user_id: &str,
        key: &str,
        request_hash: &str,
    ) -> Result<Option<StoredResponse>, IdempotencyError> {
        let row = sqlx::query_as::<_, (String, i64, String, DateTime<Utc>)>(
            "SELECT request_hash, response_status, response_body, expires_at
             FROM idempotency_keys WHERE key = ? AND user_id = ?",
        )
        .bind(key)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some((stored_hash, status, body, expires_at)) = row else {
            return Ok(None);
        };
        if stored_hash != request_hash {
            return Err(IdempotencyError::Conflict);
        }
        if expires_at <= Utc::now() {
            return Ok(None);
        }

        Ok(Some(StoredResponse {
            status: status as u16,
            body,
        }))
    }

    pub async fn store(
        &self,
        user_id: &str,
        key: &str,
        request_hash: &str,
        response: &StoredResponse,
    ) -> Result<(), IdempotencyError> {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO idempotency_keys
             (key, user_id, request_hash, response_status, response_body, created_at, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT (key, user_id) DO UPDATE SET
                 request_hash = excluded.request_hash,
                 response_status = excluded.response_status,
                 response_body = excluded.response_body,
                 created_at = excluded.created_at,
                 expires_at = excluded.expires_at",
        )
        .bind(key)
        .bind(user_id)
        .bind(request_hash)
        .bind(i64::from(response.status))
        .bind(&response.body)
        .bind(now)
        .bind(now + Duration::hours(24))
        .execute(&self.pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;

    const PASSWORD: &str = "correct horse battery staple";

    async fn service_and_user() -> (IdempotencyService, String) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let user = AuthService::new(pool.clone())
            .create_user("person@example.com", "Person", PASSWORD, true)
            .await
            .unwrap()
            .id;

        (IdempotencyService::new(pool), user)
    }

    #[tokio::test]
    async fn replays_a_stored_response() {
        let (service, user) = service_and_user().await;
        let response = StoredResponse {
            status: 201,
            body: "{\"id\":\"1\"}".to_owned(),
        };

        service
            .store(&user, "key-1", "hash-1", &response)
            .await
            .unwrap();

        assert_eq!(
            service.find(&user, "key-1", "hash-1").await.unwrap(),
            Some(response)
        );
    }

    #[tokio::test]
    async fn rejects_reusing_a_key_for_a_different_request() {
        let (service, user) = service_and_user().await;
        let response = StoredResponse {
            status: 201,
            body: "{}".to_owned(),
        };
        service
            .store(&user, "key-1", "hash-1", &response)
            .await
            .unwrap();

        assert!(matches!(
            service.find(&user, "key-1", "hash-2").await,
            Err(IdempotencyError::Conflict)
        ));
    }
}
