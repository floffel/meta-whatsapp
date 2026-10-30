# meta-whatsapp

Typed client and webhook toolkit for the [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api) (Meta Graph API).

Three independent parts, all usable on their own:

* **Webhook** — constant-time `X-Hub-Signature-256` verification, GET handshake, replay window, and the payload parsed into types (message events, statuses, unknown future events).
* **Client** — every message family, media upload/download/delete, template listing, and account administration (business profile, phone-number quality and limits, blocklist, webhook subscriptions).
* **Account types** — `business`, `subscription` and `usage` modules behind that surface, including Meta's `X-Business-Use-Case-Usage` budget header.

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

## Examples

Two runnable examples, both compiled in CI:

```bash
cargo run --example webhook     # verify and parse a signed payload, offline
cargo run --example send_text   # send a text message over the reqwest transport
```

`webhook` needs nothing: it signs a fixture with a throwaway secret, verifies it,
prints the inbound messages and statuses, and asserts that a tampered body *and*
a replayed body are both rejected — the two failure modes a webhook endpoint has
to get right.

`send_text` reads `WHATSAPP_PHONE_NUMBER_ID`, `WHATSAPP_TOKEN` and `WHATSAPP_TO`.
Without the credentials it prints the JSON body it would POST and exits, so the
example builds and runs on a machine with no WhatsApp account, and the
`Error::Ambiguous` arm shows what to do when a send's outcome is unknown.

## Delivery semantics

Sending a message is **not** safely repeatable: the Cloud API has no idempotency
key, so a second POST is a second message on the recipient's phone. The client
therefore automates only the retries that cannot duplicate anything:

| Failure | Non-idempotent (`POST` send/upload/register, `deregister`) | Idempotent (`GET`, other `DELETE`) |
| --- | --- | --- |
| 429 rate limited | retried; `Retry-After` waited out verbatim, surfaced if it exceeds `max_delay` | same |
| DNS / connect / TLS failure (before the write) | retried | retried |
| 5xx | **not retried** → `Error::Ambiguous` | retried with backoff |
| Timeout or reset after the write | **not retried** → `Error::Ambiguous` | retried |
| Other 4xx | terminal | terminal |

`deregister` is the one `DELETE` in the table's left column: deregistering twice is a
state change the caller has to see, so it overrides the verb rule
(`Repeatability::of_method`) and is treated as non-idempotent, exactly like
`register` and `set_two_step_verification_pin`.

`Error::Ambiguous` means *the message may or may not have been delivered*:
`is_retryable()` is `false`, `is_ambiguous()` is `true`, `status()` carries the
5xx when one arrived, and `source()` is the underlying Graph/transport error.
Retrying it would duplicate the message; dropping it may lose one. That decision
belongs to the caller, not to the library.

The three delivery guarantees, in the usual vocabulary:

* **at-most-once** — never retry an ambiguous send (safe against duplicates, a
  message can be lost).
* **at-least-once** — retry an ambiguous send anyway (no loss, possible
  duplicates).
* **ambiguous** — surface it and let a human/ledger decide, which is what the
  default does.

Because the API cannot deduplicate for you, carry your own key and use it on the
receiving side: WhatsApp returns `biz_opaque_callback_data` on the status
webhooks for exactly that (set it with
`MessageRequest::biz_opaque_callback_data`), or keep your own outbox ledger with
one row per send attempt. `mark_read` is treated as idempotent (marking read
twice leaves the same state), so a 5xx there is retried.

The retry decision is one pure function, `RetryPolicy::decide`, and the loop
around it only sleeps: no sleeping, blocking or hidden retry lives in the
library's data path. `RetryPolicy::delay` remains the pure backoff arithmetic —
exact integer shifts on `Duration`, no float curve. Issue #5 asked for that
arithmetic to be delegated to a retry crate; the three candidates (`backoff`,
`backon`, `retry-policies`) were measured and each loses something this crate
needs, so the reasons live in the `retry` module docs next to the code.

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
| catalog products | `SingleProductMessage`, `MultiProductMessage` + `ProductSection` | one product card, or up to 30 products in up to 10 sections |
| template | `TemplateMessage` | text/currency/date-time/media parameters, header/body/button merges |

Envelope-level features: reply context (`context.message_id`),
`biz_opaque_callback_data`, and `recipient_type`.

