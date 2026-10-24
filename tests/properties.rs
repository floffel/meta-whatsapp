//! Property tests: the invariants that must hold for *all* inputs, not just the
//! fixtures above.

use std::time::Duration;

use meta_whatsapp::media::{validate_media_id, MAX_MEDIA_ID_LEN};
use meta_whatsapp::{
    MessageRequest, RetryPolicy, TemplateQuery, TextMessage, WebhookPayload, WebhookVerifier,
};
use proptest::prelude::*;
use serde_json::{Map, Value};

/// Arbitrary JSON values, deep enough to hit nesting but bounded.
fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<f64>().prop_filter_map("finite", |value| {
            serde_json::Number::from_f64(value).map(Value::Number)
        }),
        ".{0,32}".prop_map(Value::String),
    ];

    leaf.prop_recursive(4, 64, 8, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec((".{0,16}", inner), 0..4).prop_map(|pairs| {
                Value::Object(pairs.into_iter().collect::<Map<String, Value>>())
            }),
        ]
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// Serde must never panic, whatever arrives. It may accept or reject.
    #[test]
    fn webhook_parsing_total(value in arb_json()) {
        let rendered = value.to_string();
        let parsed = serde_json::from_str::<WebhookPayload>(&rendered);
        if let Ok(payload) = parsed {
            // A payload we accepted must re-serialise into an equal payload.
            let round_tripped = serde_json::to_string(&payload).unwrap();
            let reparsed: WebhookPayload = serde_json::from_str(&round_tripped).unwrap();
            prop_assert_eq!(payload, reparsed);
        }
    }

    /// No byte string can make the parser panic, and signing is total.
    #[test]
    fn arbitrary_bytes_are_signed_and_parsed_without_panicking(body in prop::collection::vec(any::<u8>(), 0..256)) {
        let verifier = WebhookVerifier::new("secret").unwrap();
        let header = verifier.sign(&body);
        prop_assert!(verifier.verify(&body, &header).is_ok());
        let _ = verifier.verify_and_parse(&body, &header);
    }

    /// Sign/verify round-trips for every secret and body; a different secret
    /// never verifies.
    #[test]
    fn signatures_are_exact(
        secret in proptest::collection::vec(any::<u8>(), 1..64),
        other in proptest::collection::vec(any::<u8>(), 1..64),
        body in prop::collection::vec(any::<u8>(), 0..128),
    ) {
        let signer = WebhookVerifier::new(secret.clone()).unwrap();
        let header = signer.sign(&body);
        prop_assert!(signer.verify(&body, &header).is_ok());

        if secret != other {
            let verifier = WebhookVerifier::new(other).unwrap();
            prop_assert!(verifier.verify(&body, &header).is_err());
        }
    }

    /// Flattening the text rule: accepted exactly for 1..=4096 bytes.
    #[test]
    fn text_acceptance_matches_the_documented_boundary(text in ".{0,4200}") {
        let accepted = MessageRequest::new("4915112345678", TextMessage::new(&text))
            .to_json()
            .is_ok();
        prop_assert_eq!(accepted, !text.is_empty() && text.len() <= 4096);
    }

    /// A media id is accepted only if it survives the charset/length rules, and
    /// an accepted id never contains a path separator.
    #[test]
    fn media_id_acceptance_is_sound(id in ".{0,300}") {
        let accepted = validate_media_id(&id).is_ok();
        let charset_ok = !id.is_empty()
            && id.len() <= MAX_MEDIA_ID_LEN
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        prop_assert_eq!(accepted, charset_ok);
        if accepted {
            prop_assert!(!id.contains('/') && !id.contains('.') && !id.contains('?'));
        }
    }

    /// Backoff never exceeds the ceiling, stops when the budget is spent, and
    /// never shrinks while attempts grow.
    #[test]
    fn retry_delays_are_bounded_and_monotone(
        attempts in 1_u32..10,
        base_ms in 0_u64..500,
        max_ms in 0_u64..5_000,
        failed in 0_u32..12,
    ) {
        let policy = RetryPolicy::new(attempts)
            .with_base_delay(Duration::from_millis(base_ms))
            .with_max_delay(Duration::from_millis(max_ms));

        match policy.delay(failed, None) {
            Some(delay) => {
                prop_assert!(delay <= policy.max_delay);
                let next = policy.delay(failed + 1, None);
                if let Some(next) = next {
                    prop_assert!(next >= delay);
                }
            }
            None => prop_assert!(failed >= attempts || attempts == 0),
        }

        if failed >= attempts.max(1) {
            prop_assert!(policy.delay(failed, None).is_none());
        }
    }

    /// Query strings stay within a conservative URL-safe alphabet.
    #[test]
    fn template_queries_are_percent_encoded(
        cursor in ".{0,64}",
        name in ".{0,64}",
        language in ".{0,16}",
        limit in 1_u32..100,
    ) {
        let query = TemplateQuery::new("102290129340398")
            .limit(limit)
            .after(&cursor)
            .name(&name)
            .language(&language);
        let rendered = query.query_string();
        prop_assert!(rendered
            .chars()
            .all(|c| c.is_ascii_alphanumeric()
                || matches!(c, '%' | '&' | '=' | '-' | '_' | '.' | '~')));
        let limit_part = format!("limit={limit}");
        prop_assert!(rendered.contains(&limit_part));
    }

    /// Rendering is stable: JSON in, same JSON out (no field loss).
    #[test]
    fn rendered_messages_round_trip(body in ".{1,64}") {
        let request = MessageRequest::new("4915112345678", TextMessage::new(&body));
        let value = request.to_json().unwrap();
        let rendered = serde_json::to_string(&value).unwrap();
        let reparsed: Value = serde_json::from_str(&rendered).unwrap();
        prop_assert_eq!(&reparsed, &value);
        prop_assert_eq!(&reparsed["text"]["body"], &Value::String(body));
    }

    /// Replay checking is a pure function of timestamp distance: accepted iff
    /// every timestamp is within the window.
    #[test]
    fn replay_window_matches_the_arithmetic(
        payload_ts in 0_u64..10_000,
        now in 0_u64..10_000,
        tolerance in 0_u64..600,
    ) {
        let body = format!(
            r#"{{"object":"whatsapp_business_account","entry":[{{"id":"1","changes":[{{"field":"messages","value":{{"metadata":{{"display_phone_number":"1","phone_number_id":"2"}},"messages":[{{"id":"m","from":"3","type":"text","timestamp":"{payload_ts}"}}]}}}}]}}]}}"#
        );
        let payload: WebhookPayload = serde_json::from_str(&body).unwrap();
        let verifier = WebhookVerifier::new("secret")
            .unwrap()
            .with_tolerance(Duration::from_secs(tolerance));
        let accepted = verifier.check_replay(&payload, now).is_ok();
        let expected = tolerance == 0 || payload_ts.abs_diff(now) <= tolerance;
        prop_assert_eq!(accepted, expected);
    }
}

