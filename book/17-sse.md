# 17. Server-sent events

**What you can do at the end.** Keep a browser or API client open and receive
`task.created`, `task.updated`, and `task.deleted` events for workspaces the
user belongs to.

**The Rust you learn.** `broadcast` channels, axum SSE, stream adapters,
filtering per subscriber, and backpressure.

**Build it.**

- Add an `EventBus` wrapping `tokio::sync::broadcast::Sender<AppEvent>`.
- Publish an event after every task mutation.
- Add `GET /events` for cookie sessions and `GET /api/v1/events` for tokens.
- At connect time, load the user’s workspace IDs into a `HashSet`.
- Filter events that are not in that set.
- Serialize each event as JSON and add `KeepAlive`.

The stream is filtered before it leaves the process:

```rust
let stream = BroadcastStream::new(receiver).filter_map(move |message| {
    let event = message.ok()?;
    allowed.contains(&event.workspace_id).then(|| {
        Ok(Event::default().event(event.kind).data(serde_json::to_string(&event).unwrap()))
    })
});
```

**Checkpoint.**

```console
$ cargo test events
```

The unit test publishes to the bus and receives the event. A manual
`curl -N` against `/events` shows the keep-alive and task events.

**Exercises.**

1. Add `Last-Event-ID` replay from the change log.
2. Drop slow subscribers after a bounded queue.
