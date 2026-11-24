---
title: "The MPA"
description: "Render the app as a server-rendered MPA with sessions, CSRF, and forms."
---

The service has users, workspaces, projects, and tasks, but no way for a person
to use them. This chapter adds the browser interface: a server-rendered
multi-page application with forms, sessions, CSRF tokens, and HTML.

There is no frontend framework, no build step, and no JavaScript requirement.
Every action is a form submission, a redirect, or a link. The browser asks for a
page; the server renders it; the user acts; the server responds with another
page. That is old technology, and for this product it is the right technology.
It keeps the state on the server, makes the authorization checks unavoidable,
and lets the whole interface work with a keyboard and a screen reader.

By the end of this chapter you can:

- sign up and log in from a browser
- keep a session in an HttpOnly cookie
- create a workspace, project, and task through forms
- toggle and delete a task
- switch workspaces and projects with query parameters
- reject a POST with a missing CSRF token
- render validation errors without losing the form
- use the app with JavaScript disabled

The Rust lessons are `askama` templates, extractors, `HeaderMap`, `Form`,
`Redirect`, cookies, and CSRF. The product lessons are POST/redirect/GET,
server-side validation, accessible forms, and the difference between
authentication and intent.

## Why an MPA

A single-page app would put routing, state, and rendering in the browser. That
means a second authorization layer in JavaScript, a JSON API for every screen,
and a build toolchain before the first form works. It also means the reader has
to learn two languages at once.

The MPA keeps one source of truth: the server. The browser is a renderer and a
form submitter. The session cookie is the only client state. A page refresh
cannot lose unsaved client state because there is no client state to lose.

The trade-off is interaction. A full page reload is slower than a local update,
and a task toggle feels heavier than a checkbox that flips in place. Chapter 18
adds a small progressive enhancement for that, but the baseline remains the MPA.
The app is usable before the enhancement and after it.

## Templates

Add `askama`:

```toml
askama = "0.14"
```

Askama compiles templates at build time. A typo in a template field becomes a
compile error, not a runtime 500. The templates live in `templates/` and are
declared with a path:

```rust
#[derive(Template)]
#[template(path = "login.html")]
struct LoginTemplate {
    error: String,
}
```

`LoginTemplate` has one field. The template can access it as `{{ error }}`.
Askama escapes HTML by default; that is the behavior you want for user data.

`templates/base.html` is the layout:

```html
<!doctype html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{% block title %}Cadence{% endblock %}</title>
    <link rel="stylesheet" href="/static/app.css">
</head>
<body>
    <header class="site-header">
        <a class="brand" href="/">Cadence</a>
        <nav><a href="/app">App</a></nav>
    </header>
    <main class="page">
        {% block content %}{% endblock %}
    </main>
    {% block scripts %}{% endblock %}
</body>
</html>
```

The `title`, `content`, and `scripts` blocks are the extension points. A page
provides only what differs. The stylesheet is one hand-written file; there is no
CSS framework and no font network request.

`templates/login.html` extends the base and renders a form:

```html
{% extends "base.html" %}

{% block title %}Log in · Cadence{% endblock %}

{% block content %}
<h1>Log in</h1>
{% if !error.is_empty() %}
<p class="error" role="alert">{{ error }}</p>
{% endif %}
<form method="post" action="/login" class="stack">
    <label>
        Email
        <input type="email" name="email" autocomplete="email" required autofocus>
    </label>
    <label>
        Password
        <input type="password" name="password" autocomplete="current-password" required>
    </label>
    <button type="submit">Log in</button>
</form>
<p>No account? <a href="/signup">Create one</a>.</p>
{% endblock %}
```

The labels wrap the inputs, so the association does not depend on `for`/`id`.
The error has `role="alert"` so a screen reader announces it. `autocomplete`
helps password managers. `autofocus` puts the cursor in the first field. None of
these are decoration; they are the difference between a form and a puzzle.

The app page is larger because it renders three nested selections. Its shape is:

```text
signed-in user
  workspaces (sidebar)
    selected workspace
      projects
        selected project
          tasks
```

The template receives view structs, not database rows:

```rust
struct WorkspaceView {
    id: String,
    name: String,
    selected: bool,
}

struct ProjectView {
    id: String,
    name: String,
    selected: bool,
}

struct TaskView {
    id: String,
    workspace_id: String,
    project_id: String,
    title: String,
    completed: bool,
}
```

The `selected` flag keeps template conditionals simple. The alternative is
comparing IDs inside the template, which Askama can do but which makes the
template depend on two values and an equality rule. Compute the flag once, in
Rust, and let the template render it.

## State and errors

