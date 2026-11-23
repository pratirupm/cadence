# 6. The MPA

**What you can do at the end.** Use Cadence from a browser: sign up, log in,
create a workspace, project, and task, and toggle it done.

**The Rust you learn.** `askama` templates, HTML forms, POST/redirect/GET,
CSRF, session cookies, and accessible server-rendered UI.

**Build it.**

- Add `askama`.
- Create a base template with a small hand-written stylesheet.
- Add landing, login, signup, and app templates.
- Read the session cookie, authenticate the token, and redirect anonymous
  users to `/login`.
- Render forms with hidden `_csrf` fields; verify the token on every POST.
- Redirect with `303` after success, and render the form again on validation
  failure.
- Keep the forms usable without JavaScript.

Every mutation is a form:

```html
<form method="post" action="/tasks">
  <input type="hidden" name="csrf" value="{{ csrf }}">
  <input type="hidden" name="workspace_id" value="{{ selected_workspace }}">
  <input type="hidden" name="project_id" value="{{ selected_project }}">
  <label>New task <input name="title" required></label>
  <button type="submit">Add task</button>
</form>
```

**Checkpoint.**

```console
$ cargo test --test web
```

The integration test signs up through HTTP, extracts the session cookie and
CSRF token, creates a workspace, project, and task, and asserts the task appears
in the rendered page.

**Exercises.**

1. Add a confirmation page before deleting a task.
2. Add a `GET /app?filter=due:today` view and preserve the filter in links.
