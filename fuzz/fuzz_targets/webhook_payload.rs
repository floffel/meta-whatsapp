#![no_main]
//! Arbitrary bytes must not panic the webhook parser.
//!
//! The payload parser is deliberately permissive: unknown fields are carried in
//! `extra`, unknown event types stay strings. Fuzzing therefore asserts two
//! things — it never panics, and an unknown key survives the round trip instead
//! of being dropped or rejected.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::{AddressReply, ChangeValue, WebhookPayload};

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
        // An address reply is a JSON *string* inside `nfm_reply`: decoding it
        // must be total for any content, and must not lose modelled fields.
        if let Some(Ok(reply)) = AddressReply::from_message(message) {
            let _ = reply.saved_address_id.as_deref();
            let _ = reply.values.get("city");
            let _ = reply.values.is_empty();
        }
    }
    // A flow reply is decoded from a stringified document: the accessors must
    // stay total whatever that string holds, and the wire object must survive
    // a round trip through the typed form.
    for (_phone_number_id, message) in payload.inbound_messages() {
        let Some(reply) = message.flow_reply() else {
            continue;
        };
        let _ = reply.name.as_deref();
        let _ = reply.body.as_deref();
        let _ = reply.response_json.as_deref();
        let _ = reply.extra.len();
        if let Ok(response) = reply.response() {
            let _ = response.flow_token();
            let _ = response.field("screen");
            let _ = response.get("data");
            let _ = response.as_value().is_object();
        }
        let encoded = serde_json::to_value(reply).expect("serialising a reply cannot fail");
        let decoded: meta_whatsapp::FlowReply =
            serde_json::from_value(encoded).expect("a serialised reply must decode again");
        assert_eq!(
            &decoded, reply,
            "a decoded reply must survive a round trip"
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
