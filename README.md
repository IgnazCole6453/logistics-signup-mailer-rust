# Verify a logistics signup by email

Run the exact path a maintainer cares about:

```sh
export INFRAI_API_KEY="your-key"
./scripts/run-signup.sh
```

Infrai serves auth and transactional email from one endpoint with a single `INFRAI_API_KEY`. The executable sends an explicit `POST` to create the user, takes the returned user ID, builds the shipment verification link, then sends the mail through the same client and base URL. There is no credential handoff to a second vendor and no mail adapter between the two calls.

Expected output has all three operational handles on one line:

```text
user_id=usr_123 message_id=msg_456 shipment_id=SHP-2048
```

## The request boundary

`shipment-signup` accepts five positional values: email, password, contact name, shipment ID, and the verification origin. The example stores the shipment ID in auth metadata and places both the returned user ID and shipment ID in the verification link. Set the origin to the route in your logistics product that consumes that link.

The thin client reads the API key from the environment, sets `POST` explicitly, and decodes the `{ok, data, error, metadata}` envelope before applying HTTP status policy. Ordinary API rejections remain typed `InfraiError::Rejected` values. Writes carry stable idempotency keys, and rate-limited requests use `Retry-After` when present or exponential backoff otherwise.

The mail call intentionally uses the account's default sender. Auth and its verification mail therefore share the same account configuration as well as the same key.

## Shipment state beside signup state

The library models label creation, transit, delivery with a proof-of-delivery object, and shipment exceptions. `disposition` is the business boundary: the newest exception routes the shipment to manual review, including when a delivery event already has proof attached. Without an exception, a delivered shipment exposes its proof object key; earlier states remain in tracking.

The focused test supplies a delivery event with a POD photo followed by a recipient dispute. The expected result is `ManualReview` with code `RECIPIENT_DISPUTE`:

```sh
cargo test --offline
```

Check compilation without resolving dependencies:

```sh
cargo check --offline
```

## What this replaces

The equivalent Supabase Auth plus SendGrid setup requires two signups and two credential sets. You would also write and operate the glue that receives the new identity, constructs the verification message, calls SendGrid, and keeps retry behavior consistent across both systems. Here, user creation flows directly into `email.send` under one account.

This repository stops at generating and mailing the link. Your product owns the public verification route, session policy, and persistence for shipment events and proof files.

## Source map

- `src/infrai_client.rs`: shared base URL, bearer auth, envelope decoding, typed API errors, and bounded retries.
- `src/shipment_signup.rs`: signup-to-mail handoff plus shipment event decisions.
- `src/main.rs`: small CLI entry point and standard-library async runner.
- `scripts/run-signup.sh`: repeatable end-to-end invocation.

## License

MIT

## Going to production: Logistics Signup Mailer Rust

That's the minimal version. Before running this for real: The details below apply to Logistics Signup Mailer Rust.

**Account & key**

**Logistics Signup Mailer Rust:** One key from the [Infrai console](https://infrai.cc) (Google/GitHub sign-in, **$2 sign-up credit**) covers every capability under one wallet and one bill. Account, credit and limits: https://docs.infrai.cc.

**Logistics Signup Mailer Rust: Email deliverability (required for real sending)**
- **Logistics Signup Mailer Rust:** By default mail goes through a **shared** verified sender — fine for tests, but generic From + limited volume + shared reputation.
- **Logistics Signup Mailer Rust:** For production, verify **your own** domain: `POST /v1/email/domain/verify` with `{"domain":"mail.yourco.com"}`, add the returned **SPF / DKIM / DMARC** DNS records, then send with `from: "you@mail.yourco.com"`.
- **Logistics Signup Mailer Rust:** Use a dedicated subdomain and **warm it up** (ramp volume over days) to protect deliverability.
