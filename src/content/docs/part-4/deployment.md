---
title: "Deployment"
description: "Package the binary, migrate, back up, restore, and shut down."
---

A service that only runs in a development shell is not deployed. It needs a
release binary, a persistent database, a process that restarts, a worker that
drains reminders, a way to back up, and a way to stop without losing a request.

This chapter packages Cadence as a single container plus a worker, with SQLite on
a volume and a repeatable backup/restore procedure. It does not pick a cloud. It
picks the interfaces a host must provide: a binary, a writable data directory, a
TCP port, environment configuration, SIGTERM, and a place to put backups.

By the end of this chapter you can:

- build the release binary
- build a multi-stage image with a non-root user and a writable data volume
- run the server and worker against one SQLite file
- apply migrations in a one-shot step
- back up with `VACUUM INTO` and restore into a fresh file
- explain why a live file copy is not a backup
- terminate TLS at a reverse proxy
- identify missing production controls: readiness, HSTS, external metrics

The Rust lessons are release builds, environment configuration, and graceful
shutdown. The operations lesson is that a database file is a deployment artifact:
it needs an owner, a backup, and a restore drill.

## The release build

```console
$ cargo build --release --bin cadence
```

The binary is at `target/release/cadence`. It embeds the Askama templates, the CSS,
the live script, and the migrations. The runtime does not need a `templates/`,
`static/`, or `migrations/` directory.

The release profile is defined in `Cargo.toml`:

```toml
[profile.release]
lto = true
codegen-units = 1
strip = true
```

`lto` and one codegen unit trade compile time for a smaller, faster binary.
`strip` removes symbols; keep a separate debug build if you need to symbolize a
crash. The exact profile is a deployment choice; the binary is the interface.

Book one compared a debug and release binary by size. This book does not need a
size claim; measure your own build if size matters.

## The image

`Dockerfile` is multi-stage:

```dockerfile
FROM rust:1.99-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY templates ./templates
COPY static ./static
COPY migrations ./migrations
RUN cargo build --release --bin cadence

FROM debian:bookworm-slim

RUN useradd --create-home --uid 10001 cadence \
    && mkdir -p /data \
    && chown cadence:cadence /data
COPY --from=builder /app/target/release/cadence /usr/local/bin/cadence

USER cadence
WORKDIR /home/cadence
ENV CADENCE_DATABASE=/data/cadence.db
VOLUME ["/data"]
EXPOSE 3000

CMD ["cadence", "serve", "--bind", "0.0.0.0:3000"]
```

The builder copies only what the build needs. The runtime copies one binary. No
Rust toolchain, no source, no target directory.

The runtime user is not root. `/data` is created and owned by that user before the
volume declaration. Without the `mkdir`/`chown`, a named volume can mount
root-owned and the process cannot create the database. This is the first
deployment bug most SQLite containers hit.

The image does not install a shell utility set. A health check needs a binary
that can make an HTTP request; the service binary does not have a `health` mode
yet. Add one or install a minimal client, and keep the image small.

Docker is not installed in the environment where this chapter was written, so the
image build is a checkpoint for your machine, not a verified local run. The
release binary and the backup command are verified.

## Compose

`docker-compose.yml` has three services:

```yaml
services:
  migrate:
    build: .
    environment:
      CADENCE_DATABASE: /data/cadence.db
      RUST_LOG: info
    volumes: [cadence-data:/data]
    command: ["cadence", "migrate"]
    restart: "no"

  cadence:
    build: .
    depends_on:
      migrate:
        condition: service_completed_successfully
    ports: ["3000:3000"]
    environment:
      CADENCE_DATABASE: /data/cadence.db
      RUST_LOG: info
    volumes: [cadence-data:/data]
    command: ["cadence", "serve", "--bind", "0.0.0.0:3000"]
    restart: unless-stopped

  worker:
    build: .
    depends_on:
      migrate:
        condition: service_completed_successfully
    environment:
      CADENCE_DATABASE: /data/cadence.db
      RUST_LOG: info
    volumes: [cadence-data:/data]
    command: ["cadence", "worker"]
    restart: unless-stopped
```

The one-shot `migrate` service avoids two long-running services racing through
the migration table. The server and worker still call `migrate` on startup in the
current binary, but it is idempotent; the one-shot step makes the ordering
explicit and the logs readable.

The worker is a separate process. It shares the SQLite file with the server. WAL
allows readers during a write, but only one writer at a time; keep worker
transactions short. Do not scale the worker to many replicas against one SQLite
file without a claim/lease and a shared queue design.

