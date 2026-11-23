# 23. Deployment

**What you can do at the end.** Build a container, run the service with a
persistent SQLite volume, migrate, back up, and restore.

**The Rust you learn.** Release builds, static assets, environment
configuration, health checks, graceful shutdown, and backup discipline.

**Build it.**

- Add a multi-stage `Dockerfile`: build with Rust, copy one binary into a slim
  runtime image.
- Add `docker-compose.yml` with a named volume and `CADENCE_DATABASE=/data/...`.
- Add `cadence backup --output backup.db`, implemented with `VACUUM INTO`.
- Run migrations explicitly before the first start.
- Expose `/health` and `/metrics`; terminate TLS at a reverse proxy.
- On SIGTERM, stop accepting connections and let in-flight requests finish.
- Test a restore into a fresh volume.

The backup is a consistent snapshot, not a file copy of a live WAL:

```console
$ cadence backup --database /data/cadence.db --output /backup/cadence.db
```

**Checkpoint.**

```console
$ docker compose up --build
$ curl http://127.0.0.1:3000/health
$ cargo run -- backup --database /tmp/cadence.db --output /tmp/backup.db
$ ls -l /tmp/backup.db
```

**Exercises.**

1. Add a `/ready` check that also verifies the database.
2. Write a restore drill and run it once.
