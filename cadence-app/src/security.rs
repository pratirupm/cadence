use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::header;
use axum::middleware::Next;
use axum::response::Response;

const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_ATTEMPTS: usize = 5;

#[derive(Debug, Default)]
pub struct LoginLimiter {
    attempts: Mutex<HashMap<String, Vec<Instant>>>,
}

impl LoginLimiter {
    pub fn allow(&self, key: &str) -> bool {
        let mut attempts = self.attempts.lock().expect("login limiter lock");
        let now = Instant::now();
        let entries = attempts.entry(key.to_owned()).or_default();
        entries.retain(|attempt| now.duration_since(*attempt) < WINDOW);
        entries.len() < MAX_ATTEMPTS
    }

    pub fn record_failure(&self, key: &str) {
        let mut attempts = self.attempts.lock().expect("login limiter lock");
        attempts
            .entry(key.to_owned())
            .or_default()
            .push(Instant::now());
    }

    pub fn clear(&self, key: &str) {
        self.attempts
            .lock()
            .expect("login limiter lock")
            .remove(key);
    }
}

pub async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self'; \
             script-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("same-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_repeated_failures() {
        let limiter = LoginLimiter::default();
        for _ in 0..MAX_ATTEMPTS {
            assert!(limiter.allow("person@example.com"));
            limiter.record_failure("person@example.com");
        }
        assert!(!limiter.allow("person@example.com"));

        limiter.clear("person@example.com");
        assert!(limiter.allow("person@example.com"));
    }
}
