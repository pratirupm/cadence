# 19. Webhooks

**What you can do at the end.** Register an HTTPS webhook, receive a signed
delivery for a task event, and see retries and a dead letter after repeated
failures.

**The Rust you learn.** HMAC signatures, SSRF defense, retry scheduling,
provider boundaries, and not trusting user-supplied URLs.

**Build it.**

- Add webhook and delivery tables.
- Require an admin to create a webhook.
- Accept only `https` URLs and reject localhost, loopback, private ranges, and
  `.local` hosts.
- Sign `secret + payload` with HMAC-SHA256 and send
  `X-Cadence-Signature: sha256=...`.
- Queue one delivery per subscribed event.
- A worker delivers due rows, retries with backoff, and marks `failed` at five
  attempts.
- List webhooks without exposing the secret; show the secret once on create.

The signature is the receiver’s proof:

```rust
let mut mac = HmacSha256::new_from_slice(secret.as_bytes())?;
mac.update(payload.as_bytes());
let signature = hex::encode(mac.finalize().into_bytes());
```

**Checkpoint.**

```console
$ cargo test webhooks
$ cargo run -- worker --once
```

The tests cover create/list, event filtering, private-URL rejection, and stable
signatures.

**Exercises.**

1. Add a delivery log page and manual replay.
2. Add a test for a 500 response retry schedule.