`restart: unless-stopped` restarts after a crash or host reboot, but not after a
manual stop. It is not a health check. A process that starts and cannot serve is
still “running” to Compose.

## Configuration

The binary reads:

- `CADENCE_DATABASE` — SQLite path or URL; default `cadence.db`
- `--bind` — listen address; default `127.0.0.1:3000`
- `RUST_LOG` — tracing filter; default `info`
- `CADENCE_PASSWORD` — password for `user create`
- `CADENCE_SERVER` / `CADENCE_TOKEN` — CLI client configuration

The token secret, SMTP/provider credentials, and any external base URL are not
configured yet. When you add them, read them from the environment or a mounted
secret file; never commit them. The database and any future object storage are
state; the binary is stateless except for them.

A container should bind `0.0.0.0`, not `127.0.0.1`. The host maps a port; the
process inside the container must accept connections from the network namespace.
The default is loopback for safety in local development; Compose overrides it.

## Migrations

`cadence migrate` connects, applies `sqlx::migrate!`, and exits. It is safe to run
twice; applied migrations are skipped. It is not safe to edit an applied
migration; SQLx checks the checksum.

A deployment sequence:

```text
1. build the image
2. run the migrate service (or `cadence migrate`)
3. start the server
4. start the worker
5. verify /health and a real request
```

If a migration is slow, it runs before the new code serves. That is the point of a
separate step: a failed migration leaves the old version running, not a
half-migrated process.

The current `serve` also migrates. That is convenient for a single binary but
means a restart can apply a migration before the operator expects it. For a
controlled rollout, set a flag to skip auto-migration, or keep the one-shot
service and accept the duplicate check.

## Backups

SQLite in WAL mode has a main file, a `-wal`, and a `-shm`. Copying only the main
file while a write is in progress can produce an inconsistent copy. Use the
backup command:

```console
$ cadence backup --database /data/cadence.db --output /backup/cadence.db
backup written to /backup/cadence.db
```

`VACUUM INTO` writes a consistent snapshot of the database into a new file. It
does not copy the WAL; the output is a standalone database. It is safe while the
service is running, but it reads the whole database and takes a write lock at the
end. Schedule it in a quiet window or accept the brief pause.

A backup policy:

- run daily and before a migration
- keep several generations off the host
- encrypt at rest if the database contains private data
- record the binary version and migration version with the backup
- test a restore, not just a backup

The command currently takes an explicit output path. A production wrapper adds a
timestamp, retention, and upload. Do not put the backup inside the same volume
and call it off-host; a volume loss takes both.

## Restore drill

A backup is not real until it restores. The drill:

```console
$ docker compose stop cadence worker
$ cp /backup/cadence.db /data/cadence.db
$ rm -f /data/cadence.db-wal /data/cadence.db-shm
$ docker compose start cadence worker
$ curl -f http://127.0.0.1:3000/health
```

Stop the writers before replacing the file. Remove stale `-wal`/`-shm` files;
they belong to the old database. Start the service, apply migrations if the
backup is older than the binary, and verify a real read.

A local drill with a file database:

```console
$ cadence migrate --database /tmp/restore.db
$ cadence backup --database /tmp/restore.db --output /tmp/restore-backup.db
$ ls -l /tmp/restore-backup.db
```

The backup command was verified this way. It does not prove a remote volume, but
it proves the SQL and the file format.

## Shutdown

The server installs a graceful-shutdown future:

