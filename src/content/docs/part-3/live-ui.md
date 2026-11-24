---
title: "Live UI"
description: "Add a tiny progressive enhancement to the MPA."
---

Chapter 17 added an event stream. The browser still does not use it. This chapter
adds the smallest useful client: a script that listens for task events and
refreshes the page. It is deliberately tiny. The MPA already works without it;
the script only removes a manual refresh.

By the end of this chapter you can:

- serve a static script from the Rust binary
- include it on one page without a build step
- open an `EventSource` to `/events`
- debounce a burst of events into one refresh
- explain why the page still works with JavaScript disabled
- identify the cost of a full reload and the reconnect gap
- test the no-JavaScript baseline and observe the live path manually

The Rust lessons are static asset serving, template blocks, and keeping policy
(CSP) compatible with the script. The frontend lesson is progressive enhancement:
add behavior, do not make behavior the only path.

## The smallest useful enhancement

The app has three live events:

```text
task.created
task.updated
task.deleted
```

All three change the task list. The simplest correct response is “reload the
page.” It does not need a client-side model, a diff, or a component tree. It does
lose focus and form state, and it is slower than an in-place update, but it is
correct and it ships.

A more ambitious client would patch the list, update counts, and preserve the
selected task. That is chapter 19’s kind of work in a different book; here the
point is the transport and the fallback.

## The script

`static/live.js`:

```js
(() => {
    const source = new EventSource("/events");
    let timer;

    const refresh = () => {
        clearTimeout(timer);
        timer = setTimeout(() => window.location.reload(), 150);
    };

    for (const event of ["task.created", "task.updated", "task.deleted"]) {
        source.addEventListener(event, refresh);
    }

    source.onerror = () => source.close();
})();
```

`EventSource` opens a long-lived GET request to `/events`. The browser sends the
session cookie automatically because the request is same-origin. The server
authenticates it with the MPA’s cookie path, not the API bearer path.

The three listeners all call `refresh`. A burst of events (one create and three
updates) schedules one reload 150 ms after the last event. Without the debounce,
a batch of changes would reload several times.

`window.location.reload()` re-fetches the current URL, including the selected
workspace and project query parameters. The server renders the new state. The
browser may restore scroll position, but form input and focus are not part of the
URL and will be lost.

`source.onerror = () => source.close()` is a deliberate simplification with a
cost. `EventSource` normally reconnects automatically. Closing on the first error
stops the live updates until the page reloads. That is acceptable for a demo and
wrong for a long-lived app. Exercise 2 removes the close and adds backoff.

## Serving the script

The script is a static asset, but the binary embeds it:

```rust
async fn live_script() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/live.js"),
    )
}
```

The route is `/static/live.js`. `include_str!` embeds the file at compile time, so
the deployed binary does not need a `static/` directory. The CSS uses the same
pattern in `styles`. For a larger asset pipeline, serve a directory with
`tower-http`; for two files, embedding is simpler and avoids a filesystem path.

The content type must be `text/javascript`. A browser refuses to execute a script
served as `text/plain` when `X-Content-Type-Options: nosniff` is set, which the
security layer sets in chapter 21.

## Including it on the app page

`base.html` has a `scripts` block after `main`:

```html
    <main class="page">
        {% block content %}{% endblock %}
    </main>
    {% block scripts %}{% endblock %}
</body>
```

`app.html` fills it:

```html
{% block scripts %}
<script src="/static/live.js" defer></script>
{% endblock %}
```

`defer` runs the script after the document is parsed, without blocking rendering.
The script does not depend on DOM nodes, so it would also work at the end of the
body, but `defer` is explicit.

The login, signup, and landing pages do not include the script. They do not need
live task events. Loading it only on the app page keeps the surface small.

The script is same-origin, so the CSP `script-src 'self'` allows it. There is no
inline script and no nonce. If you add an inline snippet, you must add a hash or
nonce to the CSP; do not weaken the policy to `unsafe-inline`.

## Why a full reload

A full reload has three costs:

1. **Network and render.** The browser fetches the HTML, CSS, and script again
   (cached assets help), then re-renders the page.
2. **Lost local UI state.** Focus, cursor position, and unsaved form input are
   gone.
3. **Race with the event.** The reload fetches after the event, so it should see
   the new state. A later event during the fetch schedules another reload; the
   debounce collapses nearby events.

The benefit is correctness with almost no client code. The server is still the
only renderer. There is no client-side cache to invalidate and no second
authorization path.

An in-place update would need:

- a stable list container and per-task IDs,
- insert/update/remove logic,
- ordering rules that match the server,
- a fallback when the event payload is insufficient,
- focus preservation.

That is a real feature, not a one-line enhancement. The exercise asks for a
narrow version: update the title and completion of an existing row, and reload
on create/delete.

## No-JavaScript baseline

With JavaScript disabled:

- login, signup, create workspace/project/task, toggle, delete all work;
- the page does not refresh automatically when another tab changes a task;
- the user can refresh manually.

The web integration tests run without a browser or JavaScript. They prove the
forms, cookie, CSRF, and redirects. They do not prove the script; there is no
automated browser test in this chapter.

The manual check:

```console
$ cargo run -- serve
```

Open two tabs to the same project. Toggle a task in one. The other reloads within
a fraction of a second. Disable JavaScript and repeat: the other tab stays stale
until refresh.

## The reconnect gap

If the stream drops, the script closes it and does not retry. Events during the
gap are lost. The change log is the repair path: a full page reload pulls the
current list, and the CLI/API client can pull changes by cursor. The browser has
no cursor, so a reload is its resync.

