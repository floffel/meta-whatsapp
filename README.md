# meta-whatsapp

Typed client and webhook toolkit for the [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api) (Meta Graph API).

Two independent halves, both usable on their own:

* **Webhook** — constant-time `X-Hub-Signature-256` verification, GET handshake, replay window, and the payload parsed into types (message events, statuses, unknown future events).
* **Client** — every message family, media upload/download/delete, and template listing, with retries on 429/5xx.

The HTTP layer sits behind the `Transport` trait: tests need no network, and you
can plug in your own stack (or the bundled `reqwest` one behind the `http`
feature, which is on by default).

```rust
use meta_whatsapp::{WebhookVerifier, WhatsAppClient, ReqwestTransport};

let verifier = WebhookVerifier::new(std::env::var("META_APP_SECRET")?)?;
let payload = verifier.verify_fresh(body_bytes, &signature_header, now_unix)?;

for (phone_number_id, message) in payload.inbound_messages() {
    println!("{phone_number_id}: {} says {:?}", message.from, message.text_body());
}
```

```rust
let client = WhatsAppClient::new(phone_number_id, access_token, ReqwestTransport::new());
client.send_text("4915112345678", "Hallo!").await?;

let image = ImageMessage::new(MediaRef::id("987654321")).caption("Speisekarte");
client.send(&MessageRequest::new("4915112345678", image)).await?;
```

## Messages

Every family is a struct with a constructor and chainable setters; `MessageRequest::to_json`
validates before rendering, so an invalid message never reaches the network.

| Body | Struct | Notes |
| --- | --- | --- |
| text | `TextMessage` | 1..=4096 bytes, optional link preview |
| image / video / audio / document / sticker | `ImageMessage`, `VideoMessage`, `AudioMessage`, `DocumentMessage`, `StickerMessage` | by media id or https link, captions, filename, voice notes |
| location | `LocationMessage` | range- and `NaN`-checked |
| contacts | `ContactsMessage` + `ContactCard` | name, phones, e-mails, org, URLs, birthday |
| reaction | `ReactionMessage` | empty emoji removes a reaction |
| interactive | `ButtonMessage`, `ListMessage`, `FlowMessage` | 1–3 buttons, 1–10 list rows, flows |
| template | `TemplateMessage` | text/currency/date-time/media parameters, header/body/button merges |

Envelope-level features: reply context (`context.message_id`),
`biz_opaque_callback_data`, and `recipient_type`.

## Media

* `upload_media` POSTs `multipart/form-data` to `/{phone_number_id}/media` and returns the id.
* `media_info` resolves id → URL, MIME type, SHA-256 and size.
* `download_media` (by id) and `download_from_url` (the CDN URL) return bytes plus content type.
* `delete_media` cleans up before Meta's 30-day expiry.

Uploads are validated before the request is built: MIME allowlist, per-type size
ceiling (image 5 MB, video/audio 16 MB, document 100 MB, sticker 500 KB), empty
payloads, and filenames that would break the multipart header. Media ids are
restricted to `[A-Za-z0-9_-]` so they cannot escape the URL path.

## Templates

`TemplateMessage` sends; `TemplateQuery` + `list_templates` reads
`/{waba_id}/message_templates` with cursor paging (`limit`, `after`, `before`,
`status`, `name`, `language`, `fields`). `TemplateList::has_next_page` tells you
whether to follow `paging.next`.

## Webhook

* Signature comparison uses `subtle::ConstantTimeEq` over the 32 HMAC bytes.
* An empty app secret is refused at construction (HMAC with an empty key would accept anything).
* `verify_fresh` additionally rejects payloads whose message/status timestamps
  fall outside the replay window (default 300 s, `with_tolerance(Duration::ZERO)` disables).
* Bodies over `max_body_bytes` (default 1 MiB) are rejected before parsing.
* Both `changes[].value` shapes parse: message/status events with typed fields,
  and account-level events (`message_template_status_update`,
  `phone_number_quality_update`, …) whose keys land in `ChangeValue::extra`.
* Unknown fields survive in `extra`; unknown `type` strings stay strings;
  duplicate *known* keys are rejected with a typed JSON error instead of being
  guessed at.

## Resilience

* 429 and 5xx are retried with exponential backoff, bounded by `RetryPolicy`
  (default 3 attempts, 250 ms doubling to an 8 s ceiling).
* `Retry-After` is honoured and clamped to `max_delay`.
* 4xx, signature, validation and decoding errors are terminal.
* Per-request timeout via `WhatsAppClient::with_timeout`; waits use
  `tokio::time::sleep` (cancellation safe). Without the `http` feature there is
  no timer to await, so retries happen immediately — bring your own pacing.

## Deliberately out of scope

These are Cloud API surfaces this crate does not implement. They are listed so
the gap is a decision, not an oversight:

* **Business profile, phone-number info, quality/messaging limits, blocklist** —
  account administration, not messaging. Separate concern from a send/receive client.
* **Webhook subscription management** (`subscribed_fields` GET/POST, re-subscribe
  on 401) — deployment-time configuration; treat it as infrastructure.
* **Catalog / product / order / address messages** — commerce-specific payloads
  with their own schema churn; `MessageBody` stays `#[non_exhaustive]` so they can
  be added without a breaking release.
* **Flow data-exchange endpoints** — the inbound `nfm_reply` payload is passed
  through raw (`InteractiveReply::nfm_reply`); decrypting/serving flows is a server concern.
* **Rate-limit budget tracking** beyond `Retry-After` (Meta's
  `X-Business-Use-Case-Usage` header is not parsed).
* **`cargo-fuzz` targets and a coverage job** — property tests cover the parsers
  and renderers; libFuzzer harnesses and a tarpaulin gate are not wired into CI yet.

## Testing

```bash
cargo test                          # unit + integration + doctests, no network
cargo test --no-default-features    # same, without reqwest/tokio
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
```

The suite has three layers:

* **Unit** — multipart encoding, Retry-After parsing, error mapping.
* **Integration** (`tests/`) — a recording `Transport` asserts method, path,
  bearer token, timeout and exact JSON body per message type; edge cases include
  empty text, the 4096-byte boundary, invalid media ids, malformed signatures,
  truncated JSON, duplicate keys, unknown event types and a 32-task concurrency test.
* **Property** (`proptest`) — webhook parsing never panics over arbitrary JSON,
  sign/verify round-trips for arbitrary secrets and bodies, text acceptance
  matches the documented boundary, backoff stays bounded, queries stay
  percent-encoded.

## License

Licensed under either of Apache-2.0 or MIT at your option.
