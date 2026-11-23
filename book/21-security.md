# 21. Security

Security is not a chapter you add at the end. It is a set of small decisions made
throughout the book: hash the password, hash the session token, check the
workspace before the project, verify the CSRF token, bind SQL values, escape HTML,
validate the webhook URL, and do not log secrets.

This chapter collects those decisions into a review. It adds response security
headers and login throttling, then walks the threat model so you can see what is
covered and what is still missing. It does not claim the service is
unhackable. It claims the common failure modes have a named defense and a test.

By the end of this chapter you can:

- explain the difference between authentication, authorization, and intent
- read the security-header middleware and the CSP
- trace a CSRF token from session to hidden field to verification
- throttle repeated login failures and explain the per-process limit
- run the authorization tests that cover workspace and project roles
- identify missing production controls: HSTS, `Secure`, shared rate limits,
  API rate limits, constant-time CSRF comparison
- review a change for injection, XSS, SSRF, and secret leakage

The Rust lessons are response middleware, an in-memory rate limiter, and
constant-time thinking. The security lesson is that a control without a test is a
hope.

## The threat model, in one table

| Threat | Current defense | Where |
| --- | --- | --- |
| Password theft from DB | Argon2id hashes | `auth.rs` |
| Session theft from DB | SHA-256 token hashes | `auth.rs` |
| CSRF | session token + hidden field + `SameSite=Lax` | `web.rs` |
| XSS | Askama auto-escape, CSP | templates, `security.rs` |
| SQL injection | bound parameters, field whitelist | `tasks.rs`, `query.rs` |
| Tenant escape | workspace/project checks | `workspaces.rs`, `tasks.rs` |
| API scope escalation | scope hierarchy + service checks | `api.rs` |
| SSRF | webhook URL validation | `webhooks.rs` |
| Brute force | login limiter | `security.rs` |
| Clickjacking | `frame-ancestors 'none'` | `security.rs` |
| MIME sniffing | `nosniff` | `security.rs` |
| Secret in logs | redaction at call sites | mailer, handlers |
| Webhook forgery | HMAC signature | `webhooks.rs` |

A table is not a proof. Each row needs a test or a manual check. The chapters
that built the defense include the test; this chapter adds the ones that are
missing at the edge.

## Authentication, authorization, intent

Three different questions:

- **Authentication:** who is this? A session cookie or bearer token.
- **Authorization:** may this identity touch this resource? A workspace role and
  project role.
- **Intent:** did this browser mean to send this request? A CSRF token.

A valid session answers the first. It does not answer the second; a viewer can
read but not write. It does not answer the third; a malicious site can make the
browser send a request with the cookie.

The three checks fail differently:

- no session -> 401 or redirect
- wrong role -> 403
- missing CSRF -> 403

Do not collapse them into one “not allowed” page. The user needs to know whether
to log in, ask an admin, or reload the form.

## Security headers

`security::headers` is a response middleware:

```rust
pub async fn headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        header::HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data:; style-src 'self'; \
             script-src 'self'; connect-src 'self'; frame-ancestors 'none'; \
             base-uri 'self'",
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
```

The CSP:

- `default-src 'self'` blocks third-party scripts, styles, and frames by default.
- `script-src 'self'` allows `/static/live.js`, not inline scripts.
- `style-src 'self'` allows the stylesheet, not inline style attributes.
- `img-src 'self' data:` allows local images and data URIs.
- `connect-src 'self'` allows the SSE request and same-origin fetch.
- `frame-ancestors 'none'` prevents embedding the app in another site.
- `base-uri 'self'` prevents a `<base>` tag from rewriting relative URLs.

`nosniff` stops a browser from treating a text file as JavaScript or CSS.
`X-Frame-Options: DENY` is a legacy duplicate of `frame-ancestors` for older
browsers. `Referrer-Policy: same-origin` prevents leaking paths to other sites.

The layer is installed after the request-count and trace layers:

```rust
.layer(TraceLayer::new_for_http())
.layer(middleware::from_fn_with_state(state.metrics.clone(), count_requests))
.layer(middleware::from_fn(security::headers))
```

Layers wrap from the inside out. The exact order matters when one layer can
short-circuit. Here the header layer only reads the response, so it applies to
errors and successes alike. A 404 gets the headers; so does a 500.

Missing headers:

- `Strict-Transport-Security` (HSTS) is not set. It should be set only on HTTPS.
  In local HTTP it would be ignored, but a production deployment must add it.
- `Permissions-Policy` is not set. The app uses no camera, microphone, or
  geolocation, so a restrictive policy is cheap.
- `Cross-Origin-Opener-Policy` / `Cross-Origin-Resource-Policy` are not set.

They are exercises, not reasons to avoid the current controls.

## Cookies

The session cookie is:

```text
cadence_session=<token>; Path=/; HttpOnly; SameSite=Lax
```

