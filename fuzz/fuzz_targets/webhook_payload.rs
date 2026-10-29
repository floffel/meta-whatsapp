#![no_main]
//! Arbitrary bytes must not panic the webhook parser.
//!
//! The payload parser is deliberately permissive: unknown fields are carried in
//! `extra`, unknown event types stay strings. Fuzzing therefore asserts two
//! things — it never panics, and an unknown key survives the round trip instead
//! of being dropped or rejected.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::{ChangeValue, WebhookPayload};

fuzz_target!(|data: &[u8]| {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };

    // Permissiveness as an invariant: a key the crate does not model must come
    // back out untouched, never silently discarded.
    let mut probe = value.clone();
    if let serde_json::Value::Object(map) = &mut probe {
        map.insert(
            "fuzz_marker".to_string(),
            serde_json::Value::String("kept".to_string()),
        );
    }
    let change = ChangeValue::from_value(probe);
    if value.is_object() {
        assert_eq!(
            change.extra_str("fuzz_marker"),
            Some("kept"),
            "an unknown key was dropped by ChangeValue::from_value"
        );
    }
    let _ = change.event();
    let _ = change.is_empty();

    // The payload side: valid JSON of the wrong shape is an error, never a panic.
    let Ok(payload) = serde_json::from_slice::<WebhookPayload>(data) else {
        return;
    };
    for (phone_number_id, message) in payload.inbound_messages() {
        let _ = phone_number_id.len();
        let _ = message.media();
        let _ = message.text_body();
        let _ = message.timestamp_secs();
    }
    // A flow reply is decoded from a stringified document: the accessors must
    // stay total whatever that string holds, and the wire object must survive.
    for reply in payload.flow_replies() {
        let _ = reply.flow_token.as_deref();
        let _ = reply.flow_name.as_deref();
        let _ = reply.screen();
        let _ = reply.data().map(|data| data.len());
        let _ = reply.get("screen");
        assert_eq!(
            serde_json::to_value(reply).expect("serialising a reply cannot fail"),
            reply.raw,
            "a decoded reply must serialise back to the object Meta sent"
        );
    }
    for status in payload.statuses() {
        let _ = status.timestamp_secs();
    }
    for change in payload.changes() {
        let _ = change.value.event();
        let _ = change.value.is_empty();
    }
    // Replay protection reads the timestamps; it must cope with any of them.
    let _ = payload.newest_timestamp();
    let _ = payload.timestamps().count();
});
