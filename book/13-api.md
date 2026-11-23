# 13. The API

The browser and the tests can use the service, but a program cannot. The CLI
needs a stable way to list and create tasks. A mobile client, a script, or a
future integration needs the same. The MPA’s HTML is not an API; it is a
rendering of one.

This chapter adds a versioned JSON API with bearer tokens, scopes, and
idempotency keys. The important constraint is that the API is not a second
permission system. It authenticates a different kind of caller, then calls the
same task, workspace, and sync services. A token does not grant access to a
workspace; the membership tables still do.

By the end of this chapter you can:

- create a scoped API token and show it once
- authenticate `Authorization: Bearer ...`
- list and create tasks over JSON
- reject a missing token with `401` and a read-only token write with `403`
- replay a create with the same `Idempotency-Key` and get the same task
- reject a reused key with a different request body
- explain why API auth failures return JSON instead of redirecting to `/login`

The Rust lessons are `FromRequestParts`, bearer parsing, scope predicates,
idempotency records, and returning a response instead of a typed JSON body. The
API lessons are token lifecycle, least-privilege scopes, and safe retries.

## Sessions versus tokens

The browser uses a session cookie. The API uses a bearer token. They are
different on purpose:

| | Browser session | API token |
| --- | --- | --- |
| Transport | HttpOnly cookie | `Authorization: Bearer` |
| Lifetime | Seven days | Optional expiry |
| Revocation | Delete session row | Revoke token row |
| CSRF | Required | Not applicable |
| Scope | The whole account | Read/write/admin |
| Audience | One browser | Scripts and clients |

A session is tied to a browser and protected by `SameSite` and CSRF. A bearer
token is a capability the client stores and sends. It does not benefit from
cookies, so CSRF does not apply; it does need careful storage and revocation.

Do not reuse the session token as an API token. The lifetimes and threat models
differ, and a browser logout should not have to revoke a CLI’s token.

## Token storage

Create `migrations/0009_api_tokens.sql`:

```sql
CREATE TABLE api_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes TEXT NOT NULL,
    expires_at TEXT,
    revoked_at TEXT,
    last_used_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX api_tokens_user_id_idx ON api_tokens (user_id);

CREATE TABLE idempotency_keys (
    key TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    request_hash TEXT NOT NULL,
    response_status INTEGER NOT NULL,
    response_body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    PRIMARY KEY (key, user_id)
);
```

`token_hash` stores SHA-256 of the raw token, not the token. The raw value is
returned once at creation. If the database leaks, the attacker has hashes of
high-entropy tokens, not usable bearers.

`scopes` is comma-separated text such as `read,write`. It is not a separate join
table because the set is tiny and the token is the only owner. A future scope
catalog might justify a table; this one does not.

`expires_at` is nullable. The CLI creates a token with no expiry for local use;
a production flow should choose a finite lifetime and a rotation policy.

`revoked_at` is a soft revoke. The row stays for audit and the token hash cannot
be reused. Authentication checks `revoked_at IS NULL`.

`idempotency_keys` is keyed by `(key, user_id)`, so two users can use the same
key string without colliding. `request_hash` detects a key reused for a different
body. `expires_at` bounds the table; 24 hours is enough for a client retry.

## Scopes

```rust
pub enum Scope {
    Read,
    Write,
    Admin,
}

impl Scope {
    fn allows(self, required: Self) -> bool {
        match self {
            Self::Admin => true,
            Self::Write => matches!(required, Self::Read | Self::Write),
            Self::Read => required == Self::Read,
        }
    }
}
```

`Admin` implies both `Read` and `Write`. `Write` implies `Read`. `Read` implies
only itself. The hierarchy is intentionally one-directional; there is no
“write-only” scope because a client that can create a task usually needs to read
the result.

The API extractor parses the token’s scopes and then checks the handler’s
requirement. A token with `read` can call `GET`; a token with `write` can call
both; a token with `admin` can manage webhooks and still cannot bypass workspace
membership.

