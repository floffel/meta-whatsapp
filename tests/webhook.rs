//! Webhook hardening: constant-time signature checks, replay window, permissive
//! parsing, and fail-closed behaviour.

use std::time::Duration;

use meta_whatsapp::{Error, WebhookPayload, WebhookVerifier};

const BODY: &[u8] = br#"{"object":"whatsapp_business_account","entry":[]}"#;

fn verifier() -> WebhookVerifier {
    WebhookVerifier::new("secret").unwrap()
}

fn message_payload(timestamp: &str) -> Vec<u8> {
    format!(
        r#"{{"object":"whatsapp_business_account","entry":[{{"id":"1","changes":[{{
            "field":"messages","value":{{
                "messaging_product":"whatsapp",
                "metadata":{{"display_phone_number":"15550001234","phone_number_id":"2"}},
                "messages":[{{"from":"4915112345678","id":"wamid.1","timestamp":"{timestamp}","type":"text","text":{{"body":"Hallo"}}}}]
            }}}}]}}]}}"#
    )
    .into_bytes()
}

// ---------------------------------------------------------------------------
// Signature
// ---------------------------------------------------------------------------

#[test]
fn sign_then_verify_round_trips() {
    let verifier = verifier();
    let header = verifier.sign(BODY);
    assert!(header.starts_with("sha256="));
    assert_eq!(header.len(), "sha256=".len() + 64);
    assert!(verifier.verify(BODY, &header).is_ok());
}

#[test]
fn wrong_secret_is_rejected() {
    let signer = WebhookVerifier::new("secret").unwrap();
    let verifier = WebhookVerifier::new("other-secret").unwrap();
    let header = signer.sign(BODY);
    assert!(matches!(
        verifier.verify(BODY, &header),
        Err(Error::SignatureMismatch)
    ));
}

#[test]
fn tampered_body_is_rejected() {
    let verifier = verifier();
    let header = verifier.sign(BODY);
    let mut tampered = BODY.to_vec();
    tampered[1] = b'X';
    assert!(matches!(
        verifier.verify(&tampered, &header),
        Err(Error::SignatureMismatch)
    ));
}

#[test]
fn every_single_byte_flip_is_rejected() {
    let verifier = verifier();
    let header = verifier.sign(BODY);
    for index in 0..BODY.len() {
        let mut tampered = BODY.to_vec();
        tampered[index] ^= 0x01;
        assert!(
            verifier.verify(&tampered, &header).is_err(),
            "flip at {index} was accepted"
        );
    }
}

#[test]
fn malformed_headers_are_rejected() {
    let verifier = verifier();
    let good = verifier.sign(BODY);
    let uppercase = good.to_uppercase();
    let short = &good[..good.len() - 2];
    let long = format!("{good}00");
    for header in [
        "",
        "sha1=abc",
        "sha256=",
        "sha256=zz",
        "sha256=00",
        uppercase.as_str(),
        short,
        long.as_str(),
        &format!(" {good}"),
        &format!("{good} "),
        "sha256=+00",
    ] {
        assert!(
            matches!(
                verifier.verify(BODY, header),
                Err(Error::MalformedSignature(_))
            ),
            "header {header:?} should be rejected"
        );
    }
}

#[test]
fn empty_secret_is_rejected() {
    // HMAC with an empty key is well defined - accepting it would turn a
    // missing META_APP_SECRET into a total auth bypass.
    assert!(matches!(
        WebhookVerifier::new(Vec::new()),
        Err(Error::EmptySecret)
    ));
    assert!(matches!(WebhookVerifier::new(""), Err(Error::EmptySecret)));
}

#[test]
fn verify_and_parse_returns_typed_payload() {
    let verifier = verifier();
    let header = verifier.sign(BODY);
    let payload = verifier.verify_and_parse(BODY, &header).unwrap();
    assert_eq!(payload.object, "whatsapp_business_account");
    assert_eq!(payload.entry, Vec::new());
}