The router needs the four services this chapter has built:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: SqlitePool,
    pub auth: AuthService,
    pub workspaces: WorkspaceService,
    pub tasks: TaskService,
}

impl AppState {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            auth: AuthService::new(pool.clone()),
            workspaces: WorkspaceService::new(pool.clone()),
            tasks: TaskService::new(pool.clone()),
            pool,
        }
    }
}
```

Later chapters add tokens, webhooks, the event bus, metrics, and the login
limiter to this struct. The handlers do not change shape when they do: they
already receive `State<AppState>`.

`AppError` is the one error type the handlers return:

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,
    #[error("authentication required")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("internal error")]
    Internal,
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Task(#[from] TaskError),
}

impl axum::response::IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        if matches!(self, Self::Unauthorized) {
            return axum::response::Redirect::to("/login").into_response();
        }

        let (status, message) = match &self {
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal error"),
            Self::Unauthorized => unreachable!(),
            Self::Auth(_) => (StatusCode::BAD_REQUEST, "account error"),
            Self::Workspace(_) => (StatusCode::FORBIDDEN, "workspace error"),
            Self::Task(_) => (StatusCode::BAD_REQUEST, "task error"),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}
```

The redirect is the browser-specific part. A fetch or API client should get a
`401` JSON response instead, which is why chapter 13 uses a separate extractor
and error path for the API. One error type can still have more than one
representation; it cannot have more than one `IntoResponse` impl, so the API
decides before it constructs `AppError::Unauthorized`.

## Sessions in a cookie

Login and signup return a session token. The browser needs to send it back. The
cookie is the transport:

```rust
fn with_session(session: AuthSession, location: &str) -> Result<Response, AppError> {
    let cookie = format!(
        "{}={}; Path=/; HttpOnly; SameSite=Lax",
        SESSION_COOKIE, session.token
    );

    let mut response = Redirect::to(location).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).map_err(|_| AppError::Internal)?,
    );

    Ok(response)
}
```

`HttpOnly` keeps JavaScript from reading the cookie. `SameSite=Lax` prevents the
cookie from being sent on most cross-site subrequests, which is a defense
against CSRF. `Path=/` makes it available to the whole app.

The cookie is not `Secure` in local development because the dev server is HTTP.
Production must add `Secure`; chapter 21 makes that explicit. Do not copy this
header into production without it.

Reading the cookie is manual:

```rust
fn session_token(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;

    cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_owned())
    })
}
```

A cookie crate would handle quoted values and edge cases. The book keeps the
parser small because the cookie name and value are generated by this service.
If you accept cookies from other applications on the same host, use a real
parser.

`current_user` combines the cookie with the auth service:

```rust
async fn current_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedUser>, AppError> {
    let Some(token) = session_token(headers) else {
        return Ok(None);
    };

    Ok(state.auth.authenticate(&token).await?)
}
```

`require_user` turns `None` into `AppError::Unauthorized`. The error’s
`IntoResponse` redirects browser requests to `/login`. API requests use a
different extractor and a 401 JSON response; chapter 13 explains why the same
error type cannot serve both.

## CSRF

A session cookie authenticates the browser, not the user’s intent. A malicious
site can submit a form to `POST /tasks` with the user’s cookie attached. The
server sees a valid session and creates a task. CSRF tokens prevent that.

Every session has a CSRF token, generated with the session token. The app page
puts it in every form:

```html
<form method="post" action="/tasks">
    <input type="hidden" name="csrf" value="{{ csrf }}">
    ...
</form>
```

The handler checks it before the mutation:

```rust
fn verify_csrf(user: &AuthenticatedUser, token: &str) -> Result<(), AppError> {
    if user.csrf_token == token {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
```

The check is constant-time enough for a token of this length, but a serious
implementation uses a constant-time comparison to avoid timing leaks. The token
is session-scoped, not form-scoped; an attacker who can read one form can read
all of them, but an attacker who cannot read the page cannot guess the token.

CSRF tokens do not replace authorization. A valid token from a viewer still
cannot create a task; the service returns `Forbidden`. The token proves origin;
the session proves identity; the role proves permission. Three checks, three
different failures.

## POST, redirect, GET

Every mutation follows the same sequence:

```text
GET  /app            -> render a form with a CSRF token
POST /tasks          -> validate, authorize, mutate
303  Location: /app  -> browser re-fetches the list
GET  /app            -> render the new state
```

The redirect is `303 See Other`, which tells the browser to switch to GET. A
refresh after the redirect re-fetches the list; it does not resubmit the form.
This is why the app does not need a “confirm resubmission” dialog.

On success, `create_task` redirects to the selected project:

