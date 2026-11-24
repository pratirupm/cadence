---
title: "Accounts"
description: "Add Argon2 passwords, sessions, verification tokens, and password resets."
---

The service can store tasks, but it cannot yet answer the first question every
multi-user program must answer: **who is asking?** A task list without an owner
is a shared whiteboard. A task list with an owner is a product.

This chapter adds identity. It does not add workspaces or permissions yet. It
adds the two pieces every later permission check depends on: a user record and a
session that proves a request belongs to that user.

By the end of this chapter you can:

- create an account with a name, email, and password
- log in and receive a session token
- authenticate a request from that token
- log out and make the token stop working
- issue and consume an email-verification token once
- request a password reset without revealing whether an email exists
- reset a password and revoke every existing session
- create a verified user from the command line

The Rust lessons are password hashing, random tokens, hashing tokens at rest,
session expiry, single-use records, transactions, and the difference between
authentication and authorization. The security lessons are more important:
never store a password, never store a session token in plaintext, never let a
reset token work twice, and never let an error message tell an attacker which
emails are registered.

## The threat model

Start with what can go wrong, because every line of this chapter is a response
to one of these:

- Someone reads the database file and learns every password.
- Someone reads the database file and steals every logged-in session.
- Someone reuses a verification or reset link after it was used.
- Someone enumerates registered email addresses through login or reset errors.
- Someone keeps an old session after a password reset.
- Someone submits a 3-character password and calls it authentication.

The design answers them in order:

- passwords are hashed with Argon2id, never stored or logged
- session tokens are random and stored only as SHA-256 hashes
- verification and reset tokens are single-use rows with expiry
- unknown emails and wrong passwords produce the same login error
- reset revokes every session for the user
- passwords have a minimum length and a hard validation error

There is no “forgot password” email yet. The service issues the token; the mail
adapter arrives in chapter 11. Until then, the tests can carry the token; the
CLI does not expose a verify/reset command.

## The schema

Create `migrations/0002_accounts.sql`:

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY NOT NULL,
    email TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    email_verified_at TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE sessions (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    csrf_token TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX sessions_user_id_idx ON sessions (user_id);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);

CREATE TABLE auth_tokens (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('verify', 'reset')),
    token_hash TEXT NOT NULL UNIQUE,
    expires_at TEXT NOT NULL,
    used_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX auth_tokens_user_id_idx ON auth_tokens (user_id);
```

The `email` uniqueness constraint is the database’s promise. The service does
not rely on a read-before-write duplicate check, because two concurrent signups
could both pass it. `create_user` inserts and classifies the database’s unique
violation as `EmailTaken`. The index is what settles the race.

`sessions.id` stores the **hash** of the session token, not the token. The
column name is `id` because a session is identified by its verifier; the raw
token exists only in the client’s cookie. If the database leaks, the attacker
gets hashes, not usable sessions.

`sessions.csrf_token` is generated with the session. Chapter 6 puts it in forms
and checks it on POST. Storing it beside the session keeps the check simple.

`auth_tokens.used_at` is nullable. A row is valid only while `used_at IS NULL`
and `expires_at > now`. Consuming a token sets `used_at` in the same transaction
that finds it. That is the single-use guarantee.

`kind` is constrained to `verify` or `reset`. If a future feature adds an
invitation token, it gets its own table or its own kind and a migration. Do not
use one untyped token column for unrelated meanings.

## Password hashing

Add the dependencies:

```toml
argon2 = "0.5"
rand = "0.9"
sha2 = "0.10"
hex = "0.4"
```

Argon2 is a password-hashing function designed to be expensive in memory and
time. It is not a general-purpose hash. Do not use SHA-256 for passwords; it is
fast, which is exactly what an attacker wants. Argon2id is the hybrid variant
recommended for password storage.

The hashing code is small on purpose:

```rust
fn hash_password(password: &str) -> Result<String, AuthError> {
    let mut salt = [0_u8; 16];
    rand::rng().fill_bytes(&mut salt);
    let salt = SaltString::encode_b64(&salt).map_err(|_| AuthError::Hash)?;

    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Hash)
}
```

Each password gets a fresh 16-byte salt. The encoded hash string contains the
algorithm parameters, the salt, and the digest. You do not need separate columns
for salt or parameters; the PHC string carries them.

Verification parses the stored string and asks Argon2 to check the password:

```rust
fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|hash| {
            Argon2::default()
                .verify_password(password.as_bytes(), &hash)
                .is_ok()
        })
        .unwrap_or(false)
}
```

A malformed stored hash is treated as a failed login, not a server error. It is
a data problem, but the user-facing contract is still “invalid credentials.” The
server can log the malformed hash separately if needed; do not leak it.

Argon2 is deliberately slow. A login test that hashes several passwords takes
noticeably longer than a pure unit test. Do not “speed it up” by lowering the
parameters in production code. If tests become too slow, use a test-only hasher
with weaker parameters and keep the production defaults.

## Password policy

The policy in this chapter is intentionally simple:

```rust
fn validate_password(password: &str) -> Result<(), AuthError> {
    if password.chars().count() < 12 {
        Err(AuthError::WeakPassword)
    } else {
        Ok(())
    }
}
```

Twelve characters is a length floor, not a complexity rule. Length is the
property that matters most against offline guessing. A future chapter can add a
breach-list check or a passphrase meter, but it must not replace length with
“one uppercase, one symbol” theatre.

`chars().count()` counts Unicode scalar values, not bytes. A password with
non-ASCII characters is measured by characters. The hash input is still bytes;
that is fine.

The validation happens before hashing. A rejected password does not consume CPU
or create a row.

## The types

The `User` type is the row the service needs:

```rust
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct User {
    pub id: String,
    pub email: String,
    pub name: String,
    pub password_hash: String,
    pub email_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}