`HttpOnly` keeps JavaScript from reading it. `SameSite=Lax` prevents the cookie
from being sent on most cross-site subrequests while allowing normal navigation.
`Path=/` sends it to every app route.

Missing for production:

- `Secure` — the cookie would travel over HTTP. Local development uses HTTP, so
  the attribute is omitted here. Production must add it.
- `__Host-` prefix — a stricter cookie name that requires `Secure`, `Path=/`, and
  no `Domain`.
- `Max-Age`/`Expires` — the cookie is a session cookie in the browser, while the
  server row has a seven-day expiry. Closing the browser loses the cookie even
  though the row still exists. That is safer, but it should be a documented
  choice.
- `Partitioned` — a newer isolation attribute; not required for this app.

A session token is a bearer credential. `HttpOnly` does not stop a stolen cookie
from being replayed; it stops script access. TLS, `Secure`, short lifetimes, and
rotation are the other controls.

## CSRF

Every mutating form includes the session’s CSRF token:

```html
<input type="hidden" name="csrf" value="{{ csrf }}">
```

The handler checks it before the service call:

```rust
fn verify_csrf(user: &AuthenticatedUser, token: &str) -> Result<(), AppError> {
    if user.csrf_token == token {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
```

Logout is a POST with a CSRF token too. That is not an accident; an attacker who
can log a user out can hide the logout from a shared machine. The test
`logout_requires_csrf_and_clears_the_session` proves a wrong token is 403 and a
valid token clears the cookie.

`SameSite=Lax` is a second layer. It is not a replacement: some browsers and some
navigation patterns still send cookies cross-site. The token is the primary
defense.

The comparison is `==`, not constant-time. For a 64-hex random token, a timing
attack is impractical over a network, but a serious implementation uses
`subtle::ConstantTimeEq` or a similar primitive. The exercise asks for it.

The token is session-scoped, not per-form. A token leak from one form compromises
all forms for that session. Per-form tokens are stricter but require storing a
nonce per form and cleaning it up. The book keeps one token and documents the
trade-off.

## Login throttling

`LoginLimiter` is an in-memory map from email to attempt timestamps:

```rust
const WINDOW: Duration = Duration::from_secs(15 * 60);
const MAX_ATTEMPTS: usize = 5;

pub fn allow(&self, key: &str) -> bool {
    let mut attempts = self.attempts.lock().expect("login limiter lock");
    let now = Instant::now();
    let entries = attempts.entry(key.to_owned()).or_default();
    entries.retain(|attempt| now.duration_since(*attempt) < WINDOW);
    entries.len() < MAX_ATTEMPTS
}
```

The login handler checks `allow` before verifying the password, records only a
failure, and clears on success:

```rust
if !state.login_limiter.allow(&form.email) {
    return render(LoginTemplate { error: "Too many attempts. Try again later.".to_owned() });
}

match state.auth.login(&form.email, &form.password).await {
    Ok(session) => {
        state.login_limiter.clear(&form.email);
        // ...
    }
    Err(_) => {
        state.login_limiter.record_failure(&form.email);
        // ...
    }
}
```

Five failures in fifteen minutes block the email. The lock is a `std::sync::Mutex`
held for a push/retain and no await; that is correct. Do not hold it across the
Argon2 verification.

Limitations:

- The key is the raw form email. `A@example.com` and `a@example.com` are separate
  buckets, even though auth normalizes them. Normalize before the limiter.
- The map is per process. Two server instances allow ten attempts combined.
  A shared store (Redis or a database table) is needed for a fleet.
- There is no IP or device key. An attacker can rotate emails; a victim can be
  locked out by someone else’s failures. A real limiter combines email, IP, and
  a global rate.
- The map is unbounded. `retain` cleans entries only when that key is checked;
  many unique emails grow memory. Add periodic cleanup or a bounded cache.
- A successful login clears the bucket. That is friendly, but it also lets an
  attacker who knows one password reset the failure count.

It is a useful first control, not a complete brute-force defense.

## Authorization tests

The service has tests for the cases that matter:

- a viewer cannot create a project;
- a non-member cannot list a private project;
- a workspace member with project viewer can read but not write a private task;
- a project editor can write;
- a workspace viewer with a project editor row still cannot write;
- a read-only API token gets 403 on create;
- a missing bearer gets 401.

These are the book’s authorization matrix. When you add a role or a route, add a
row. A test that logs in as an owner and calls every endpoint proves almost
nothing about the forbidden cases.

The API scope check and the tenant role check are separate. A `write` token for a
viewer is still forbidden. Do not “simplify” by trusting the token scope alone.

## Injection and XSS

SQL values are bound:

```rust
sqlx::query("SELECT ... WHERE id = ?").bind(id)
```