Even without the explicit close, `EventSource` would reconnect and send
`Last-Event-ID`. The server does not set an event ID and does not replay, so the
browser would reconnect at the live edge and still miss the gap. Reconnect alone
is not a replay protocol.

A robust client would:

1. open the stream,
2. on error, back off and reconnect,
3. after reconnect, refresh or pull,
4. show a small “reconnecting” state.

The server-side replay is chapter 17’s exercise. The client-side backoff is this
chapter’s exercise.

## Security

The stream uses the session cookie. The script does not read the cookie; the
browser attaches it. `HttpOnly` still applies. The script cannot steal the
session even if it wanted to.

CSRF does not apply to GET streaming. The endpoint is read-only. A malicious site
can open an `EventSource` to `/events`, but the browser’s same-origin policy and
`SameSite=Lax` cookie behavior prevent it from reading the response with the
user’s session. Do not add state-changing behavior to the stream.

The script does not use `eval`, does not inject HTML, and does not read the URL
into the DOM. The only data it handles is an event notification; it ignores the
payload entirely. That is the safest possible client.

## Tests and manual evidence

The web tests prove the no-JS baseline:

- signup → app → workspace → project → task through forms;
- login failure re-renders the page;
- logout requires CSRF and clears the cookie.

The event bus test from chapter 17 proves publish/subscribe. The script itself is
manually verified with two tabs and the browser network panel.

A browser test could use Playwright: open two pages, toggle in one, assert the
other’s text changes. That is a good integration test, but it adds a browser
dependency to CI. The book keeps the manual check and names the gap.

## Worked burst

A user completes three tasks in quick succession:

```text
t=0ms    task.updated  -> timer = 150ms
t=40ms   task.updated  -> clear, timer = 150ms
t=90ms   task.updated  -> clear, timer = 150ms
t=240ms  reload fires
```

One reload covers three events. Without the debounce, the page would reload three
times, and each reload would compete with the next event. A 150 ms window is a
guess; a busy workspace may need 500 ms or an in-place update.

A create followed by an update in the same burst also causes one reload. The
reload renders the final state; the intermediate state is not shown. That is fine
for a list, and it is why the script does not try to replay events.

If the reload takes longer than the gap between bursts, the next burst schedules
another reload after the first finishes. The browser serializes navigations; the
script does not queue them.

## No-JavaScript parity

The feature matrix with and without the script:

| Action | No JavaScript | With script |
| --- | --- | --- |
| Sign up / log in | yes | yes |
| Create workspace/project/task | yes | yes |
| Toggle / delete | yes | yes |
| See another tab’s change | manual refresh | automatic reload |
| Preserve typed input on live change | n/a | no (reload) |
| Reconnect after stream error | n/a | no (script closes) |

The first four rows are the contract. The last two are the enhancement’s costs.
A progressive enhancement that breaks a form is not an enhancement; test the
table’s left column before adding the right.

## Checkpoint

```console
$ cargo test --test web
$ cargo test events
$ cargo run -- serve
```

Open two tabs, toggle a task, watch the other refresh. Disable JavaScript and
confirm the forms still work.

## Failure modes you will meet

**The other tab does not refresh.** The stream is not connected: check the
network panel for `/events`, a 303 to `/login`, or a CSP violation. The script is
only included on `/app`.

**The tab reloads repeatedly.** Every event schedules a reload, and the reload
itself does not suppress the next event. The debounce is 150 ms; a very busy
workspace can still thrash. Increase the debounce or update in place.

**Live updates stop after a network blip.** `source.onerror` closes the stream and
does not reconnect. Remove the close and add backoff.

**The script is blocked by CSP.** It is served from `/static/live.js` and
`script-src 'self'` should allow it. Check for an inline script or a different
origin.

**The stream returns 303.** The session expired. The browser follows the redirect
to `/login`, which is not an event stream. Refresh and log in.

**A task change from the CLI does not refresh the tab.** The sync endpoint does
not publish events (chapter 17’s gap). Refresh manually or add publishing.

**The reload loses typed input.** A full reload discards form state. If that
matters, update in place or warn before reload. The current script is a demo.

## Under the hood

`EventSource` is a browser API, not a library. It reconnects with `Last-Event-ID`
when the server provides IDs, and it retries with a browser-defined backoff. The
script’s `onerror` overrides that behavior by closing.

`defer` scripts execute in order after parsing. The script does not wait for
`DOMContentLoaded`; it does not need the DOM. If it did, it would add a listener.

`include_str!` makes the binary larger by the size of the script and removes a
runtime file dependency. The trade-off is a rebuild when the script changes,
which is exactly what a compiled asset should require.

## Exercises

1. **Update in place.** On `task.updated`, find the row by task ID and replace the
   title/completion. Reload on create/delete. Keep the no-JS fallback.
2. **Reconnect with backoff.** Remove `source.close()`, add exponential backoff on
   error, and refresh after a successful reconnect.
3. **Pause when hidden.** Use `document.visibilityState` to close the stream when
   the tab is hidden and reopen when visible. Test with two tabs.
4. **Add a status indicator.** Show “live” / “reconnecting” without hiding the
   page when the stream is down. Use text, not color alone.
5. **Write a browser test.** Use Playwright to open two pages, toggle a task, and
   assert the other page’s text. Run it in CI with a headless browser.

## What comes next

The browser can see live task changes, but other systems cannot subscribe. Chapter
19 adds signed webhooks with retries, SSRF protection, and a delivery log. The
event bus and the webhook dispatcher are different consumers of the same task
change.