`TokenService::create` returns the raw token and a summary:

```rust
let (raw, summary) = state.tokens
    .create(&user_id, "CLI", &[Scope::Read, Scope::Write], None)
    .await?;
println!("{} {}", summary.id, raw);
```

The CLI prints the raw token once. Do not log it in the server. The `summary`
contains the ID, name, scopes, expiry, and timestamps — never the raw token or
its hash.

`authenticate` hashes the incoming token, looks up an unrevoked, unexpired row,
checks the required scope, and updates `last_used_at`:

```rust
if row.revoked_at.is_some() { return Err(TokenError::Invalid); }
if let Some(expires_at) = row.expires_at && expires_at <= Utc::now() {
    return Err(TokenError::Invalid);
}

let scopes = parse_scopes(&row.scopes)?;
if !scopes.iter().any(|scope| scope.allows(required)) {
    return Err(TokenError::Forbidden);
}
```

An invalid token and an expired token both return `Invalid`; the caller should
not learn which. A valid token with the wrong scope returns `Forbidden`, which is
safe to expose because the token was already proven valid.

## The extractor

`ApiUser` implements `FromRequestParts<AppState>`:

```rust
impl FromRequestParts<AppState> for ApiUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .ok_or(AppError::ApiUnauthorized)?;

        match state.tokens.authenticate(header, Scope::Read).await {
            Ok(user) => Ok(Self {
                user_id: user.user_id,
                scopes: user.scopes,
            }),
            Err(TokenError::Forbidden) => Err(AppError::Forbidden),
            Err(_) => Err(AppError::ApiUnauthorized),
        }
    }
}
```

The extractor always authenticates with `Read` first; that proves the token is
valid and loads its scopes. The handler then calls `user.require(Scope::Write)`
for mutations. This avoids parsing the token twice and keeps the scope decision
in the handler where the required level is visible.

`AppError::ApiUnauthorized` maps to `401` JSON. `AppError::Unauthorized` maps to
a redirect to `/login`, which is correct for the MPA and wrong for an API. The
two variants exist because the same “no session” condition has two audiences.
Do not solve this by inspecting `Accept` headers; the route knows its audience.

A missing header, a malformed header, an unknown token, and an expired token all
produce `ApiUnauthorized`. That is deliberate: the client learns “authenticate
again,” not which check failed.

## Routes

The API is versioned under `/api/v1`:

```rust
Router::new()
    .route("/api/v1/me", get(me))
    .route("/api/v1/sync", get(sync).post(push_sync))
    .route("/api/v1/events", get(events))
    .route(
        "/api/v1/workspaces/{workspace_id}/projects/{project_id}/tasks",
        get(list_tasks).post(create_task),
    )
    .route(
        "/api/v1/workspaces/{workspace_id}/webhooks",
        get(list_webhooks).post(create_webhook),
    )
```

The version is in the path, not a header. A breaking change gets `/api/v2`; a
new optional field does not. The API does not share the MPA’s routes, so the
browser and the client can evolve separately.

`GET /api/v1/me` returns the user ID and scope names. It is the smallest
authenticated endpoint and a useful health check for a client.

The task list takes `?filter=` and reuses the DSL:

```rust
let tasks = state.tasks
    .list_filtered(&user.user_id, &project_id, query.filter.as_deref().unwrap_or(""))
    .await?;
```

The path also contains a `workspace_id`, but the list handler ignores it. The
service authorizes through the project’s workspace. That is safe but sloppy as a
contract: a client can send the wrong workspace and still get results. A strict
handler compares the path workspace to the project’s workspace and returns 404
on a mismatch. The exercise asks for it.

## Idempotent creates

A network can fail after the server commits but before the client reads the
response. The client retries. Without idempotency, the user gets two tasks.

`create_task` reads an optional header:

```rust
let key = headers
    .get("idempotency-key")
    .and_then(|value| value.to_str().ok())
    .map(str::to_owned);
let request_hash = hash_token(&format!(
    "{workspace_id}:{project_id}:{}",
    body.title
));
```

The request hash covers the fields that change the result. If the same key
arrives with a different title, it is not a retry; it is a client bug or an
attack. `IdempotencyService::find` returns `Conflict` for that case.

Before creating:

```rust
if let Some(key) = &key
    && let Some(stored) = idempotency.find(&user.user_id, key, &request_hash).await?
{
    return Ok(stored_response(stored));
}
```

`stored_response` returns the recorded status and body with a JSON content type.
The replay is indistinguishable from the first response except for timing and
headers. That is the point.

After creating, the response is stored for 24 hours:

```rust
idempotency.store(
    &user.user_id,
    &key,
    &request_hash,
    &StoredResponse {
        status: StatusCode::CREATED.as_u16(),
        body: body.clone(),
    },
).await?;
```

The store is not in the same transaction as the task insert. A crash between
“insert task” and “store idempotency” means a retry creates a second task. The
fix is a transaction or a unique constraint on a request ID that the task table
can enforce. The current code is honest about the gap; the exercise asks you to
close it.

The key is scoped to the user, not the token. Rotating a token does not let a
client replay another user’s key. It also means two tokens for the same user
share the namespace, which is convenient for a CLI and a web client.

## Authorization is still the service

A valid token with `write` does not automatically allow a write. The handler
calls `TaskService::create`, which checks workspace write permission and project
access:

```text
bearer token -> scope check -> handler -> TaskService -> membership + project role
```

The scope is the API’s coarse gate. The service is the tenant’s fine gate. A
`write` token for a user who is only a workspace viewer still gets `Forbidden`.

The same applies to webhooks: `admin` scope plus a workspace admin role. The API
does not invent a second role system.

## Error shapes

`AppError` renders JSON:

```json
{"error":"forbidden"}
```

The API does not return HTML error pages, and it does not redirect. A 401 means
“authenticate”; a 403 means “authenticated, not allowed”; a 404 means “not
found or not visible.” The messages are terse on purpose; the details belong in
logs, not in a response that may be shown to an attacker.

A JSON deserialization failure is handled by axum before the handler and returns
a 422 with a plain rejection body, not the `AppError` shape. If the API contract
requires a consistent error envelope, add a custom `Json` extractor rejection.
The tests currently exercise valid JSON and missing tokens, not malformed bodies.

## Scope matrix

The token scope is a coarse gate. The workspace role is still checked.

| Route | Required scope | Additional service check |
| --- | --- | --- |
| `GET /api/v1/me` | read | valid token |
| `GET /tasks` | read | workspace member + project read |
| `POST /tasks` | write | workspace write + project write |
| `POST /sync` | write | per-operation task access |
| `GET /webhooks` | admin | workspace admin |
| `POST /webhooks` | admin | workspace admin |
| `GET /events` | read | workspace membership filter |

A `write` token does not satisfy `admin` because `Scope::Write` does not imply
`Admin`. An `admin` token satisfies read and write. A `read` token cannot create,
even if the user is an owner. If that feels too strict, it is still the right
default for a programmable credential: the token should carry the narrowest
authority the client needs.

The scope matrix is also a test plan. For each mutating route, add a read-token
request and assert 403. The current API tests do this for task creation; the
sync and webhook routes need the same.

## Worked idempotent retry

```text
POST /tasks  Idempotency-Key: k1  {"title":"buy milk"}
201 {"id":"01J...","title":"buy milk",...}

   (client times out, retries)

POST /tasks  Idempotency-Key: k1  {"title":"buy milk"}
201 {"id":"01J...","title":"buy milk",...}   same ID, same body

POST /tasks  Idempotency-Key: k1  {"title":"buy oat milk"}
409 {"error":"idempotency key reused for a different request"}
```