Catalog product messages (`interactive.type` `product` and `product_list`) need a
catalog linked to the WhatsApp Business Account. The wire pins the shapes
unevenly, so the types follow it: `product_list` requires a text header and a
body — both are constructor arguments of `MultiProductMessage` — while `product`
forbids a header and allows the body to be omitted, so `SingleProductMessage`
has no header field at all and an optional `body`. A multi-product message takes
1–10 sections; every section needs a title once there is more than one, carries
1–30 products, and the message holds at most 30 products in total. Section titles
are 24 characters, header and footer 60, the body 1024 bytes.

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

## Business account

Account administration for the number the client is bound to:

| Call | Endpoint | Returns |
| --- | --- | --- |
| `business_profile` | `GET /{phone_number_id}/whatsapp_business_profile` | `BusinessProfile` (about, address, description, e-mail, websites, vertical) |
| `update_business_profile` | `POST /{phone_number_id}/whatsapp_business_profile` | `{"success": true}` |
| `register` | `POST /{phone_number_id}/register` | `SuccessOutcome` (`{"success": true}`) — a number cannot send until this succeeds |
| `register_with_backup` | same, plus `backup{password,data}` | same, migrating an on-premises account in the same call |
| `deregister` | `DELETE /{phone_number_id}/deregister` | `SuccessOutcome` |
| `set_two_step_verification_pin` | `POST /{phone_number_id}` (body `pin` only) | `SuccessOutcome` — the only spelling; the API cannot disable two-step verification |
| `phone_number_info` | `GET /{phone_number_id}?fields=…` | `PhoneNumberInfo` (quality rating, messaging-limit tier, account mode, display-name and 2FA status) |
| `phone_number_info_with` | same, explicit field list | same, but you choose the fields |
| `blocked_users` | `GET /{phone_number_id}/block_users` | `BlockedUsersPage` (with paging) |
| `block_users` | `POST /{phone_number_id}/block_users` | `BlockOutcome` (`added_users`) |
| `unblock_users` | `DELETE /{phone_number_id}/block_users` (ids in the body) | `BlockOutcome` (`removed_users`) |

The three lifecycle writes take or return a `VerificationPin`: a value that only
exists if it is exactly six ASCII digits, is `Debug`- and `Display`-redacted, has
no getter and no `Default`, and never appears in an error message. They are
non-idempotent — a 5xx or a lost answer is `Error::Ambiguous`, never a silent
re-send (see *Delivery semantics*).

Quality and limits are typed but permissive: `QualityRating` (GREEN/YELLOW/RED/NA)
and `MessagingLimitTier` (TIER_50 … TIER_UNLIMITED) keep an unknown value as
`Unknown(String)` instead of failing, and `PhoneNumberInfo::daily_contact_limit()`
turns the tier into a number. `permits_broadcast()` is our own conservative
policy (not RED, not SANDBOX), not a field Meta returns.

## Rate-limit budget

`send_with_usage` returns the parsed body *and* Meta's
`X-Business-Use-Case-Usage` header as `BusinessUseCaseUsage` (call count, CPU and
wall-clock budget, estimated time to regain access, per business and use case).
Any `Response` exposes the same via `Response::business_use_case_usage()`.

The header is optional and unparsable values yield `None` — telemetry never fails
a send. Use it to slow down *before* a 429; it does not change delivery
semantics.

## Webhook subscriptions

| Call | Endpoint |
| --- | --- |
| `webhook_subscriptions(waba_id)` | `GET /{waba_id}/subscribed_apps` |
| `subscribe_webhooks(waba_id, config)` | `POST /{waba_id}/subscribed_apps` (`override_callback_uri`, `verify_token`) |
| `unsubscribe_webhooks(waba_id)` | `DELETE /{waba_id}/subscribed_apps` |
| `ensure_webhook_subscription(waba_id, app_id)` | list, then subscribe **only if** the app is missing |

`ensure_webhook_subscription` is the safe thing to call on start-up and again on
an error path: it never re-POSTs an existing subscription, and it reports
`SubscriptionState::{AlreadySubscribed, Subscribed}` so you can log which
happened. A 401 from the list is returned as is instead of attempting a write
with a token the API just rejected.

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