#[test]
fn verify_and_parse_rejects_non_json_and_truncated_bodies() {
    let verifier = verifier();
    for body in [
        b"not json".to_vec(),
        // Truncated mid-object.
        br#"{"object":"whatsapp_business_account","entry":[{"id":"1""#.to_vec(),
        // NUL byte.
        b"\x00".to_vec(),
        // Unterminated string.
        br#"{"object":"x"#.to_vec(),
    ] {
        let header = verifier.sign(&body);
        let err = verifier.verify_and_parse(&body, &header).unwrap_err();
        assert!(
            matches!(err, Error::Json { .. }),
            "body {body:?} gave {err:?}"
        );
    }
}

#[test]
fn a_json_array_deserialises_as_defaulted_struct() {
    // serde accepts a sequence where a struct is expected; every field has a
    // default, so this degrades to an empty payload instead of failing.
    let verifier = verifier();
    let body = b"[]";
    let payload = verifier
        .verify_and_parse(body, &verifier.sign(body))
        .unwrap();
    assert_eq!(payload.entry.len(), 0);
    assert_eq!(payload.object, "");
}

#[test]
fn signature_errors_win_over_json_errors() {
    // Fail closed: a malformed body with a bad signature is a signature error,
    // so callers cannot use parse errors as an oracle.
    let verifier = verifier();
    let err = verifier
        .verify_and_parse(b"not json", "sha256=00")
        .unwrap_err();
    assert!(matches!(err, Error::MalformedSignature(_)));
}

#[test]
fn duplicate_keys_are_rejected_rather_than_guessed() {
    // serde refuses duplicated *known* fields ("duplicate field `object`")
    // instead of silently keeping one — ambiguous input fails closed with a
    // typed error, and never panics.
    let verifier = verifier();
    let body = br#"{"object":"first","object":"whatsapp_business_account","entry":[]}"#;
    let err = verifier
        .verify_and_parse(body, &verifier.sign(body))
        .unwrap_err();
    assert!(matches!(err, Error::Json { .. }), "got {err:?}");

    // Unknown keys may repeat freely; they are only carried around.
    let body = br#"{"object":"whatsapp_business_account","entry":[],"x":1,"x":2}"#;
    assert!(verifier
        .verify_and_parse(body, &verifier.sign(body))
        .is_ok());
}

#[test]
fn unknown_fields_at_every_level_are_ignored() {
    let body = br#"{
        "object":"whatsapp_business_account",
        "future_top_level": {"a": 1},
        "entry":[{
            "id":"1",
            "future_entry":"x",
            "changes":[{
                "field":"messages",
                "future_change": 7,
                "value":{
                    "messaging_product":"whatsapp",
                    "metadata":{"display_phone_number":"1","phone_number_id":"2","future":"y"},
                    "messages":[{
                        "id":"wamid.1","from":"3","type":"text","timestamp":"10",
                        "text":{"body":"hi"},
                        "future_message":{"deep": true}
                    }],
                    "future_value":[1,2,3]
                }
            }]
        }]
    }"#;
    let verifier = verifier();
    let payload = verifier
        .verify_and_parse(body, &verifier.sign(body))
        .unwrap();
    let (phone_number_id, message) = payload.inbound_messages().next().unwrap();
    assert_eq!(phone_number_id, "2");
    assert_eq!(message.text_body(), Some("hi"));
    assert_eq!(message.extra["future_message"]["deep"], true);
}

#[test]
fn body_size_limit_is_enforced() {
    let verifier = verifier().with_max_body_bytes(64);
    let body = message_payload("1000");
    assert!(body.len() > 64);
    let err = verifier
        .verify_and_parse(&body, &verifier.sign(&body))
        .unwrap_err();
    assert!(matches!(err, Error::Payload(_)));
    assert_eq!(verifier.max_body_bytes(), 64);
}

// ---------------------------------------------------------------------------
// Both `changes[].value` shapes
// ---------------------------------------------------------------------------