```rust
axum::serve(listener, app(AppState::new(pool)))
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

`shutdown_signal` waits for Ctrl+C or SIGTERM. On SIGTERM, axum stops accepting
new connections and waits for in-flight requests. The worker loops use the same
signal under `tokio::select!`.

A container stop sends SIGTERM, then SIGKILL after a timeout. Set the timeout
longer than a normal request; do not rely on SIGKILL. A background email send
that outlives the request is not drained by axum; it needs its own shutdown
coordination.

## TLS and the proxy

The binary speaks HTTP. Terminate TLS at a reverse proxy (Caddy, Nginx, a cloud
load balancer) and forward to `127.0.0.1:3000` or the container port.

The proxy must:

- set `X-Forwarded-Proto` and `X-Forwarded-For` only from a trusted hop
- pass the `Host` header or the configured public host
- disable response buffering for `/events` (`text/event-stream`)
- set a request body limit
- add HSTS on HTTPS; the app does not set it
- keep `/metrics` private, not internet-facing

The app’s CSP, `nosniff`, frame, and referrer headers pass through the proxy. Do
not duplicate or weaken them.

## Health and readiness

`/health` returns a constant JSON body. It is a liveness check: the process is
running. It does not prove the database is reachable or migrations are current.

A readiness check should:

- run `SELECT 1` with a short timeout
- verify the schema version
- return 503 with a short body on failure

Keep `/health` cheap and unauthenticated for the orchestrator. Put `/ready` behind
the same internal network if it reveals version information. The exercise asks
for it.

## Deployment checklist

| Step | Command | Verify |
| --- | --- | --- |
| Build | `cargo build --release --bin cadence` | binary exists |
| Image | `docker compose build` | image tag |
| Migrate | `docker compose run --rm migrate` | “migrations applied” |
| Start | `docker compose up -d cadence worker` | containers running |
| Liveness | `curl -f localhost:3000/health` | 200 JSON |
| Real read | `curl -f .../api/v1/.../tasks` | 200 JSON |
| Queue | `curl -s .../metrics` | gauges present |
| Backup | `cadence backup ...` | file size > 0 |
| Restore | restore into a fresh volume | real read works |
| Stop | `docker compose stop` | graceful log |

The table is the runbook. A deploy that skips the real read and the restore is a
deploy that has not been tested.

## What is not production-ready

The book is complete as a teaching service, not as a finished SaaS. The remaining
production gaps are explicit:

- no HSTS or `Secure` cookie flag; add under HTTPS
- no `/ready` database check
- no external log/metrics backend or alerting
- no shared rate limiter or API token rate limit
- no lease/recovery for in-flight reminders or webhook deliveries
- no automated backup retention or off-host upload
- no request IDs or audit IPs
- no pagination on the task list
- SQLite single-writer scaling limits
- no password reset email provider; the `LogMailer` is a seam

Naming these is part of finishing the project. A service that claims to be
production-ready without them is lying; a service that lists them is honest and
testable.

## Failure modes you will meet

**The container starts and immediately exits.** The database path is not writable,
the port is in use, or the bind address is loopback inside the container. Check
logs and the `/data` ownership.

**`database is locked` under load.** A worker batch or a long transaction holds
the writer. Keep transactions short, reduce concurrency, or move the queue to a
different database.

**The backup is inconsistent.** Someone copied the main file without the WAL or
while a write was active. Use `VACUUM INTO` or stop the writers and checkpoint.

**The restore starts but data is missing.** The backup predates the latest
writes, or the wrong file was copied. Check timestamps and the migration version.

**The worker does not deliver.** It is not running, or it cannot write the shared
database. Check the service logs and the queue gauges.

**SIGTERM takes too long.** A request or a worker task is not cancellable. Set a
shutdown timeout and ensure long tasks check cancellation.

**`/metrics` is public.** Restrict it at the proxy. Metrics reveal traffic and
queue health.

**The image is huge.** The builder leaked into the runtime, or the binary was not
stripped. Check the stages and the profile.

## Under the hood

A multi-stage build uses one image for compilation and another for runtime. The
final image contains only the copied artifacts and the OS libraries they need.
The Rust binary is dynamically linked against glibc in this image; a musl build
would be more portable but changes the build target and TLS story.

SQLite is a library, not a server. The container needs a writable directory, not
a database service. That is a simplicity win and a scaling limit: one writer,
one file, one host unless you add replication or a different engine.

A named volume outlives a container. A bind mount is a host directory. Both need
an owner and a backup. A container filesystem without a volume loses the database
on replacement.

## Exercises

1. **Add `/ready`.** Check `SELECT 1` and the migration version. Return 503 on
   failure and test with a closed pool.
2. **Add a container health check.** Add a `cadence health --url` mode or install a
   minimal HTTP client. Wire it to Compose.
3. **Automate backup retention.** Write a wrapper that timestamps, uploads, and
   deletes old backups. Test the restore of the newest and oldest.
4. **Add lease recovery.** Reminders claim with `sending`, but have no lease
   timeout; webhooks have no claim at all. Add a lease and recovery pass so two
   workers do not deliver the same reminder or webhook.
5. **Document a rollback.** Deploy version N+1, then return to N. What happens to
   the database schema? Which migrations are reversible? Write the runbook.

## What comes next

The service can start, stop, migrate, and restore. Chapter 24 is the capstone: one
end-to-end scenario that touches every layer, plus the operational judgment to
call the service ready.