* Only 429 and pre-write connection failures are retried automatically; see
  [Delivery semantics](#delivery-semantics).
* Backoff is exponential and exact, bounded by `RetryPolicy` (default 3
  attempts, 250 ms doubling to an 8 s ceiling). A `Retry-After` hint is
  authoritative and is never shortened: it is waited out verbatim while it fits
  under `max_delay`, and a longer hint surfaces the 429 (still
  `is_retryable()`, with `retry_after()` intact) so the caller can schedule it —
  waiting less than the server asked for only earns a second 429 and burns an
  attempt. `max_delay` therefore decides *whether* to retry, not how long;
  `without_retry_after()` ignores the header and lets the curve decide.
* Per-request timeout via `WhatsAppClient::with_timeout`; waits use
  `tokio::time::sleep` (cancellation safe). Without the `http` feature there is
  no timer to await, so retries happen immediately — bring your own pacing.
* `ReqwestTransport` classifies `reqwest` failures by phase: `is_connect()` is
  "before the write" (retryable), a timeout or body/decode error is "after the
  write" (ambiguous for a send), and anything else is terminal.
* TLS trust store: the `http` feature builds `reqwest` with
  `default-features = false, features = ["json", "rustls"]`. `reqwest` 0.13
  validates against the platform trust store (`rustls-platform-verifier`) and
  no longer offers the bundled `webpki-roots` feature, so a minimal container
  without `ca-certificates` must install one (or supply its own
  `reqwest::Client` through the `Transport` trait).

## Unverified against the provider docs

Meta's published Cloud API description does not pin these down, so they are
handled permissively instead of guessed at. Treat them as the parts most likely
to need a patch release — `docs/contracts.md` records the sources:

* **Business-profile envelope.** The reference shows the profile fields at the
  top level and inside a `data` array, the OpenAPI example wraps them again in
  `business_profile`. `BusinessProfile::parse` accepts all three shapes; only the
  field names are certain.
* **Phone-number node envelope.** The spec returns a flat object; the parser also
  accepts `{"data": [{…}]}` in case a proxy or a future version wraps it.
* **`subscribed_apps` entries.** The spec example wraps each app in
  `whatsapp_business_api_data`, but not every documented example does; both parse.
* **Field-level subscription.** There is no `subscribed_fields` parameter on the
  WABA node — field selection lives on the app-level `/{app_id}/subscriptions`
  endpoint, which this crate does not implement. `subscribe_webhooks` covers the
  WABA-level subscription only.
* **Re-subscribe on 401.** Nothing in the docs says a 401 implies a lost
  subscription, so `ensure_webhook_subscription` re-reads instead of
  re-subscribing blindly.
* **Usage-header counters.** The field set comes from the header example; unknown
  counters are ignored, not typed.
* **Blocklist batch size.** No documented maximum, so none is enforced beyond the
  per-number validation.
* **`success` on the lifecycle calls.** The reference types it as a boolean, but the
  field is also seen as `1`/`0`, as `"true"`/`"1"`, or absent. `SuccessOutcome`
  accepts every one of those and reads a missing flag as `false`; only a body that
  cannot be an object at all is an error.
* **`backup{password,data}` on register.** The migration blob for an on-premises
  Business API move; the OpenAPI description does not pin the shape down, so both
  parts are opaque strings here and the caller supplies whatever its export produced.
* **Catalog ids and retailer ids.** The spec types both as plain strings and gives
  no maximum length, so neither is capped; they are checked for emptiness and
  control characters only. Whether an id exists in the catalog is Meta's answer,
  not ours.

## Tracked gaps

A Cloud API surface this crate does not implement is an issue, not an exclusion.
The list below is the shortcut; the issue is the contract, and an entry leaves
this section when its issue closes.

* **Catalog product messages are covered** (`SingleProductMessage`,
  `MultiProductMessage`, issue #19). What is still open from issue #7 are the
  **order and address messages** (`interactive.type` `order_details`,
  `address_message`), which carry their own schema churn; `MessageBody` stays
  `#[non_exhaustive]` so they can land without a breaking release. The
  `catalog_message` action (`{name: "catalog_message"}` with a thumbnail product)
  is not implemented either: it is a catalog-level "view catalog" entry point,
  not a product card, and belongs to its own issue.
* **Flow data-exchange endpoints** — issue #8. Until then the inbound
  `nfm_reply` payload is passed through raw (`InteractiveReply::nfm_reply`);
  decrypting and serving flows is a server concern, not a client one.
* **App-level webhook field subscription** (`/{app_id}/subscriptions`) and
  registering the callback URL itself — issue #9.
* **WABA-level phone-number listing / onboarding** (`GET|POST /{waba_id}/phone_numbers`,
  its `register`, `request_code`) — issue #9. The per-number quality and limits *and*
  the per-number lifecycle (register / deregister / two-step PIN) *are* covered above;
  what is left is enumerating and onboarding numbers from the WABA node, where the
  code request has no per-number PIN.

The only exclusion that does not need an issue is a surface the provider does
not offer, or has deprecated — and then the README says which, with a link.

## Testing

```bash
cargo test                          # unit + integration + doctests, no network
cargo test --no-default-features    # same, without reqwest/tokio
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
cargo deny check
cargo llvm-cov --all-features --summary-only   # 96.3 % lines, gate at 95 %
cargo fuzz run webhook_payload      # nightly; one target per parser, see below
```

The suite has five layers:

* **Unit** — multipart encoding, Retry-After parsing, error mapping, retry
  classification, response-envelope unwrapping.
* **Integration** (`tests/`) — a recording `Transport` asserts method, path,
  bearer token, timeout and exact JSON body per message type; edge cases include
  empty text, the 4096-byte boundary, invalid media ids, malformed signatures,
  truncated JSON, duplicate keys, unknown event types and 32-task concurrency
  tests. `tests/delivery.rs` pins the retry contract call-by-call;
  `tests/account.rs` covers profile/quality/limits/blocklist/subscriptions/usage;
  `tests/reqwest_transport.rs` drives the bundled transport against a local
  socket and a refused connection.
* **Property** (`proptest`) — webhook parsing never panics over arbitrary JSON,
  sign/verify round-trips for arbitrary secrets and bodies, text acceptance
  matches the documented boundary, backoff stays bounded, queries stay
  percent-encoded, and "no non-idempotent request is retried after a 5xx" holds
  for every status, budget and attempt count.
* **Doctest** — every public item has a runnable example (403 doctests).
* **Fuzz** (`fuzz/`, nightly) — one libFuzzer target per parser, each pinning the
  invariant it protects instead of only "does not panic":
  * `webhook_payload` — arbitrary JSON through the permissive parser; a key the
    crate does not model must come back out of `extra`, not be dropped.
  * `webhook_signature` — a header verifies only if it is byte-identical to the
    genuine HMAC, and no header shape panics the verifier.
  * `template_payload` — list parsing plus the query string: no raw space, `#`,
    tab or newline may escape the percent-encoding.
  * `account_payload` — profile, phone-number, blocklist, block-result and
    subscription parsers, including the `Display` of every typed error.
  * `header_values` — `Retry-After`, the usage-budget header, and the round trip
    of the two permissive string enums through their wire spelling.

  The seeds in `fuzz/corpus/` are real payloads; the signature seed carries a
  genuine HMAC for the fuzz secret, so the parser is reached on the first
  iteration. CI builds all five targets and runs 5 000 iterations each on the
  committed corpus.

## Versioning and MSRV

[Semantic Versioning](https://semver.org/spec/v2.0.0.html), with the 0.x caveat
stated rather than implied: while the major version is 0, a **minor** bump
(`0.1` → `0.2`) may carry a breaking change, and a **patch** bump
(`0.1.2` → `0.1.3`) never does. `#[non_exhaustive]` on the error and payload
enums plus additive message families mean most provider-side additions land in a
patch release; a deliberate break waits for the next minor version.

* Every release is tagged `v<version>`, and the tag has to equal the version in
  `Cargo.toml` — the release workflow refuses to publish a mismatch.
* [CHANGELOG.md](CHANGELOG.md) has one section per released version, and CI fails
  when the manifest's version has no section.
* **MSRV is Rust 1.89**, declared as `rust-version` in `Cargo.toml`. Raising it is
  a minor-version change and is named in the changelog entry.
* docs.rs builds with `all-features = true`.
* A fix to a shape listed as *unverified* in `docs/contracts.md` (see
  [Unverified against the provider docs](#unverified-against-the-provider-docs))
  is a patch release: the earlier version was guessing, not promising.

## License

Licensed under either of Apache-2.0 or MIT at your option.
