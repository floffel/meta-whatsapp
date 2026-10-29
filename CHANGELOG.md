# Changelog

All notable changes to this crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the crate adheres
to [Semantic Versioning](https://semver.org/spec/v2.0.0.html); the policy for
0.x versions and the MSRV is stated in the
[versioning section](https://github.com/floffel/meta-whatsapp#versioning-and-msrv)
of the README.

## [Unreleased]

### Added
- Address messages: `AddressMessage` renders Meta's `address_message`
  interactive form (`header`, `body`, `footer` and the action parameters
  `country`, `values`, `saved_addresses`, `validation_errors`), validated before
  the request is built — a two-letter ISO 3166-1 `country`, a non-empty body,
  `in_pin_code` ≤ 6 characters, every other value ≤ 1024 UTF-8 bytes, unique
  saved-address ids and at most one validation error per field.
  `AddressValues` carries the eleven documented fields under their wire names
  (`in_pin_code` is the postal code, `address` the street line); unknown keys
  survive in `extra`. `InteractiveMessage::Address`, `From<AddressMessage> for
  MessageBody`, and a new `address` module with `AddressReply`, which decodes
  the `nfm_reply.response_json` string of a submitted address. Closes #22.
- Tests: the exact JSON body for a full and a minimal form, one boundary pair
  per limit (country length, body bytes, field bytes, pin-code characters,
  phone digits, header/footer, id and message lengths), the documented webhook
  payload, property tests for the byte limits and the wire round-trip, and an
  `address_message` seed for the existing `webhook_payload` fuzz target, which
  now decodes every `nfm_reply` it meets.

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