#[test]
fn message_rendering_is_stable_for_all_ascii_punctuation() {
    // A deterministic sweep next to the property: every printable ASCII code
    // point survives a render/parse cycle.
    for byte in 0x20_u8..=0x7e {
        let text = String::from_utf8(vec![byte]).unwrap();
        let value = MessageRequest::new("1", TextMessage::new(&text))
            .to_json()
            .unwrap();
        assert_eq!(value["text"]["body"], Value::String(text));
    }
}

#[test]
fn retry_policy_handles_extreme_attempt_counts_without_overflow() {
    let policy = RetryPolicy::new(u32::MAX).with_base_delay(Duration::from_secs(1));
    for attempts in [1, 2, 31, 32, 1_000, u32::MAX - 1] {
        if let Some(delay) = policy.delay(attempts, None) {
            assert!(delay <= policy.max_delay);
        }
    }
}

#[test]
fn duplicate_json_keys_never_panic_and_fail_closed() {
    // Duplicate known fields are ambiguous, so serde reports an error instead
    // of picking a winner; the parser must surface that as a typed error.
    for body in [
        r#"{"object":"a","object":"b","entry":[]}"#,
        r#"{"entry":[],"entry":[],"object":"x"}"#,
        r#"{"entry":[{"id":"1","id":"2","changes":[]}]}"#,
    ] {
        let parsed: Result<WebhookPayload, _> = serde_json::from_str(body);
        let error = parsed.expect_err("duplicate field must be rejected");
        assert!(error.is_data(), "{error}");
    }

    // A single occurrence is fine.
    let payload: WebhookPayload = serde_json::from_str(r#"{"object":"x","entry":[]}"#).unwrap();
    assert_eq!(payload.object, "x");
}
