# 21. Security

**What you can do at the end.** Serve the app with security headers, throttle
repeated login failures, and keep every mutation behind CSRF and authorization.

**The Rust you learn.** Response middleware, in-memory rate limiting, secret
handling, and a threat model expressed as tests.

**Build it.**

- Add `security.rs` with a middleware that sets CSP, `nosniff`,
  `X-Frame-Options: DENY`, and `Referrer-Policy: same-origin`.
- Add a `LoginLimiter` keyed by normalized email: five failures in fifteen
  minutes, then refuse until the window clears.
- Check the limiter before password verification; record only failures.
- Keep CSRF on all POSTs and authorization in services.
- Never log tokens, passwords, or full email bodies.
- Add tests for the limiter and for missing/invalid CSRF.

The limiter is a small in-memory window:

```rust
pub fn allow(&self, key: &str) -> bool {
    let mut attempts = self.attempts.lock().unwrap();
    attempts.entry(key.to_owned()).or_default()
        .retain(|at| at.elapsed() < WINDOW);
    attempts[key].len() < MAX_ATTEMPTS
}
```

**Checkpoint.**

```console
$ cargo test security
$ cargo test --test web login_rejects_a_bad_password
```

The tests prove the sixth attempt is refused and the headers are present on
responses.

**Exercises.**

1. Add a token/IP rate limit for the JSON API.
2. Add an authz matrix test that covers every role and route.