#[test]
fn message_and_account_events_parse_from_one_payload() {
    let body = br#"{
        "object":"whatsapp_business_account",
        "entry":[{
            "id":"102290129340398",
            "changes":[
                {"field":"messages","value":{
                    "messaging_product":"whatsapp",
                    "metadata":{"display_phone_number":"15550001234","phone_number_id":"106540352242922"},
                    "messages":[{"id":"wamid.1","from":"4915","type":"text","text":{"body":"Hi"}}],
                    "statuses":[{"id":"wamid.0","status":"read","timestamp":"1729600000"}]
                }},
                {"field":"message_template_status_update","value":{
                    "event":"APPROVED",
                    "message_template_id":594425479261596,
                    "message_template_name":"order_update",
                    "message_template_language":"de",
                    "reason":"NONE"
                }},
                {"field":"phone_number_quality_update","value":{
                    "display_phone_number":"15550001234",
                    "event":"FLAGGED",
                    "current_limit":"TIER_1K"
                }}
            ]
        }]
    }"#;
    let verifier = verifier();
    let payload = verifier
        .verify_and_parse(body, &verifier.sign(body))
        .unwrap();

    assert_eq!(payload.inbound_messages().count(), 1);
    assert_eq!(payload.statuses().count(), 1);
    assert_eq!(payload.changes().count(), 3);

    let template_event = payload
        .changes()
        .find(|change| change.field == "message_template_status_update")
        .expect("template change");
    assert_eq!(template_event.value.event(), Some("APPROVED"));
    assert_eq!(
        template_event.value.extra_str("message_template_name"),
        Some("order_update")
    );

    let quality_event = payload
        .changes()
        .find(|change| change.field == "phone_number_quality_update")
        .expect("quality change");
    assert_eq!(quality_event.value.event(), Some("FLAGGED"));
    assert_eq!(quality_event.value.messages.len(), 0);
}

#[test]
fn unknown_event_types_are_passed_through() {
    let body = br#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[
        {"field":"flows_event","value":{"event":"FLOW_STATUS_CHANGED","future":42}}
    ]}]}"#;
    let payload: WebhookPayload = serde_json::from_slice(body).unwrap();
    let change = payload.changes().next().unwrap();
    assert_eq!(change.field, "flows_event");
    assert_eq!(change.value.extra["future"], 42);
    assert!(change.value.is_empty());
}

#[test]
fn non_object_values_do_not_break_the_batch() {
    let body = br#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[
        {"field":"messages","value":"unexpected"},
        {"field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"id":"wamid.1","from":"3","type":"text","text":{"body":"still parsed"}}]
        }}
    ]}]}"#;
    let payload: WebhookPayload = serde_json::from_slice(body).unwrap();
    assert_eq!(payload.inbound_messages().count(), 1);
}

#[test]
fn one_broken_message_does_not_discard_the_rest() {
    let body = br#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[
                {"id": 5, "from":"3","type":"text"},
                {"id":"wamid.2","from":"4","type":"text","text":{"body":"ok"}}
            ]
        }
    }]}]}"#;
    let payload: WebhookPayload = serde_json::from_slice(body).unwrap();
    let ids: Vec<_> = payload
        .inbound_messages()
        .map(|(_, message)| message.id.as_str())
        .collect();
    assert_eq!(ids, vec!["wamid.2"]);
}

// ---------------------------------------------------------------------------
// Replay window
// ---------------------------------------------------------------------------

#[test]
fn fresh_payloads_pass_the_replay_check() {
    let verifier = verifier().with_tolerance(Duration::from_secs(300));
    let body = message_payload("1729600000");
    let header = verifier.sign(&body);
    let payload = verifier
        .verify_fresh(&body, &header, 1_729_600_000)
        .unwrap();
    assert_eq!(payload.inbound_messages().count(), 1);
}