```rust
Ok(Redirect::to(&format!(
    "/app?workspace={}&project={}",
    task.workspace_id, task.project_id
)).into_response())
```

The redirect target is built from server-owned IDs, not from a user-supplied
`return_to`. That avoids open redirects. If you add a return path, validate that
it starts with `/app` and contains no scheme or host.

On validation failure, the login and signup handlers render the form again with
the error field set. The create-task handler currently returns an `AppError` for
an empty title, which becomes a JSON error page. That is a real inconsistency:
the HTML form should re-render with a field error and a 422. The exercise asks
you to fix it. The book keeps the behavior visible rather than hiding it behind
a generic “everything is validated” claim.

## The app page

`GET /app` loads the user, then selects a workspace and project:

```rust
let workspaces = state.workspaces.list_workspaces(&user.user.id).await?;
let selected_workspace = query
    .workspace
    .filter(|id| workspaces.iter().any(|workspace| &workspace.id == id))
    .or_else(|| workspaces.first().map(|workspace| workspace.id.clone()))
    .unwrap_or_default();
```

The query parameter is accepted only if it names a workspace the user can see.
Otherwise the page falls back to the first workspace. That fallback is a small
authorization check: a caller cannot use `?workspace=<someone else's id>` to
peek at a name, because the list is already scoped.

The same pattern selects a project. If there are no projects, the page shows a
create form. If there is no selected project, it shows a prompt instead of an
empty task list.

The task query uses the project ID and the filter:

```rust
let tasks = state.tasks
    .list_filtered(&user.user.id, &selected_project, &filter)
    .await?;
```

The service checks membership and project access again. The page’s earlier list
is not an authorization token; it is a convenience for selection. Never let a
template’s hidden field replace a service check.

## Forms and handlers

The router maps each page and action:

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(landing))
        .route("/signup", get(signup_form).post(signup))
        .route("/login", get(login_form).post(login))
        .route("/logout", post(logout))
        .route("/app", get(app_page))
        .route("/workspaces", post(create_workspace))
        .route("/projects", post(create_project))
        .route("/tasks", post(create_task))
        .route("/tasks/{id}/toggle", post(toggle_task))
        .route("/tasks/{id}/delete", post(delete_task))
        .route("/static/app.css", get(styles))
}
```

`GET` renders; `POST` mutates. There is no `PUT` or `DELETE` because HTML forms
only support GET and POST. The path encodes the action (`/toggle`, `/delete`)
instead of simulating a REST client. That is normal for an MPA.

`Form<T>` deserializes the URL-encoded body into a struct:

```rust
#[derive(Debug, Deserialize)]
struct CreateTaskForm {
    csrf: String,
    workspace_id: String,
    project_id: String,
    title: String,
}
```

Every field is required by default. A missing field produces a 422 before the
handler runs. That is a framework-level validation, not a domain rule. The
handler still checks the CSRF token and the service still checks permissions.

The hidden `workspace_id` and `project_id` come from the rendered selection.
They are not trusted for authorization; the service compares the project’s
workspace to the claimed workspace and the actor’s membership.

## Tests

The browser is tested at the HTTP boundary, not with a real browser:

```rust
async fn test_app() -> axum::Router {
    let pool = db::connect("sqlite::memory:").await.unwrap();
    db::migrate(&pool).await.unwrap();
    app(AppState::new(pool))
}
```

The signup flow:

1. POST `/signup` with form fields.
2. Assert `303`.
3. Read `Set-Cookie` and keep the first `name=value` pair.
4. GET `/app` with the cookie.
5. Extract the CSRF token from the HTML.
6. POST `/workspaces`, `/projects`, `/tasks` with the cookie and token.
7. GET the project page and assert the task title appears.

The extraction helper is intentionally small:

```rust
fn csrf(html: &str) -> String {
    let marker = "name=\"csrf\" value=\"";
    let start = html.find(marker).unwrap() + marker.len();
    html[start..].split('"').next().unwrap().to_owned()
}
```

A real test suite might parse the DOM. The string search is enough to prove the
hidden field exists and is wired to the form. If the template changes the
attribute order, the test fails loudly; that is acceptable for a contract test.

The second test proves logout requires a valid CSRF token and clears the cookie.
The third posts an unknown email and a wrong password to `/login` and asserts the
page still returns `200` with “Invalid email or password.” It proves the form
re-renders instead of redirecting or leaking a different error.

The tests do not use a real browser, JavaScript, or a network socket. They are
fast and deterministic. The manual browser check is still valuable for layout,
focus order, and the feel of the form.

## Accessibility

A server-rendered form is not automatically accessible. The chapter’s baseline:

- every input has a visible label
- errors are text, not color alone
- the error has `role="alert"`
- the first field receives focus when the form loads
- the page has one `h1` and a logical heading order
- links and buttons are real `<a>` and `<button>` elements
- focus styles are visible
- the layout works at narrow widths
- no interaction depends on hover

The app uses a system font stack and a dark-mode media query. Those are visual;
the semantic HTML is the part that matters to assistive technology.

The task toggle is a form with a button, not a `div` with a click handler. The
button has an accessible name (“Toggle task”) and the state is also present in
the task text (`✓` vs `○`). A screen reader can operate it without JavaScript.

## Checkpoint

```console
$ cargo test --test web
$ cargo test
$ cargo run -- serve
```

Open `http://127.0.0.1:3000/`, create an account, create a workspace, create a
project, add a task, toggle it, and delete it. Then disable JavaScript and do it
again. Both runs should work.

