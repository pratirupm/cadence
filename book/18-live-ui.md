# 18. Live UI

**What you can do at the end.** Keep the MPA open and see task changes arrive
without a manual refresh, while the app still works with JavaScript disabled.

**The Rust you learn.** Progressive enhancement, the boundary between server
state and a tiny client script, and testing the no-JS baseline.

**Build it.**

- Add `static/live.js`: open `EventSource("/events")`, listen for the three task
  events, debounce, and reload the page.
- Serve the script from `/static/live.js`.
- Add a `scripts` block to the app template and include the script with
  `defer`.
- Leave every form and link unchanged. The page must be fully usable without
  the script.
- Keep the script dependency-free; no build step.

The script is deliberately boring:

```js
const source = new EventSource("/events");
source.addEventListener("task.updated", () => {
    window.location.reload();
});
```

**Checkpoint.**

```console
$ cargo test --test web
$ cargo run -- serve
```

Open two browser tabs to the same project, toggle a task in one, and watch the
other refresh. Disable JavaScript and confirm the form still works.

**Exercises.**

1. Replace the reload with an in-place list update and keep a no-JS fallback.
2. Add a connection-status indicator that is hidden until the stream fails.