The query DSL whitelists fields and operators and binds values. The `format!` in
`list_filtered` interpolates only the compiled clause, which is built from
whitelisted fragments. A new field must be added to the compiler, not passed
through as raw text.

HTML is escaped by Askama by default. The app does not render user Markdown, so
there is no `safe` filter on user content. If you add Markdown, sanitize it with
a library such as `ammonia` and keep the CSP. Do not concatenate HTML strings.

The CSP is the second layer for XSS. It does not fix a server-side injection; it
limits what an injected script can do. Both are needed.

## SSRF and webhooks

Webhook URLs are user-supplied requests from the server. Chapter 19’s validation
blocks HTTPS-only plus some private ranges. It is incomplete: DNS rebinding,
redirects, `172.16/12`, IPv6 ULA, and cloud metadata endpoints are not covered.
Treat webhooks as a security-sensitive feature and resolve/validate every hop at
delivery time.

## Secrets and logs

Rules:

- never log a password, session token, API token, reset token, or Authorization
  header;
- never log an email body;
- never put a token in a URL query string;
- never serialize the `User.password_hash` or `Webhook.secret` into a list
  response;
- rotate a leaked secret by revoking it, not by editing history.

The log mailer logs recipient and subject, not the body. The auth errors do not
include the password. The API list endpoint maps webhooks to a view without the
secret. The audit log stores IDs and roles, not tokens.

## Tests and manual evidence

The limiter test:

```text
limits_repeated_failures ... ok
```

It proves five failures block and `clear` unblocks. The web tests prove logout
CSRF and the no-JS form flow. The workspace/task tests prove the role matrix. The
API tests prove scope rejection.

Manual checks:

```console
$ curl -sI http://127.0.0.1:3000/health | grep -i 'content-security-policy'
$ cargo test security
$ cargo test --test web
$ cargo test workspaces tasks
$ cargo test --test api
```

The headers should appear on success and error responses. A CSP violation appears
in the browser console, not in a Rust test.

## Checkpoint

```console
$ cargo test security
$ cargo test --test web
$ cargo test workspaces tasks
$ cargo test --test api
$ cargo test
$ cargo fmt --check
$ cargo clippy --all-targets --all-features -- -D warnings
```

The checkpoint is not “no vulnerabilities.” It is “the named controls are
present, tested where practical, and the gaps are written down.”

## Failure modes you will meet

**A POST returns 403 with a valid session.** The CSRF token is missing, stale, or
from another session. Reload the form. A missing field is a 422 before the
handler; a wrong token is a 403.

**A user is locked out after five failures.** The limiter is working. Wait for
the window, clear the bucket on a successful login from another path, or add an
admin unlock. Do not disable the limiter to fix a support ticket.

**The limiter does not stop a distributed attack.** It is per process and keyed
by raw email. Add a shared store and an IP key.

**The CSP blocks the live script.** The script is same-origin and allowed by
`script-src 'self'`. Check for an inline script or a different host. Do not add
`unsafe-inline`.

**The cookie is not sent in production.** It is missing `Secure` or the domain
does not match. Set `Secure` under HTTPS and use a `__Host-` name if possible.

**A webhook reaches a private IP.** The hostname validation was bypassed by DNS
or a redirect. Resolve and revalidate at delivery.

**An error log contains a token.** Find the call site; do not add a filter that
silently drops the whole log line. Redact the field and keep the event.

**The metrics endpoint is public.** Restrict it at the proxy. Metrics reveal
traffic and queue health.

## Under the hood

Middleware order is a security boundary. A layer that rewrites the request after
auth or before CSRF can change the meaning of the check. Keep authentication,
authorization, and CSRF in that order, and test the negative path.

CSP is enforced by the browser, not the server. A non-browser client ignores it.
It is an XSS mitigation, not an API control.

Rate limiting is a policy on a key. The key choice determines who is protected
and who can be locked out. There is no universal correct key; email, IP, account,
and device each have a different abuse model.

## Exercises

1. **Add HSTS and `Secure` in production.** Read an environment flag, set
   `Strict-Transport-Security` and the cookie `Secure` only under HTTPS. Add a
   test for the header in the test configuration.
2. **Constant-time CSRF comparison.** Use `subtle::ConstantTimeEq` and a test that
   the comparison still accepts a valid token and rejects a one-byte change.
3. **Normalize the limiter key.** Lowercase and trim the email before `allow`,
   `record_failure`, and `clear`. Add a test for case variants.
4. **Add an IP key.** Thread the client IP into the limiter and combine it with
   the email. Document the proxy trust rule.
5. **Automate the authorization matrix.** Write a table-driven test that runs
   every role against every protected route and asserts the status. Fail when a
   new route has no row.

## What comes next

The security review is a set of controls with costs. Chapter 22 measures the
cost that matters most: query performance. Indexes, N+1 queries, and a benchmark
turn “it feels slow” into a number you can compare.