The checkpoint is not “the page looks good.” It is “the browser can complete the
same contract as the service tests, with the cookie and CSRF token.”

The chapter shows the template shape and the handlers that carry the contract.
The complete views are `templates/base.html`, `landing.html`, `signup.html`,
`login.html`, and `app.html`; the full browser tests are the three tests in
`cadence-app/tests/web.rs`:

- `signup_login_and_task_flow`
- `logout_requires_csrf_and_clears_the_session`
- `login_rejects_a_bad_password`

Copying the templates is expected here; they are markup, not a hidden lesson.
Read them as you copy, and compare them with the accessibility checklist above.

Commit the checkpoint:

```text
Add the server-rendered browser app
```

## Failure modes you will meet

**A redirect loop between `/login` and `/app`.** The session cookie is missing,
expired, or not being sent. Check `Set-Cookie`, the cookie path, and whether the
browser rejects the cookie because of `Secure` on HTTP.

**`403` on every form.** The CSRF token is missing or stale. The page may have
been cached from an old session. Reload `/app` and submit the fresh token.

**The app shows the wrong workspace.** The query parameter named a workspace the
user cannot see, and the fallback chose the first visible one. That is intended.
Do not “fix” it by trusting the parameter.

**A form submits but nothing changes.** Check the service error. A `Forbidden`
or `Conflict` response is an error page; the redirect only happens on success.

**An empty task title returns JSON on an HTML page.** The handler uses
`AppError` instead of re-rendering the form. That is the known gap in this
chapter; exercise 2 fixes it.

**User text appears as HTML.** Askama escapes by default. The only `safe` filter
should be for sanitized Markdown, which this app does not render yet. Never mark
a user field safe.

**A hidden `workspace_id` is tampered with.** The service checks that the
project belongs to the workspace and the actor belongs to both. The hidden field
is a convenience, not a permission.

## Under the hood

Askama generates Rust code from the template at compile time. The template
struct’s fields become a context; a missing field is a compile error. This is
why the template and the handler must change together.

`Form<T>` reads the request body based on `Content-Type:
application/x-www-form-urlencoded`. A JSON body will not deserialize into the
same struct. The MPA and the API have different extractors and different error
shapes on purpose.

`Redirect::to` sets the `Location` header and a status. `303` is the correct
status after a POST because it tells the client to use GET for the next request.
`302` is common but ambiguous; use `303` when you mean “see other.”

The session cookie is a bearer token. CSRF protects against a malicious site
using the browser’s cookie; it does not protect against a stolen cookie. The
`HttpOnly` and `SameSite` flags reduce the attack surface; chapter 21 documents
the remaining production controls (`Secure`, `__Host-`, HSTS, rotation).

## Exercises

1. **Add a confirmation page before delete.** `POST /tasks/{id}/delete` should
   render a page with the title and two buttons, then a second POST performs the
   delete. No JavaScript `confirm()`.
2. **Re-render task validation.** On an empty title, return `422` with the app
   template, the submitted title, and a field error. Add a test that the page
   contains the error and the form still works.
3. **Add a filter form.** A GET form with `filter` and hidden workspace/project
   fields should preserve the selection. The service already accepts the filter.
4. **Add a no-JS test.** Fetch `/app` without a cookie and assert the redirect to
   `/login`; fetch it with a cookie and assert the HTML contains a `<form>` with
   a CSRF input.
5. **Add a skip link.** Put “Skip to content” before the header, target the main
   element, and verify it is the first focusable element.

## What comes next

The browser can create and complete tasks, but the list is only as useful as its
filters. Chapter 7 adds the query DSL: `due:today & priority:>2`, parsed into an
AST and compiled to parameterized SQL. The form’s `filter` field already has a
place to put it.
