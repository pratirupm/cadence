# 17. Server-sent events

The sync client asks for changes when the user runs `sync`. A browser can poll,
but polling every few seconds is wasteful and still feels late. A task service
with two open tabs should show the other tab’s change without a manual refresh.

This chapter adds a one-way event stream. The server pushes task events to
connected clients. It is not a replacement for the change log; it is a live
notification layer on top of it. If a client misses an event, the cursor pull
repairs it.

By the end of this chapter you can:

- publish a typed event after a task mutation
- fan it out to every subscriber with a `broadcast` channel
- stream events as SSE over a cookie or bearer token
- filter events to the workspaces the user belongs to
- explain why the filter is computed at connect time
- identify missed events on reconnect, slow consumers, and sync writes
- test the bus and manually observe the stream with `curl`

The Rust lessons are `tokio::sync::broadcast`, `StreamExt::filter_map`, axum’s
`Sse`, and per-connection state. The systems lesson is that a live stream is a
hint, not a durable queue.

## SSE versus polling and WebSockets

Three live-update options:

1. **Polling.** The client asks every N seconds. Simple, but latency and load
   are coupled to N.
2. **SSE.** One long-lived HTTP response, server-to-client text events. Built into
   browsers, reconnects automatically, works over ordinary HTTP.
3. **WebSocket.** Bidirectional, framed, more moving parts. Useful when the
   client also sends a stream.

This product needs server-to-client notifications: task created, updated,
deleted. The client does not need a socket to send edits; it already has forms
and an API. SSE is the smaller tool.

SSE is one-directional. The server cannot ask the client for anything. That is a
feature here: fewer states, no ping/pong protocol, no shared socket lifecycle.

## The event type

