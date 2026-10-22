# meta-whatsapp

Typed client and webhook toolkit for the WhatsApp Cloud API (Meta Graph API).

Two independent halves, both usable on their own:

* **Webhook** — constant-time `X-Hub-Signature-256` verification, the GET handshake, and the payload parsed into types.
* **Client** — send text messages and mark inbound messages as read via the Graph API.

The HTTP layer sits behind the `Transport` trait: tests need no network, and
you can plug in your own stack (or the bundled `reqwest` one).

```rust
use meta_whatsapp::{WebhookVerifier, WhatsAppClient, ReqwestTransport};

let verifier = WebhookVerifier::new(std::env::var("META_APP_SECRET")?);
let payload = verifier.verify_and_parse(body_bytes, &signature_header)?;

for (phone_number_id, message) in payload.inbound_messages() {
    println!("{phone_number_id}: {} says {:?}", message.from, message.text);
}
```

```rust
let client = WhatsAppClient::new(phone_number_id, access_token, ReqwestTransport::new());
client.send_text("4915112345678", "Hallo!").await?;
```

## Scope

General-purpose: nothing in this crate knows about a particular product or
database. Persistence, routing and retries stay in the consumer.

## Testing

```bash
cargo test                          # unit + integration tests, no network
cargo test --no-default-features    # client compiles without reqwest
cargo doc --open
```

## License

Licensed under either of Apache-2.0 or MIT at your option.
