---
title: Glossary
description: Terms used across the book, in the sense Cadence uses them.
---

**ACL / authorization matrix.** A table of roles and actions. Cadence tests the
rows rather than trusting a hidden UI.

**API token.** A random bearer credential for the JSON API, stored as a hash and
scoped to read, write, or admin.

**At-least-once.** A delivery guarantee that permits duplicates after a crash or
lost response. Reminders and webhooks use it.

**Backoff.** A delay before retrying a failed operation. Cadence grows the delay
with the attempt count.

**Bearer token.** A credential the client sends on every request. Possession is
authority; there is no proof-of-possession.

**Change log.** An append-only table of task changes with a monotonic `seq`,
used by the sync client. It is not the audit log.

**Claim.** A state transition that gives a worker ownership of a queued row
(`pending` → `sending`). A lease adds an expiry to the claim.

**CSRF.** Cross-site request forgery. Cadence uses a session-scoped token in
forms and `SameSite=Lax` as a second layer.

**Cursor.** The last change sequence a client has applied. The next pull asks
for `seq > cursor`.

**DST.** Daylight saving time. A local 09:00 task keeps its wall time across a
DST change; the UTC instant moves.

**HMAC.** A keyed hash used to sign webhook payloads. The receiver recomputes it
with the shared secret.

**Idempotency key.** A client-chosen string that lets a retried create return the
stored response instead of creating a second row.

**Keyset pagination.** Paging by an ordered key (`created_at, id` or `seq`)
instead of an offset. Stable under inserts.

**Lease.** A claim with an expiry so a crashed worker’s rows can be recovered.
Cadence has a reminder claim but no lease yet.

**Migration.** An append-only SQL file that changes the schema. Applied
migrations must not be edited.

**MPA.** Multi-page application. Cadence renders HTML on the server and uses
forms, redirects, and cookies; no client-side framework.

**Optimistic concurrency.** A write succeeds only if the row's version matches
the version the client read. A mismatch is a conflict, not a silent overwrite.

**RRULE.** A recurrence rule syntax. Cadence supports a documented subset.

**Scope.** A coarse API permission (`read`, `write`, `admin`). It does not
replace the workspace role.

**Session.** A server-side row plus an HttpOnly cookie. Deleting the row is the
real logout.

**Soft delete.** Setting `deleted_at` instead of removing the row, so sync
clients receive a tombstone.

**SSE.** Server-sent events. A one-way HTTP stream used for live task updates.

**SSRF.** Server-side request forgery. A webhook URL is untrusted; the server
must not fetch private networks or metadata endpoints.

**Tenant.** The isolation boundary for data. In Cadence, the workspace.

**Tombstone.** A deleted row retained in the change log so clients can remove
their local copy.

**TTL.** Time to live. Verification and reset tokens expire; sessions expire;
idempotency records expire after a day.

**Version.** The per-task edit counter used for optimistic concurrency. It is
not the global change sequence.

**WAL.** SQLite write-ahead log. Readers can proceed during a write; there is
still one writer.

**Webhook.** An HTTP callback to an external system, signed and retried from a
delivery queue.
