use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use chrono::{DateTime, Duration, Utc};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use ulid::Ulid;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub password_hash: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthSession {
    pub user: User,
    pub token: String,
    pub csrf_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignupResult {
    pub session: AuthSession,
    pub verification_token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    pub user: User,
    pub csrf_token: String,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid email address")]
    InvalidEmail,
    #[error("name cannot be empty")]
    InvalidName,
    #[error("password must be at least 12 characters")]
    WeakPassword,
    #[error("email is already registered")]
    EmailTaken,
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("invalid or expired token")]
    InvalidToken,
    #[error("password hashing failed")]
    Hash,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenKind {
    Verify,
    Reset,
}

impl TokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Verify => "verify",
            Self::Reset => "reset",
        }
    }

    fn ttl(self) -> Duration {
        match self {
            Self::Verify => Duration::hours(24),
            Self::Reset => Duration::hours(1),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthService {
    pool: SqlitePool,
}

#[derive(Debug, sqlx::FromRow)]
struct SessionRecord {
    user_id: String,
    csrf_token: String,
}

impl AuthService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_user(
        &self,
        email: &str,
        name: &str,
        password: &str,
        verified: bool,
    ) -> Result<User, AuthError> {
        let email = normalize_email(email)?;
        let name = normalize_name(name)?;
        validate_password(password)?;

        let password_hash = hash_password(password)?;
        let now = Utc::now();
        let user = User {
            id: Ulid::new().to_string(),
            email,
            name,
            password_hash,
            email_verified_at: verified.then_some(now),
            created_at: now,
        };

        let result = sqlx::query(
            "INSERT INTO users (id, email, name, password_hash, email_verified_at, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&user.id)
        .bind(&user.email)
        .bind(&user.name)
        .bind(&user.password_hash)
        .bind(user.email_verified_at)
        .bind(user.created_at)
        .execute(&self.pool)
        .await;

        match result {
            Ok(_) => Ok(user),
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                Err(AuthError::EmailTaken)
            }
            Err(error) => Err(AuthError::Database(error)),
        }
    }

    pub async fn signup(
        &self,
        email: &str,
        name: &str,
        password: &str,
    ) -> Result<SignupResult, AuthError> {
        let user = self.create_user(email, name, password, false).await?;
        let session = self.create_session(&user).await?;
        let verification_token = self.issue_token(&user.id, TokenKind::Verify).await?;

        Ok(SignupResult {
            session,
            verification_token,
        })
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<AuthSession, AuthError> {
        let email = normalize_email(email)?;
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, name, password_hash, email_verified_at, created_at
             FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        let Some(user) = user else {
            return Err(AuthError::InvalidCredentials);
        };

        if !verify_password(password, &user.password_hash) {
            return Err(AuthError::InvalidCredentials);
        }

        self.create_session(&user).await
    }

    pub async fn authenticate(&self, token: &str) -> Result<Option<AuthenticatedUser>, AuthError> {
        let id = hash_token(token);
        let session = sqlx::query_as::<_, SessionRecord>(
            "SELECT user_id, csrf_token FROM sessions
             WHERE id = ? AND expires_at > ?",
        )
        .bind(id)
        .bind(Utc::now())
        .fetch_optional(&self.pool)
        .await?;

        let Some(session) = session else {
            return Ok(None);
        };

        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, name, password_hash, email_verified_at, created_at
             FROM users WHERE id = ?",
        )
        .bind(&session.user_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(user.map(|user| AuthenticatedUser {
            user,
            csrf_token: session.csrf_token,
        }))
    }

    pub async fn logout(&self, token: &str) -> Result<(), AuthError> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(hash_token(token))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn verify_email(&self, token: &str) -> Result<(), AuthError> {
        let user_id = self.consume_token(token, TokenKind::Verify).await?;

        sqlx::query("UPDATE users SET email_verified_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(user_id)
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    pub async fn request_password_reset(&self, email: &str) -> Result<Option<String>, AuthError> {
        let email = normalize_email(email)?;
        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, name, password_hash, email_verified_at, created_at
             FROM users WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;

        let Some(user) = user else {
            return Ok(None);
        };

        let token = self.issue_token(&user.id, TokenKind::Reset).await?;

        Ok(Some(token))
    }

    pub async fn reset_password(&self, token: &str, password: &str) -> Result<(), AuthError> {
        validate_password(password)?;
        let user_id = self.consume_token(token, TokenKind::Reset).await?;
        let password_hash = hash_password(password)?;

        let mut transaction = self.pool.begin().await?;
        sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
            .bind(password_hash)
            .bind(&user_id)
            .execute(&mut *transaction)
            .await?;
        sqlx::query("DELETE FROM sessions WHERE user_id = ?")
            .bind(&user_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        Ok(())
    }

    async fn create_session(&self, user: &User) -> Result<AuthSession, AuthError> {
        let token = random_token();
        let csrf_token = random_token();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO sessions (id, user_id, csrf_token, expires_at, created_at)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(hash_token(&token))
        .bind(&user.id)
        .bind(&csrf_token)
        .bind(now + Duration::days(7))
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(AuthSession {
            user: user.clone(),
            token,
            csrf_token,
        })
    }

    async fn issue_token(&self, user_id: &str, kind: TokenKind) -> Result<String, AuthError> {
        let token = random_token();
        let now = Utc::now();

        sqlx::query(
            "INSERT INTO auth_tokens (id, user_id, kind, token_hash, expires_at, used_at, created_at)
             VALUES (?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(Ulid::new().to_string())
        .bind(user_id)
        .bind(kind.as_str())
        .bind(hash_token(&token))
        .bind(now + kind.ttl())
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(token)
    }

    async fn consume_token(&self, token: &str, kind: TokenKind) -> Result<String, AuthError> {
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT id, user_id FROM auth_tokens
             WHERE token_hash = ? AND kind = ? AND used_at IS NULL AND expires_at > ?",
        )
        .bind(hash_token(token))
        .bind(kind.as_str())
        .bind(Utc::now())
        .fetch_optional(&mut *transaction)
        .await?;

        let Some((id, user_id)) = row else {
            return Err(AuthError::InvalidToken);
        };

        sqlx::query("UPDATE auth_tokens SET used_at = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        Ok(user_id)
    }
}

fn normalize_email(email: &str) -> Result<String, AuthError> {
    let email = email.trim().to_lowercase();
    let valid = email.len() >= 3
        && email.contains('@')
        && !email.starts_with('@')
        && !email.ends_with('@')
        && !email.chars().any(char::is_whitespace);

    if valid {
        Ok(email)
    } else {
        Err(AuthError::InvalidEmail)
    }
}

fn normalize_name(name: &str) -> Result<String, AuthError> {
    let name = name.trim();
    if name.is_empty() {
        Err(AuthError::InvalidName)
    } else {
        Ok(name.to_owned())
    }
}

fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.chars().count() < 12 {
        Err(AuthError::WeakPassword)
    } else {
        Ok(())
    }
}

fn hash_password(password: &str) -> Result<String, AuthError> {
    let mut salt = [0_u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|_| AuthError::Hash)?;

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Hash)
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
        .unwrap_or(false)
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    const PASSWORD: &str = "correct horse battery staple";

    async fn service() -> AuthService {
        let pool = db::connect("sqlite::memory:").await.unwrap();
        db::migrate(&pool).await.unwrap();
        AuthService::new(pool)
    }

    #[tokio::test]
    async fn signs_up_and_authenticates_a_session() {
        let service = service().await;

        let signup = service
            .signup(" Person@Example.com ", "Ada", PASSWORD)
            .await
            .unwrap();

        assert_eq!(signup.session.user.email, "person@example.com");
        assert!(signup.session.user.email_verified_at.is_none());
        assert_ne!(signup.session.user.password_hash, PASSWORD);

        let authenticated = service
            .authenticate(&signup.session.token)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(authenticated.user.id, signup.session.user.id);
        assert_eq!(authenticated.csrf_token, signup.session.csrf_token);
    }

    #[tokio::test]
    async fn rejects_a_duplicate_email() {
        let service = service().await;
        service
            .signup("person@example.com", "Ada", PASSWORD)
            .await
            .unwrap();

        let error = service
            .signup("PERSON@example.com", "Grace", PASSWORD)
            .await
            .unwrap_err();

        assert!(matches!(error, AuthError::EmailTaken));
    }

    #[tokio::test]
    async fn rejects_invalid_emails_and_weak_passwords() {
        let service = service().await;

        assert!(matches!(
            service.signup("not-an-email", "Ada", PASSWORD).await,
            Err(AuthError::InvalidEmail)
        ));
        assert!(matches!(
            service.signup("person@example.com", "Ada", "short").await,
            Err(AuthError::WeakPassword)
        ));
        assert!(matches!(
            service.signup("person@example.com", "  ", PASSWORD).await,
            Err(AuthError::InvalidName)
        ));
    }

    #[tokio::test]
    async fn logs_in_with_the_right_password_only() {
        let service = service().await;
        service
            .signup("person@example.com", "Ada", PASSWORD)
            .await
            .unwrap();

        assert!(service.login("person@example.com", PASSWORD).await.is_ok());
        assert!(matches!(
            service.login("person@example.com", "wrong password").await,
            Err(AuthError::InvalidCredentials)
        ));
        assert!(matches!(
            service.login("other@example.com", PASSWORD).await,
            Err(AuthError::InvalidCredentials)
        ));
    }

    #[tokio::test]
    async fn verifies_email_once() {
        let service = service().await;
        let signup = service
            .signup("person@example.com", "Ada", PASSWORD)
            .await
            .unwrap();

        service
            .verify_email(&signup.verification_token)
            .await
            .unwrap();

        let user = sqlx::query_as::<_, User>(
            "SELECT id, email, name, password_hash, email_verified_at, created_at
             FROM users WHERE id = ?",
        )
        .bind(&signup.session.user.id)
        .fetch_one(&service.pool)
        .await
        .unwrap();
        assert!(user.email_verified_at.is_some());

        assert!(matches!(
            service.verify_email(&signup.verification_token).await,
            Err(AuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn resets_a_password_and_revokes_sessions() {
        let service = service().await;
        let signup = service
            .signup("person@example.com", "Ada", PASSWORD)
            .await
            .unwrap();
        let token = service
            .request_password_reset("person@example.com")
            .await
            .unwrap()
            .unwrap();

        service
            .reset_password(&token, "a brand new passphrase")
            .await
            .unwrap();

        assert!(
            service
                .authenticate(&signup.session.token)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            service
                .login("person@example.com", "a brand new passphrase")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn logs_out_a_session() {
        let service = service().await;
        let signup = service
            .signup("person@example.com", "Ada", PASSWORD)
            .await
            .unwrap();

        service.logout(&signup.session.token).await.unwrap();

        assert!(
            service
                .authenticate(&signup.session.token)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn creates_a_verified_user_from_the_cli() {
        let service = service().await;

        let user = service
            .create_user("person@example.com", "Ada", PASSWORD, true)
            .await
            .unwrap();

        assert!(user.email_verified_at.is_some());
    }
}