The second request does not execute `TaskService::create`. It finds the stored
response and returns it. The third request finds the key but the request hash
differs, so it fails before touching the task table.

If the first response was a `422` or `403`, the API does not store it. The client
can retry after fixing the request. Only successful creates are currently stored;
a more complete implementation caches every terminal response for the key.

## Tests

The token service has three tests:

- `creates_and_authenticates_a_token`: raw token works, scopes round-trip,
  summary appears in the list.
- `enforces_scopes_and_revocation`: read token cannot write; revoked token is
  invalid.
- `rejects_expired_tokens`: a negative duration creates an already-expired token.

The idempotency service has two:

- `replays_a_stored_response`: same key and hash returns the stored response.
- `rejects_reusing_a_key_for_a_different_request`: same key, different hash is
  `Conflict`.

The API integration test has two:

- `token_auth_lists_and_creates_tasks_idempotently`: `/me`, create twice with the
  same key, assert identical bodies, list contains one task.
- `rejects_missing_and_scopeless_tokens`: no bearer is 401; a read-only token
  posting a task is 403.

Run:

```console
$ cargo test tokens
$ cargo test idempotency
$ cargo test --test api
```

## Checkpoint

```console
$ cargo run -- token create --email you@example.com --name CLI --scopes read,write
$ cargo test --test api
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

The CLI prints an ID and a raw token. Store the raw token in the client; the
server cannot show it again.

## Failure modes you will meet

**`401` with a valid-looking token.** The token was revoked, expired, or belongs
to a different database. Check `revoked_at`, `expires_at`, and the database path.

**`403` on a write with a `write` token.** The token is fine; the user lacks
workspace write or project access. The API scope is not the tenant role.

**The same key creates two tasks.** The response was not stored, the store failed,
or the key was not sent. Check the header and the 24-hour window.

**A reused key returns `409`.** The request hash changed: different title,
workspace, or project. A retry must send exactly the same body.

**The API redirects to `/login`.** The handler returned `AppError::Unauthorized`
instead of `ApiUnauthorized`. The browser variant redirects; the API variant must
not.

**A read token can mutate through the sync endpoint.** The handler forgot
`user.require(Scope::Write)`. Test every mutating route, not just task create.

**The token appears in a log.** The CLI prints it once; the server must never log
the raw header. Redact `Authorization` in tracing middleware.

**A path workspace does not match the project.** The list handler ignores the
path workspace. Compare it to the project’s workspace if the contract promises
they agree.

## Under the hood

`FromRequestParts` runs before the handler and can reject with a response. It has
access to headers and state, not the body. That is why the token check is an
extractor and the JSON body is a separate `Json<T>` argument.

Bearer authentication is a shared secret. TLS protects it in transit; hashing
protects it at rest; revocation protects it after a leak. There is no signature
or proof-of-possession. A stolen bearer is usable until it expires or is revoked.

Idempotency is a server-side memo keyed by a client-chosen string. The server
must store enough of the response to replay it and enough of the request to
detect a collision. The 24-hour window is a policy, not a law.

## Exercises

1. **Validate the path workspace.** In `list_tasks`, load the project and compare
   its workspace to the path. Return 404 on a mismatch. Add a test.
2. **Add `DELETE /tasks/{id}`.** Require `write`, use the same version check, and
   return 204. Test a successful delete and a stale version.
3. **Make the idempotency store atomic.** Insert the task and the idempotency
   record in one transaction. Add a test that a failure after task insert leaves
   no task.
4. **Add token rotation.** Create a new token in the same scope, revoke the old
   one, and test that the old token stops working immediately.
5. **Add a token/IP rate limit.** Return 429 with `Retry-After`, and test that a
   burst is bounded. Do not rate-limit health checks.

## What comes next

The API can create tasks, but it cannot tell a client what changed since its last
request. Chapter 14 adds the workspace change log: a monotonic sequence, trigger
payloads, tombstones, and the cursor endpoint the CLI will pull.
