use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;
use ulid::Ulid;

use crate::auth::{hash_token, random_token};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Write,
    Admin,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    fn allows(self, required: Self) -> bool {
        match self {
            Self::Admin => true,
            Self::Write => matches!(required, Self::Read | Self::Write),
            Self::Read => required == Self::Read,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSummary {
    pub id: String,
    pub name: String,
    pub scopes: Vec<Scope>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUser {
    pub user_id: String,
    pub scopes: Vec<Scope>,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("invalid token")]
    Invalid,
    #[error("token does not have the required scope")]
    Forbidden,
    #[error("invalid scope: {0}")]
    InvalidScope(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct TokenService {
    pool: SqlitePool,
}

#[derive(Debug, sqlx::FromRow)]
struct TokenRow {
    user_id: String,
    scopes: String,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
}

impl TokenService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        user_id: &str,
        name: &str,
        scopes: &[Scope],
        expires_in: Option<Duration>,
    ) -> Result<(String, TokenSummary), TokenError> {
        if name.trim().is_empty() {
            return Err(TokenError::InvalidScope("name".to_owned()));
        }
        if scopes.is_empty() {
            return Err(TokenError::InvalidScope("at least one scope".to_owned()));
        }

        let raw = format!("cad_{}", random_token());
        let now = Utc::now();
        let expires_at = expires_in.map(|duration| now + duration);
        let summary = TokenSummary {
            id: Ulid::new().to_string(),
            name: name.trim().to_owned(),
            scopes: scopes.to_vec(),
            expires_at,
            last_used_at: None,
            created_at: now,
        };
        let scope_text = scopes
            .iter()
            .map(|scope| scope.as_str())
            .collect::<Vec<_>>()
            .join(",");

        sqlx::query(
            "INSERT INTO api_tokens
             (id, user_id, name, token_hash, scopes, expires_at, revoked_at, last_used_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?)",
        )
        .bind(&summary.id)
        .bind(user_id)
        .bind(&summary.name)
        .bind(hash_token(&raw))
        .bind(scope_text)
        .bind(expires_at)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok((raw, summary))
    }

    pub async fn list(&self, user_id: &str) -> Result<Vec<TokenSummary>, TokenError> {
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                Option<DateTime<Utc>>,
                Option<DateTime<Utc>>,
                DateTime<Utc>,
            ),
        >(
            "SELECT id, name, scopes, expires_at, last_used_at, created_at
             FROM api_tokens
             WHERE user_id = ? AND revoked_at IS NULL
             ORDER BY created_at DESC, id DESC",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(id, name, scopes, expires_at, last_used_at, created_at)| {
                Ok(TokenSummary {
                    id,
                    name,
                    scopes: parse_scopes(&scopes)?,
                    expires_at,
                    last_used_at,
                    created_at,
                })
            })
            .collect()
    }

    pub async fn revoke(&self, user_id: &str, id: &str) -> Result<bool, TokenError> {
        let result = sqlx::query(
            "UPDATE api_tokens SET revoked_at = ?
             WHERE id = ? AND user_id = ? AND revoked_at IS NULL",
        )
        .bind(Utc::now())
        .bind(id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() == 1)
    }

    pub async fn authenticate(&self, raw: &str, required: Scope) -> Result<TokenUser, TokenError> {
        let row = sqlx::query_as::<_, TokenRow>(
            "SELECT user_id, scopes, expires_at, revoked_at
             FROM api_tokens WHERE token_hash = ?",
        )
        .bind(hash_token(raw))
        .fetch_optional(&self.pool)
        .await?
        .ok_or(TokenError::Invalid)?;

        if row.revoked_at.is_some() {
            return Err(TokenError::Invalid);
        }
        if let Some(expires_at) = row.expires_at
            && expires_at <= Utc::now()
        {
            return Err(TokenError::Invalid);
        }

        let scopes = parse_scopes(&row.scopes)?;
        if !scopes.iter().any(|scope| scope.allows(required)) {
            return Err(TokenError::Forbidden);
        }

        sqlx::query("UPDATE api_tokens SET last_used_at = ? WHERE user_id = ? AND token_hash = ?")
            .bind(Utc::now())
            .bind(&row.user_id)
            .bind(hash_token(raw))
            .execute(&self.pool)
            .await?;

        Ok(TokenUser {
            user_id: row.user_id,
            scopes,
        })
    }
}

fn parse_scopes(value: &str) -> Result<Vec<Scope>, TokenError> {
    value
        .split(',')
        .map(|scope| Scope::parse(scope).ok_or_else(|| TokenError::InvalidScope(scope.to_owned())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthService;
    use crate::db;

    const PASSWORD: &str = "correct horse battery staple";

    async fn service_and_user() -> (TokenService, String) {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        let user = AuthService::new(pool.clone())
            .create_user("person@example.com", "Person", PASSWORD, true)
            .await
            .unwrap()
            .id;

        (TokenService::new(pool), user)
    }

    #[tokio::test]
    async fn creates_and_authenticates_a_token() {
        let (tokens, user) = service_and_user().await;
        let (raw, summary) = tokens
            .create(&user, "CLI", &[Scope::Read, Scope::Write], None)
            .await
            .unwrap();

        let authenticated = tokens.authenticate(&raw, Scope::Write).await.unwrap();
        assert_eq!(authenticated.user_id, user);
        assert_eq!(authenticated.scopes, [Scope::Read, Scope::Write]);
        assert_eq!(tokens.list(&user).await.unwrap()[0].id, summary.id);
    }

    #[tokio::test]
    async fn enforces_scopes_and_revocation() {
        let (tokens, user) = service_and_user().await;
        let (raw, summary) = tokens
            .create(&user, "Reader", &[Scope::Read], None)
            .await
            .unwrap();

        assert!(matches!(
            tokens.authenticate(&raw, Scope::Write).await,
            Err(TokenError::Forbidden)
        ));
        assert!(tokens.revoke(&user, &summary.id).await.unwrap());
        assert!(matches!(
            tokens.authenticate(&raw, Scope::Read).await,
            Err(TokenError::Invalid)
        ));
    }

    #[tokio::test]
    async fn rejects_expired_tokens() {
        let (tokens, user) = service_and_user().await;
        let (raw, _) = tokens
            .create(&user, "Old", &[Scope::Read], Some(Duration::seconds(-1)))
            .await
            .unwrap();

        assert!(matches!(
            tokens.authenticate(&raw, Scope::Read).await,
            Err(TokenError::Invalid)
        ));
    }
}
