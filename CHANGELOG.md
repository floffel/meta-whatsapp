# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the crate adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html); the policy for
0.x versions and the MSRV is stated in the
[versioning section](https://github.com/floffel/meta-whatsapp#versioning-and-msrv)
of the README.

## [Unreleased]

### Added
- Typed inbound flow replies. `InteractiveReply::nfm_reply` was an untyped
  `serde_json::Value`; it is now a `FlowReply` (`name`, `body`, `response_json`,
  plus `extra` for members Meta adds later) reachable through
  `Message::flow_reply()`. The flow's answers arrive JSON-encoded in a string,
  so `FlowReply::response()` decodes them into a `FlowResponse` with
  `flow_token()`, `get()` and `field()`; a missing or malformed
  `response_json` is `Error::Payload` and the raw string stays on the struct.
  Breaking for anyone who read the untyped object. Closes #8.
- `FlowAction::navigate_with_data` and a validation for
  `flow_action_payload.data`: a non-object payload is now `Error::Validation`
  instead of being rendered and sent. Closes #8 together with the typed reply
  above.
- Catalog product messages (#19): `SingleProductMessage` (`interactive.type`
  `product`) and `MultiProductMessage` (`product_list`) with `ProductSection` and
  `ProductItem`, plus the matching `InteractiveMessage::Product` /
  `InteractiveMessage::ProductList` variants and `From` conversions into
  `InteractiveMessage` and `MessageBody`. The contract's asymmetries are in the
  types rather than in a runtime check: `product_list` needs a text header and a
  body, so `MultiProductMessage::new` takes both, while `product` forbids a
  header and allows the body to be omitted, so `SingleProductMessage` has no
  header field at all.
- Catalog limits as named constants: `PRODUCT_SECTIONS_MAX` (10),
  `PRODUCT_ITEMS_MAX` (30 per section), `PRODUCTS_MAX` (30 in the whole message,
  Meta's reference cap on top of the per-section one) and
  `PRODUCT_SECTION_TITLE_MAX_CHARS` (24). Section titles are required once a
  message has more than one section, and `catalog_id` / `product_retailer_id`
  must be present and printable before anything is rendered —
  `Error::Validation` names the offending wire field.
- Tests: four recording-transport tests pinning the exact JSON body of both
  families (`tests/messages.rs`), a boundary matrix for each limit including
  multi-byte titles and the 30/31-product split across sections, a proptest
  pinning multi-product acceptance to the documented envelope, and the new
  builders added to the surface sweep.
- Phone-number lifecycle (#13): `WhatsAppClient::register`,
  `WhatsAppClient::register_with_backup`, `WhatsAppClient::deregister` and
  `WhatsAppClient::set_two_step_verification_pin`, with the matching
  `register_url`, `deregister_url` and `two_step_verification_url` builders. A
  freshly provisioned number cannot send at all until `register` succeeds, so
  this closed the last per-number gap in the README's tracked list; the
  WABA-level onboarding surface stays with issue #9.
- `VerificationPin`, a newtype for the six-digit registration / two-step PIN. It
  cannot be constructed unless the value is exactly six ASCII digits (ASCII
  only: `１２３４５６` and `١٢٣٤٥٦` are six characters and are refused), it is
  `Debug`- and `Display`-redacted, has no getter, no `Hash` and no `Default`,
  and compares in constant time. The PIN never appears in a `Display`, `Debug`
  or error message, and it is validated before anything is allocated or sent.
  `RegistrationBackup` redacts its password and blob the same way.
- `SuccessOutcome` for the `{"success": true}` answer all three calls give back,
  plus `RegisterRequest` and `TwoStepVerificationRequest` for the bodies.
  Permissive as elsewhere: a missing flag, `1`/`0` and `"true"`/`"1"` all
  decode, and unknown fields survive in `extra`.
- Delivery correctness for these writes: they are non-idempotent, so a 5xx or an
  answer lost after the write surfaces as `Error::Ambiguous` and is never
  re-sent, while 429 and pre-write failures stay retryable. `deregister`
  overrides the verb rule — `DELETE` is normally idempotent, deregistering twice
  is not — via `execute_with(…, Repeatability::NotIdempotent)`. The decision
  remains the same pure `RetryPolicy::decide`.
- Two-step verification can be set and rotated but **not** disabled: the
  reference offers no endpoint for it, and the rustdoc says so instead of
  implying a `disable` is missing.
- Tests: `tests/lifecycle.rs` (14 tests — wire shape per call, the PIN boundary
  pairs including unicode digits, and an assertion that the PIN appears in no
  `Debug`/`Display` string of any request or error type), six lifecycle
  delivery cases in `tests/delivery.rs` including a 32-task hammer test on a
  cloned client, and three properties in `tests/properties.rs`.

### Changed
- `transport::Request` and `RequestBody` now redact in `Debug`: the access token
  is replaced with `[redacted]`, and so is any body field named `pin`,
  `password`, `backup`, `token`, `access_token`, `verify_token` or `app_secret`,
  recursively. A transport can log a whole request without leaking a PIN.

### Fixed
- `Retry-After` is no longer shortened to `max_delay` (#14). The hint is the
  server's own schedule: `RetryPolicy::delay` now waits it out verbatim while it
  fits under `max_delay`, and a longer hint answers `Action::Surface` instead of
  a truncated wait. The caller sees the 429 with `retry_after()` intact and
  `is_retryable() == true`, so it can schedule the wait the server asked for;
  waiting less used to guarantee a second 429 and burn one of the attempts.
  `max_delay` now decides *whether* a hint is acceptable, never how much of it
  to wait — the backoff curve is still clamped by it, and
  `without_retry_after()` still hands the decision to the curve. The boundary is
  pinned to the nanosecond in `src/retry.rs`, `tests/delivery.rs`,
  `tests/client.rs` (a 3600 s hint is slept for exactly 3600 s of paused time;
  3601 s against a 3600 s ceiling is not slept at all) and by two new properties
  in `tests/properties.rs`.

## [0.1.5] - 2026-10-27

### Changed
- The query encoder is `percent-encoding` 2.3.2 (released 2025-08-21, 1.03 bn
  downloads) instead of a hand-rolled byte loop. The escape set is
  `NON_ALPHANUMERIC` minus `-`, `.`, `_` and `~` — RFC 3986's unreserved set —
  so the wire output is unchanged: `%20` for a space, `%2C` for the field-list
  separator, uppercase hex, and a literal `%` escaped to `%25`. A new property
  test decodes every rendered query value back and compares it with the input.
  Closes #6.
- The retry module documents why its backoff curve stays local: `backoff` 0.4
  (the crate the audit named) has had no release since 2021-12 and needs
  `&mut self`; `backon` 1.6 computes in `f32` seconds (a 100 ms base yields
  200.000 003 ms) and indexes its curve by walking it; `retry-policies` 0.5
  answers with a `SystemTime` and takes no `Retry-After`. The arithmetic itself
  is unchanged, and its exactness is now pinned by tests.
- README: "Deliberately out of scope" became "Tracked gaps". Every unimplemented
  surface already had an issue (#7, #8, #9), so the section now points at them
  instead of presenting them as decisions.

### Added
- `retry` unit tests for the boundary the curve has to survive: exact doubling
  from an arbitrary base, a ceiling below the base delay, a zero base delay, and
  a `u32::MAX` budget that must stay exact and cheap.

## [0.1.4] - 2026-10-25

### Added
- `examples/webhook.rs` — verifies a signed payload and parses it with no
  network and no credentials (`cargo run --example webhook`); asserts that a
  tampered body and a replayed body are both rejected.
- `examples/send_text.rs` — sends over `ReqwestTransport`, shows the
  `Error::Ambiguous` arm, and prints the rendered request instead of sending
  when the credentials are absent, so it runs without an account.
- README › Examples, and a CI step `Examples build` that compiles both.

No API change.

## [0.1.3] - 2026-10-25

### Added
- `fuzz/` with one libFuzzer target per parser — `webhook_payload`,
  `webhook_signature`, `template_payload`, `account_payload`, `header_values`.
  Each target pins the invariant it protects (an unmodelled key survives in
  `extra`, a signature header verifies only when byte-identical to the genuine
  HMAC, no raw space/`#`/tab/newline escapes query encoding, every typed error's
  `Display` runs), not merely "does not panic". Seeds are real payloads; the
  signature seed carries a genuine HMAC for the fuzz secret.
- CI job `coverage`: `cargo llvm-cov --all-features --fail-under-lines 95`.
- CI job `fuzz`: builds all five targets and runs 5 000 iterations per target on
  the committed corpus.

No public API change.

## [0.1.2] - 2026-10-25

### Changed
- Dependency majors: `hmac` 0.13 and `sha2` 0.11 (`digest` 0.11),
  `reqwest` 0.12 → 0.13 (`json` + `rustls`). `reqwest` 0.13 validates against
  the platform trust store and no longer ships bundled `webpki-roots`, so a
  minimal container needs `ca-certificates` (or its own `reqwest::Client`).

### Fixed
- The local-socket transport test reads the request head before answering, so it
  no longer races the write.

No API change.

## [0.1.1] - 2026-10-25

### Added
- Explicit ambiguous delivery semantics: `Error::Ambiguous`, `Error::is_ambiguous()`
  and `RetryPolicy::decide` as one pure, testable rule set. A 5xx or a lost
  response after the request was written is never retried for a non-idempotent
  request; 429 and pre-write transport failures stay retryable.
- Account surface: business profile, phone-number quality rating and messaging
  limits, blocklist, webhook subscriptions, and Meta's
  `X-Business-Use-Case-Usage` budget header.
- `docs/contracts.md`: the provider documentation each unverified shape was read
  from, with the shapes that stay guesses marked as such.

### Changed
- README and crate docs document the delivery contract per operation
  (at-most-once / at-least-once / ambiguous).

## [0.1.0] - 2026-10-24

### Added
- Webhook toolkit: constant-time `X-Hub-Signature-256` verification, the `GET`
  handshake, a replay window, and payloads parsed into typed events
  (messages, statuses, unknown future events carried through instead of
  rejected).
- `WhatsAppClient` over the whole message surface: text, media, location,
  contacts, reaction, and interactive buttons/list/flow, plus template messages;
  media upload/download/delete and template listing.
- `Transport` trait with the `reqwest` implementation behind the default-on
  `http` feature; payload and webhook types stay usable without it.
- Typed error enum (`thiserror`, `#[non_exhaustive]`) with retryable vs terminal
  classification, `Retry-After` parsing and a rate-limit budget.
- A quickstart in the README and a runnable doctest on every
  public item; dual license Apache-2.0 OR MIT; CI with fmt, clippy
  (`-D warnings`), tests with and without default features, docs and
  `cargo-deny`.

### Fixed
- The constructor rejects an empty app secret (HMAC with an empty key is well
  defined, so a missing environment variable must not become an auth bypass).
- `Debug` for the client redacts the access token.

[Unreleased]: https://github.com/floffel/meta-whatsapp/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/floffel/meta-whatsapp/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/floffel/meta-whatsapp/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/floffel/meta-whatsapp/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/floffel/meta-whatsapp/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/floffel/meta-whatsapp/releases/tag/v0.1.0