```

`password_hash` is part of the persistence type because login needs it. It must
never be part of an API response. Later chapters will define a public user view
without it, or serialize selected fields. A type that contains a secret is not
automatically safe to return.

`email_verified_at` is optional. `None` means “signed up, not verified.” The
chapter does not block login on verification; it records the state so later
workspaces can require it for invitations. This is a product decision, and it
should be explicit. Blocking login entirely would make the first local run
annoying and would require email delivery before the reader can use the app.

The session result separates the user from the token:

```rust
pub struct AuthSession {
    pub user: User,
    pub token: String,
    pub csrf_token: String,
}
```

The raw `token` is returned once to the caller. The database stores its hash.
The `csrf_token` is also returned so the web layer can put it in a form. Neither
field belongs in a log line.

`SignupResult` adds the verification token:

```rust
pub struct SignupResult {
    pub session: AuthSession,
    pub verification_token: String,
}
```

The verification token is not part of the session because it is not used for
authentication. It is a one-time capability for a different action.

`AuthError` is the service’s failure set:

```rust
pub enum AuthError {
    InvalidEmail,
    InvalidName,
    WeakPassword,
    EmailTaken,
    InvalidCredentials,
    InvalidToken,
    Hash,
    Database(sqlx::Error),
}
```

The HTTP layer maps these to status codes in `error.rs`. The service does not
know what a 401 is. That separation lets the CLI reuse the same service without
inventing HTTP responses.

## Creating a user

`create_user` normalizes the input, validates the password, hashes it, and
inserts:

```rust
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

    // insert ...
}
```

Normalization happens before validation:

```rust
fn normalize_email(email: &str) -> Result<String, AuthError> {
    let email = email.trim().to_lowercase();
    let valid = email.len() >= 3
        && email.contains('@')
        && !email.starts_with('@')
        && !email.ends_with('@')
        && !email.chars().any(char::is_whitespace);

    if valid { Ok(email) } else { Err(AuthError::InvalidEmail) }
}
```

Lowercasing is not a complete email specification. It is a pragmatic choice for
this product: `Person@Example.com` and `person@example.com` must not create two
accounts. The unique index enforces the normalized form. If you later support
case-sensitive local parts, this is the function to revisit.

`normalize_name` trims and rejects empty names. `validate_password` rejects short
passwords. All three run before the insert.

The insert handles the unique constraint:

```rust
match result {
    Ok(_) => Ok(user),
    Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
        Err(AuthError::EmailTaken)
    }
    Err(error) => Err(AuthError::Database(error)),
}
```

Do not match on the error string. `is_unique_violation()` is portable across the
drivers SQLx supports. The distinction matters: a duplicate email is a 409, a
database outage is a 500.

The `verified` flag exists for the CLI. `cadence user create --verified` creates
an account that can be used before an email provider exists. The public signup
path passes `false`.

## Signup

Signup composes three operations:

```rust
pub async fn signup(&self, email: &str, name: &str, password: &str)
    -> Result<SignupResult, AuthError>
{
    let user = self.create_user(email, name, password, false).await?;
    let session = self.create_session(&user).await?;
    let verification_token = self.issue_token(&user.id, TokenKind::Verify).await?;

    Ok(SignupResult { session, verification_token })
}
```

If `create_user` fails, no session and no token are created. If session creation
fails, the user exists but the caller gets an error. That is not a transaction
across three independent concerns, and it is acceptable at this scale: the user
can log in and request another verification token. If the product needs
“signup is all-or-nothing,” wrap the three inserts in one transaction. The
chapter keeps the pieces visible; the exercise asks you to make the stronger
guarantee.

`create_session` is private. It is only called after a user has been created or
successfully authenticated.

## Sessions

A session token is 32 random bytes, hex-encoded:

```rust
pub(crate) fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
```

Sixty-four hex characters is long enough to make guessing infeasible. The token
is a bearer capability: whoever has it can act as the user. Treat it like a
password.

The database stores only the hash:

```rust
pub(crate) fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
```

SHA-256 is fine here because the token already has 256 bits of entropy. You do
not need Argon2 for a random capability; you need a one-way function so a
database leak does not yield usable tokens. The raw token is returned to the
client once and never stored server-side.

`create_session` inserts the hashed ID, the user ID, the CSRF token, and a
seven-day expiry:

```rust
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
```

The raw token goes into `AuthSession` and from there into the cookie in chapter
6. The CSRF token is stored raw; it is not a bearer credential for the account,
but it still should not be logged.

`authenticate` hashes the incoming token and looks up an unexpired session:

```rust
let id = hash_token(token);
let session = sqlx::query_as::<_, SessionRecord>(
    "SELECT user_id, csrf_token FROM sessions
     WHERE id = ? AND expires_at > ?",
)
.bind(id)
.bind(Utc::now())
.fetch_optional(&self.pool)
.await?;
```

If the session exists, the user is loaded. If not, the result is `None`, not an
error. A missing or expired session is a normal anonymous request. The caller
decides whether that is allowed.

`logout` deletes the row:

```rust
sqlx::query("DELETE FROM sessions WHERE id = ?")
    .bind(hash_token(token))
    .execute(&self.pool)
    .await?;
