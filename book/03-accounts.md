# 3. Accounts

**What you can do at the end.** Sign up, log in, hold a session, verify an
email token, and reset a password.

**The Rust you learn.** `argon2`, random tokens, SHA-256 hashing, session
records, and the difference between authentication and authorization.

**Build it.**

- Add `argon2`, `rand`, `sha2`, and `hex`.
- Create users, sessions, and auth-token tables.
- Hash passwords with Argon2id and a random salt; never store the password.
- Generate 32 random bytes for a session token. Store only its SHA-256 hash.
- Return the raw token to the caller once; authenticate by hashing the incoming
  token and looking up the hash.
- Add email-verification and password-reset tokens with expiry and single use.
- Add `cadence user create` for local administration.

The session token is a capability. The database keeps the verifier:

```rust
let token = random_token();
sqlx::query("INSERT INTO sessions (id, user_id, ...) VALUES (?, ?, ...)")
    .bind(hash_token(&token))
    .execute(&pool)
    .await?;
```

**Checkpoint.**

```console
$ cargo test auth
$ cargo run -- user create --email you@example.com --name You \
    --password "correct horse battery staple" --verified
```

The tests cover duplicate email, weak passwords, wrong passwords, verification
single-use, password reset, session revocation, and logout.

**Exercises.**

1. Add a test that an expired session does not authenticate.
2. Add a session cleanup command that deletes rows older than thirty days.
