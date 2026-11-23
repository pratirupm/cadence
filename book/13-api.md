# 13. The API

**What you can do at the end.** Use a scoped bearer token to list and create
tasks over JSON, replay a request safely with an idempotency key, and see the
same task only once.

**The Rust you learn.** Extractors, bearer authentication, token scopes,
idempotency, JSON error responses, and separating API errors from browser
redirects.

**Build it.**

- Add `api_tokens` with hashed tokens, scopes, expiry, and revocation.
- Add `ApiUser` as a `FromRequestParts` extractor.
- Require `Read` or `Write` for each handler; `Admin` implies both.
- Add `/api/v1/me`, task list/create, and sync endpoints.
- Store idempotent responses by `(key, user)` and request hash.
- Reject a reused key with a different request body.
- Return `401` JSON for API auth failures, not a redirect to `/login`.

The extractor is the gate:

```rust
impl FromRequestParts<AppState> for ApiUser {
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState)
        -> Result<Self, Self::Rejection> {
        // read Bearer, authenticate, return user + scopes
    }
}
```

**Checkpoint.**

```console
$ cargo test --test api
$ cargo run -- token create --email you@example.com --name CLI --scopes read,write
```

The tests cover missing tokens, read-only tokens on writes, idempotent replay,
and the list after create.

**Exercises.**

1. Add a `DELETE /api/v1/tasks/{id}` with the same version check.
2. Add rate limiting per token and a test for the 429 response.