```

Deleting is better than marking a session “logged out” for this design: there is
no audit requirement for old sessions, and a missing row cannot be accidentally
accepted by a query that forgets a status filter.

## Login

Login must not reveal whether an email exists. Both outcomes return the same
error:

```rust
pub async fn login(&self, email: &str, password: &str) -> Result<AuthSession, AuthError> {
    let email = normalize_email(email)?;
    let user = sqlx::query_as::<_, User>(/* ... */)
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
```

There is still a timing side channel: a missing user returns before the Argon2
verification, while a wrong password pays the hashing cost. A production login
can verify against a dummy hash when the user is missing to equalize the work.
For this book the error shape is the important lesson; the exercise asks you to
close the timing gap.

Login is allowed for unverified users. Verification is a separate state that
later workspace rules can require. If you want to block it now, check
`user.email_verified_at` after the password check and return a distinct error.
Be explicit about the product choice.

## Verification and reset tokens

Both flows use the same table and the same single-use machinery. The `TokenKind`
enum keeps the two purposes apart:

```rust
enum TokenKind { Verify, Reset }
```

`issue_token` generates a raw token, stores its hash, and sets a TTL: 24 hours
for verification, 1 hour for reset.

`consume_token` is the interesting part:

```rust
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
```

The transaction prevents two consumers from both seeing `used_at IS NULL`. The
second waits, then sees no row. On SQLite the writer lock provides the same
guarantee in practice; the explicit transaction makes the intent portable.

`verify_email` consumes a verify token and sets `email_verified_at`.
`reset_password` consumes a reset token, validates the new password, updates the
hash, and deletes every session for the user in one transaction:

```rust
let mut transaction = self.pool.begin().await?;
sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?")
    // ...
sqlx::query("DELETE FROM sessions WHERE user_id = ?")
    // ...
transaction.commit().await?;
```

Revoking sessions is not optional. If an attacker had a session and the user
resets the password to recover, leaving old sessions alive defeats the reset.
Any future “log out everywhere” feature is the same operation.

`request_password_reset` returns `Option<String>`. `None` means “no such account
or no token.” The HTTP layer should render the same response either way. The
service deliberately does not return `UserNotFound`, because that would be an
enumeration oracle.

## The CLI

Local administration needs a way to create the first account:

```console
$ cargo run -- user create \
    --email you@example.com \
    --name "You" \
    --password "correct horse battery staple" \
    --verified
```

The CLI connects to the database, migrates, builds `AuthService`, and calls
`create_user(..., true)`. The password comes from a flag or
`CADENCE_PASSWORD`; the flag is visible in shell history, so production use
should prefer the environment variable or a prompt. The book keeps the flag for
clarity.

The CLI prints the user ID and email. It never prints the password hash.

## Tests

The auth suite uses the same in-memory database fixture as chapter 2:

```rust
async fn service() -> AuthService {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    AuthService::new(pool)
}
```

Eight tests cover the contract:

- `signs_up_and_authenticates_a_session`: email is normalized, the password hash
  is not the password, `authenticate` returns the same user and CSRF token.
- `rejects_a_duplicate_email`: case-insensitive duplicate returns `EmailTaken`.
- `rejects_invalid_emails_and_weak_passwords`: each validation error is distinct.
- `logs_in_with_the_right_password_only`: correct password succeeds; wrong
  password and unknown email return the same `InvalidCredentials`.
- `verifies_email_once`: the flag is set, and the same token fails the second
  time.
- `resets_a_password_and_revokes_sessions`: the old session is gone, the new
  password works.
- `logs_out_a_session`: logout deletes the session.
- `creates_a_verified_user_from_the_cli`: the admin path sets the verification
  timestamp.

Run them:

```console
$ cargo test auth
```

The tests are slow-ish because Argon2 is doing real work. That is evidence the
production path is not mocked.

## Checkpoint

```console
$ cargo run -- migrate
$ cargo run -- user create --email you@example.com --name You \
    --password "correct horse battery staple" --verified
$ cargo test auth
$ cargo test
```

The manual command prints an ID and email. The tests prove hashing, login,
sessions, verification, reset, and logout.

## Failure modes you will meet

**`password must be at least 12 characters`.** The CLI path uses the same
validation as signup. Use a longer passphrase in development; do not lower the
floor to make a demo easier.

**`email is already registered`.** The normalized email exists. Log in instead,
or use a different address. The error comes from the unique index, not from a
pre-check.

**Login works but the session is `None` on the next request.** The raw token was
not preserved, or the cookie was not sent. The database stores only the hash;
you cannot recover the raw token by looking at the table. Issue a new session.

**A reset token works twice.** It should not. Check that `consume_token` runs in
a transaction and that `used_at` is updated before commit. If two requests race,
the transaction is the guard.

**The old session still works after a password reset.** `reset_password` must
delete sessions for the user in the same transaction. A hash-only update leaves
the attacker logged in.

**A user can log in with a different case in the email.** Normalization must
happen on every path: signup, login, and reset lookup. If one path skips it, the
unique index and the lookup disagree.

**Argon2 is slow under load.** That is the point, but it can also exhaust a
small worker pool. Later chapters move hashing to a blocking pool or cap login
concurrency. Do not remove hashing to fix latency.

## Under the hood

Argon2 hashes are PHC strings such as `$argon2id$v=19$m=...$salt$hash`. The
parameters are stored with the hash, so changing them later does not invalidate
old passwords; verification reads the stored parameters.

SHA-256 is a fast cryptographic hash. It is appropriate for high-entropy tokens
and content fingerprints, not for passwords. The distinction is entropy, not
fashion.

A session is a row plus a cookie. The cookie is the client’s copy; the row is
the server’s authority. Deleting the row is the only reliable logout. A client
that clears its cookie but keeps a copied token still has a valid session until
it expires or is revoked.

CSRF is not authentication. A CSRF token proves the request came from a page the
server rendered for this session, not that the user intended the action. Chapter
6 wires it into forms.

## Exercises

1. **Close the timing gap.** When the email is missing, verify the submitted
   password against a fixed dummy Argon2 hash before returning
   `InvalidCredentials`. Add a test that the error is unchanged.
2. **Make signup atomic.** Wrap user, session, and verification-token inserts in
   one transaction. Add a test that a failure in token creation leaves no user.
3. **Expire sessions on verification.** Decide whether verifying an email should
   rotate the session. Implement it if so, and document why.
4. **Add a session cleanup command.** Delete rows with `expires_at < now`. Add a
   test that an expired session cannot authenticate, then run the cleanup.
5. **Add a password-strength check.** Reject a password that is 12 characters of
   the same letter or a known-bad list. Keep the error type and the HTTP mapping
   stable.

## What comes next

A user can sign in, but every user still sees the same tasks. Chapter 4 adds
workspaces, memberships, and roles. The first authorization question is not “is
this user logged in?” but “is this user allowed to touch this workspace?” The
session from this chapter becomes the input to that question.
