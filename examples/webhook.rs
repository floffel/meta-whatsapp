//! Verify and parse a webhook offline: `cargo run --example webhook`.
//!
//! A webhook endpoint does two things, in this order: check the
//! `X-Hub-Signature-256` header against the raw body, then parse the body. Both
//! live in the crate, so this example needs no network, no credentials and no
//! HTTP framework — the body below is signed with a throwaway secret at
//! runtime, exactly as Meta would sign it.
//!
//! The order matters: parse first and an attacker chooses your payload, verify
//! first and a forged body stops at `Error::SignatureMismatch`.

use meta_whatsapp::WebhookVerifier;

/// Stands in for `META_APP_SECRET`. A real deployment reads it from the
/// environment and never hard-codes it.
const APP_SECRET: &str = "example-app-secret";

/// The moment we pretend to receive the payload, in unix seconds. It has to
/// match the `timestamp` inside the body, otherwise the replay window rejects
/// the example before the parse is reached.
const NOW_UNIX: u64 = 1_729_600_000;

/// One batch of webhook events as Meta posts it: an inbound text message and a
/// delivery status for an earlier outbound message.
const BODY: &str = r#"{
  "object": "whatsapp_business_account",
  "entry": [{
    "id": "102290129340398",
    "changes": [{
      "field": "messages",
      "value": {
        "messaging_product": "whatsapp",
        "metadata": {
          "display_phone_number": "4915112345678",
          "phone_number_id": "106540352242922"
        },
        "messages": [{
          "id": "wamid.inbound",
          "from": "4915198765432",
          "timestamp": "1729600000",
          "type": "text",
          "text": { "body": "Hallo" }
        }],
        "statuses": [{
          "id": "wamid.outbound",
          "status": "delivered",
          "timestamp": "1729600001",
          "recipient_id": "4915198765432",
          "biz_opaque_callback_data": "order-42"
        }]
      }
    }]
  }]
}"#;

fn main() -> meta_whatsapp::Result<()> {
    // An empty secret is refused rather than turned into an HMAC with an empty
    // key, so `new` returns a `Result`.
    let verifier = WebhookVerifier::new(APP_SECRET)?;
    let body = BODY.as_bytes();

    // What the sender computes with the same secret. In production this arrives
    // in the `X-Hub-Signature-256` header.
    let signature = verifier.sign(body);

    // `verify_fresh` is signature check plus replay window in one call; pass the
    // current unix time. Use `verify_and_parse` when your framework has already
    // done the freshness check.
    let payload = verifier.verify_fresh(body, &signature, NOW_UNIX)?;

    // Typed iteration over the same envelope, so callers never walk
    // `entry[].changes[].value` by hand.
    for (phone_number_id, message) in payload.inbound_messages() {
        println!(
            "{phone_number_id} <- {} ({}) {:?}",
            message.from,
            message.id,
            message.text_body().unwrap_or("<not a text message>")
        );
    }

    for status in payload.statuses() {
        println!(
            "{} -> {} by {} (correlation: {})",
            status.id,
            status.status,
            status.recipient_id,
            status.biz_opaque_callback_data.as_deref().unwrap_or("-")
        );
    }

    // Reject, do not sanitise: one changed byte invalidates the HMAC.
    let tampered = BODY.replacen("Hallo", "Hallo!", 1);
    assert!(
        verifier
            .verify_fresh(tampered.as_bytes(), &signature, NOW_UNIX)
            .is_err(),
        "a tampered body must not verify"
    );
    println!("tampered body rejected");

    // The replay window is part of `verify_fresh`: the same signed body an hour
    // later is stale, so a captured request cannot be posted twice.
    assert!(
        verifier
            .verify_fresh(body, &signature, NOW_UNIX + 3_600)
            .is_err(),
        "a replayed body must be outside the tolerance"
    );
    println!("replayed body rejected");

    Ok(())
}