#[test]
fn old_payloads_are_rejected_as_replays() {
    let verifier = verifier().with_tolerance(Duration::from_secs(300));
    let body = message_payload("1729600000");
    let header = verifier.sign(&body);
    let err = verifier
        .verify_fresh(&body, &header, 1_729_600_301)
        .unwrap_err();
    match err {
        Error::Replay {
            timestamp,
            skew_secs,
            tolerance_secs,
        } => {
            assert_eq!(timestamp, 1_729_600_000);
            assert_eq!(skew_secs, 301);
            assert_eq!(tolerance_secs, 300);
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn future_payloads_are_rejected_too() {
    let verifier = verifier().with_tolerance(Duration::from_secs(60));
    let body = message_payload("2000");
    let header = verifier.sign(&body);
    assert!(verifier.verify_fresh(&body, &header, 1_000).is_err());
    assert!(verifier.verify_fresh(&body, &header, 1_980).is_ok());
}

#[test]
fn the_window_is_inclusive_at_the_boundary() {
    let verifier = verifier().with_tolerance(Duration::from_secs(60));
    let body = message_payload("1000");
    let header = verifier.sign(&body);
    assert!(verifier.verify_fresh(&body, &header, 1_060).is_ok());
    assert!(verifier.verify_fresh(&body, &header, 940).is_ok());
    assert!(verifier.verify_fresh(&body, &header, 1_061).is_err());
    assert!(verifier.verify_fresh(&body, &header, 939).is_err());
}

#[test]
fn zero_tolerance_disables_the_window_but_not_the_signature() {
    let verifier = verifier().with_tolerance(Duration::ZERO);
    let body = message_payload("1000");
    let header = verifier.sign(&body);
    assert!(verifier.verify_fresh(&body, &header, 0).is_ok());
    assert!(verifier.verify_fresh(&body, "sha256=00", 0).is_err());
}

#[test]
fn non_numeric_timestamps_fail_closed() {
    let verifier = verifier();
    let body = message_payload("not-a-number");
    let header = verifier.sign(&body);
    let err = verifier.verify_fresh(&body, &header, 1_000).unwrap_err();
    assert!(matches!(err, Error::Payload(_)));
}

#[test]
fn every_timestamp_in_the_batch_must_be_fresh() {
    use meta_whatsapp::types::{Change, Entry, Message, Metadata, Status};

    let payload = WebhookPayload {
        object: "whatsapp_business_account".into(),
        entry: vec![Entry {
            id: "1".into(),
            changes: vec![Change {
                field: "messages".into(),
                value: meta_whatsapp::ChangeValue {
                    metadata: Some(Metadata {
                        display_phone_number: "1".into(),
                        phone_number_id: "2".into(),
                    }),
                    messages: vec![Message {
                        id: "m".into(),
                        from: "3".into(),
                        timestamp: "500".into(),
                        message_type: "text".into(),
                        ..Message::default()
                    }],
                    statuses: vec![Status {
                        id: "m2".into(),
                        status: "read".into(),
                        timestamp: "505".into(),
                        ..Status::default()
                    }],
                    ..meta_whatsapp::ChangeValue::default()
                },
            }],
        }],
    };
    assert_eq!(payload.newest_timestamp(), Some(505));

    let verifier = verifier().with_tolerance(Duration::from_secs(10));
    assert!(verifier.check_replay(&payload, 510).is_ok());

    // A single stale entry is enough to reject the batch: an old message must
    // not ride along inside a fresh-looking envelope.
    match verifier.check_replay(&payload, 515).unwrap_err() {
        Error::Replay {
            timestamp,
            skew_secs,
            ..
        } => {
            assert_eq!(timestamp, 500);
            assert_eq!(skew_secs, 15);
        }
        other => panic!("unexpected {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// GET handshake
// ---------------------------------------------------------------------------

#[test]
fn challenge_handshake_accepts_matching_token() {
    let verifier = verifier();
    assert_eq!(
        verifier.verify_challenge(
            Some("subscribe"),
            Some("tok"),
            Some("1234"),
            "tok",
            "subscribe"
        ),
        Some("1234")
    );
}

#[test]
fn challenge_handshake_rejects_mismatches_and_missing_parts() {
    let verifier = verifier();
    for (mode, token, challenge) in [
        (Some("subscribe"), Some("wrong"), Some("1234")),
        (Some("unsubscribe"), Some("tok"), Some("1234")),
        (None, Some("tok"), Some("1234")),
        (Some("subscribe"), None, Some("1234")),
        (Some("subscribe"), Some("tok"), None),
    ] {
        assert_eq!(
            verifier.verify_challenge(mode, token, challenge, "tok", "subscribe"),
            None,
            "mode={mode:?} token={token:?}"
        );
    }
}

#[test]
fn debug_impl_does_not_leak_the_secret() {
    let verifier = WebhookVerifier::new("super-secret-value").unwrap();
    let rendered = format!("{verifier:?}");
    assert!(!rendered.contains("super-secret-value"));
    assert!(rendered.contains("tolerance"));
}

#[test]
fn verifier_is_clone_and_send_sync() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    let verifier = verifier();
    assert_send_sync(&verifier);
    let cloned = verifier.clone();
    assert!(cloned.verify(BODY, &verifier.sign(BODY)).is_ok());
}