Create `src/events.rs`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AppEvent {
    pub kind: String,
    pub workspace_id: String,
    pub task: Option<Task>,
}
```

`kind` is `task.created`, `task.updated`, or `task.deleted`. The workspace ID is
the routing key for membership filtering. The task is optional because a future
event may carry a different payload or only an ID.

The task is the full domain value, serialized by serde. That is convenient for a
small client and expensive for a large one. A busy workspace sends the whole task
on every event; a client that only needs the ID ignores the rest. A future
protocol sends IDs and lets the client pull the row, which keeps the stream
small and the change log authoritative.

The event has no sequence number. That is the biggest gap in this chapter. A
durable stream needs an ID so a reconnect can say “send me everything after
this.” The change log already has `seq`; the bus does not. Exercise 1 adds it.

## The bus

`EventBus` wraps a broadcast sender:

```rust
#[derive(Debug, Clone)]
pub struct EventBus {
    sender: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, event: AppEvent) {
        let _ = self.sender.send(event);
    }
}
```

`capacity` is 256 in `AppState::new`. A broadcast channel keeps a ring buffer of
the last `capacity` messages. A subscriber that falls behind receives a `Lagged`
error and misses the skipped messages. A subscriber that connects after an event
does not receive it. There is no persistence.

`publish` ignores the send result. Sending fails when there are no subscribers,
which is normal. A task change with nobody listening is not an error.

`subscribe` must be called before the events the client cares about; the receiver
only sees messages sent after subscription.

The bus is held in `AppState` and cloned with the state. The sender is cheap to
clone; each clone points at the same ring buffer.

## Publishing

The web handlers publish after a successful mutation:

```rust
state.events.publish(AppEvent {
    kind: "task.created".to_owned(),
    workspace_id: task.workspace_id.clone(),
    task: Some(task.clone()),
});
```

Completion publishes `task.updated` with the updated task. Delete publishes
`task.deleted` with the task as it was before the soft delete, because the
`delete` method returns `()`. The event payload is a snapshot, not a tombstone
flag; the change log carries `deleted_at`.

The API create handler publishes too. The API sync endpoint does **not** publish
per operation. That is a real gap: a CLI push changes the server and writes the
change log, but connected browsers do not receive a live event. They see the
change on the next refresh or sync pull. The fix is to publish from
`SyncService::push` for each successful result, or to derive the stream from the
change log instead of a separate bus. The exercise asks for the former.

Publishing after the database commit is important. An event that says “created”
before the row is visible would make a client fetch a 404. The current handlers
publish after the service returns, which is after the commit.

The event send is best-effort. If the broadcast buffer is full, `publish` drops
the message for slow subscribers. The database still has the change; the client
recovers on reconnect with a pull.

## The web stream

`GET /events` uses the session cookie:

```rust
async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>>, AppError> {
    let user = require_user(&state, &headers).await?;
    let allowed: HashSet<String> = state
        .workspaces
        .list_workspaces(&user.user.id)
        .await?
        .into_iter()
        .map(|workspace| workspace.id)
        .collect();
    let receiver = state.events.subscribe();

    let stream = BroadcastStream::new(receiver).filter_map(move |message| {
        let event = message.ok()?;
        if !allowed.contains(&event.workspace_id) {
            return None;
        }
        let data = serde_json::to_string(&event).ok()?;
        Some(Ok(Event::default().event(event.kind).data(data)))
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
```

`require_user` redirects anonymous browsers to `/login`. The browser’s
`EventSource` sends cookies automatically, so the session is available.

The workspace set is loaded once, at connect time. A user who joins a new
workspace while the stream is open will not receive its events until they
reconnect. A user who is removed keeps receiving events for that workspace until
the connection closes, because the set is a snapshot. Both are reasons to treat
the stream as a hint and rely on the change log for authorization-sensitive
state.

The filter is a `HashSet<String>`. It is small and read-only after construction.
Do not query the database for every event; that would turn a cheap stream into a
per-event load.

`BroadcastStream` converts the receiver into a stream. `filter_map` drops:

- lag errors (`message.ok()?` returns `None`),
- events for other workspaces,
- events that fail to serialize.

Dropping a lag error is silent. A client that fell behind does not know it missed
events. A robust protocol sends a `resync` event or closes the connection so the
client reconnects and pulls. The current stream keeps going with a gap.

`Sse::new(stream).keep_alive(KeepAlive::default())` sends periodic comment lines.
Keep-alive prevents proxies from closing an idle connection and lets the client
detect a dead stream. The default interval is 15 seconds in axum.

## The API stream

`GET /api/v1/events` is the same handler with `ApiUser`:

```rust
user.require(Scope::Read)?;
let allowed: HashSet<String> = state.workspaces
    .list_workspaces(&user.user_id).await?
    .into_iter().map(|workspace| workspace.id).collect();
```

A browser `EventSource` cannot set an `Authorization` header, so the cookie route
is for browsers and the bearer route is for native clients, `curl`, and tests.
Do not accept a token in a query string; URLs end up in logs and browser history.

The same membership snapshot and lag behavior apply. The API route does not add
a cursor or replay.

## The wire format

An SSE response is a stream of text records:

```text
event: task.created
data: {"kind":"task.created","workspace_id":"01J...","task":{...}}

event: task.updated
data: {"kind":"task.updated","workspace_id":"01J...","task":{...}}

: keep-alive
```

The `event:` line sets the event name; `data:` is one JSON line. A blank line
ends the record. `Event::default().event(...).data(...)` formats it. The
`Content-Type` is `text/event-stream`; axum sets it.

A client listens by event name:

```js
source.addEventListener("task.updated", (event) => {
    const payload = JSON.parse(event.data);
});
```

The event name is redundant with `payload.kind`; keeping both makes routing easy
without parsing a line that may be large. Do not put a newline inside `data`; a
multi-line record is possible but unnecessary for one JSON object.

## Reconnect and replay

`EventSource` reconnects automatically and sends `Last-Event-ID` if the server set
an `id:` field. This server does not set one, so a reconnect starts fresh with no
replay. Events during the disconnect are lost.

The change log is the repair path: on reconnect, the client pulls from its stored
cursor. The stream does not need to be durable if the pull is reliable.

A durable stream would:

1. Set `id:` to the change `seq`.
2. Read `Last-Event-ID` on connect.
3. Replay changes after that sequence, then switch to live.
4. Send a resync marker if the cursor is too old (compaction).

That is more machinery than this chapter needs, but it is the difference between
a live hint and a reliable feed. The exercise asks for the first two steps.

## Event lifecycle

| Event | Published by | Payload | Notes |
| --- | --- | --- | --- |
| `task.created` | web/API create | full task | after commit |
| `task.updated` | web/API toggle | full task | includes new version |
| `task.deleted` | web delete | pre-delete task | soft delete already committed |
| none | CLI sync | — | known gap |

The events are not persisted. If a subscriber connects after `task.created`, it
does not get the create; it gets the next event. The change log is the history;
the stream is the live edge.

The publish happens after the service call. A failed service call publishes
nothing. A successful call whose publish fails (no subscribers, full buffer) still
changed the database; the client repairs by pulling.

## Why broadcast

Three channel shapes are easy to confuse:

- **mpsc**: many producers, one consumer. A worker queue.
- **watch**: one value, latest wins. A configuration flag.
- **broadcast**: many producers, many consumers, bounded ring. A live feed.

SSE has one receiver per connection and many connections. `broadcast` is the
matching shape. Each connection gets its own receiver; `publish` clones the event
for each receiver. A slow receiver lags and loses old messages rather than
blocking the publisher.

A `watch` channel would keep only the latest event and lose the rest. An `mpsc`
with one receiver would let one connection consume everyone else’s events. The
choice is not stylistic; it follows the fan-out.

The capacity is a backpressure policy. 256 events is small enough to bound memory
and large enough for a burst. If a client cannot keep up with 256 events, it
needs a resync, not a bigger buffer.

## Testing

The bus has one unit test:

- `publishes_to_subscribers`: subscribe, publish a typed event, receive it.

There is no automated SSE integration test. The handler returns a stream that
never ends, so a test has to read a bounded prefix with a timeout. The manual
check is:

```console
$ curl -N -b cookies.txt http://127.0.0.1:3000/events
event: task.updated
data: {"kind":"task.updated","workspace_id":"...","task":{...}}
```

Open the app in two tabs, toggle a task in one, and watch the other receive the
event. The browser’s network panel shows the `text/event-stream` request.

The unit test proves the bus; the manual check proves the route, auth, filtering,
and keep-alive. Do not claim the route is tested by the bus test.

## Checkpoint

```console
$ cargo test events
$ cargo test
$ cargo run -- serve
$ curl -N -b cookies.txt http://127.0.0.1:3000/events
```

The stream stays open. Toggle a task in another tab; the event appears. Stop the
server; the client sees the stream close.

## Failure modes you will meet

**The stream opens but no events arrive.** The task was changed by the CLI sync
endpoint, which does not publish. Use the web/API create or toggle path, or add
publishing to sync.

**Events from another workspace arrive.** The workspace set was built from a
different user or the filter uses the wrong ID. Check `list_workspaces` and the
event’s `workspace_id`.

**A user removed from a workspace keeps receiving events.** The allowed set is a
snapshot. Close and reopen the stream, or re-check membership per event with a
short cache.

**Events stop after a burst.** A subscriber lagged past the 256-message buffer.
The stream silently drops the lag error. Reconnect and pull from the change log.

**A proxy closes the stream after 60 seconds.** Keep-alive comments are missing or
buffered by the proxy. Disable response buffering for `text/event-stream` and keep
the interval below the proxy timeout.

**`EventSource` gets 401/302.** The cookie is missing, expired, or `SameSite`
blocks it. Check the browser console and the login redirect.

**The same event appears twice.** The client reconnected and the server replayed
from a sequence that includes an already-applied change. Dedupe by change `seq`,
not by payload equality.

**The stream leaks memory.** Each connection holds a receiver and a `HashSet`.
Close the connection when the client disconnects; axum drops the stream when the
body is dropped. Monitor open connections.

## Under the hood

`broadcast` is a multi-producer, multi-consumer channel with a bounded ring. It
clones each message for each receiver. The clone is cheap for `AppEvent` because
the task is cloned per subscriber; a huge payload would make fan-out expensive.
Keep events small.

`BroadcastStream` polls the receiver and yields `Result`. The `Lagged(n)` error
tells you how many messages were skipped. The handler drops it; a better handler
maps it to a `resync` SSE event.

`Sse` wraps a stream of `Event`s in the HTTP body. It does not buffer or retry.
The client reconnects; the server does not remember.

SSE is HTTP/1.1-friendly and works through most proxies when buffering is off.
HTTP/2 multiplexes many streams over one connection. WebSockets are a different
upgrade with different proxy behavior; do not assume one is universally easier.

## Exercises

1. **Add event IDs and replay.** Include the change `seq` in `AppEvent`, set
   `id:` on the SSE record, read `Last-Event-ID`, and replay from the change log
   before switching live. Test a reconnect with a gap.
2. **Publish from sync.** After each successful sync operation, publish the same
   event the web handler would. Test that a CLI push updates an open stream.
3. **Send resync on lag.** Map `Lagged` to an SSE `resync` event. The client then
   pulls from its cursor.
4. **Filter by project.** Add a query parameter for a project ID and filter
   events to that project. Test two projects in one workspace.
5. **Compare with WebSockets.** Implement the same stream over a WebSocket and
   write a short note on the operational differences. Do not ship both.

## What comes next

The server can push events, but the browser does not use them yet. Chapter 18
adds a tiny `EventSource` script that refreshes the MPA when a task changes, with
a no-JavaScript fallback that still works.
