//! Property tests: the invariants that must hold for *all* inputs, not just the
//! fixtures above.

use std::time::Duration;

use meta_whatsapp::business::{
    block_users_payload, validate_block_user, validate_field_list, BusinessProfile,
    BusinessProfileUpdate, MessagingLimitTier, QualityRating, BLOCK_USER_MAX_DIGITS,
    BLOCK_USER_MIN_DIGITS,
};
use meta_whatsapp::media::{validate_media_id, MAX_MEDIA_ID_LEN};
use meta_whatsapp::retry::{Action, Attempt, Repeatability};
use meta_whatsapp::subscription::{validate_waba_id, WebhookSubscriptions};
use meta_whatsapp::transport::Method;
use meta_whatsapp::usage::BusinessUseCaseUsage;
use meta_whatsapp::{
    Error, MessageRequest, RetryPolicy, TemplateQuery, TextMessage, WebhookPayload, WebhookVerifier,
};
use percent_encoding::percent_decode_str;
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

/// An API error carrying `status`, as `api_error` would build it.
fn api_error(status: u16, retry_after: Option<Duration>) -> Error {
    Error::Api {
        code: 0,
        status,
        message: String::new(),
        retry_after,
        retryable: false,
    }
}

/// Any attempt the retry classifier can produce from an error, plus the ones
/// the transport builds directly.
fn arb_attempt() -> impl Strategy<Value = Attempt> {
    prop_oneof![
        proptest::option::of(0_u64..120).prop_map(|secs| Attempt::ServerError {
            retry_after: secs.map(Duration::from_secs)
        }),
        proptest::option::of(0_u64..120).prop_map(|secs| Attempt::RateLimited {
            retry_after: secs.map(Duration::from_secs)
        }),
        Just(Attempt::NotSent),
        Just(Attempt::NoAnswer),
        Just(Attempt::Terminal),
    ]
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

    /// A `Retry-After` within the ceiling is honoured exactly; one above it is
    /// surfaced rather than shortened into an early retry. A spent budget still
    /// wins in both cases.
    #[test]
    fn retry_after_is_honoured_only_within_the_ceiling(
        hint_ms in 0_u64..10_000,
        max_ms in 0_u64..5_000,
        attempts_failed in 0_u32..12,
        max_attempts in 1_u32..12,
    ) {
        let policy = RetryPolicy::new(max_attempts)
            .with_max_delay(Duration::from_millis(max_ms));
        let hint = Duration::from_millis(hint_ms);
        let action = policy.decide(
            Attempt::RateLimited {
                retry_after: Some(hint),
            },
            Repeatability::NotIdempotent,
            attempts_failed,
        );
        let expected = if attempts_failed < max_attempts && hint <= policy.max_delay {
            Action::Retry(hint)
        } else {
            Action::Surface
        };
        prop_assert_eq!(action, expected);
    }

    /// The delivery contract, pinned: whatever the status, whatever the budget,
    /// a non-idempotent request is **never** retried after a 5xx.
    #[test]
    fn no_non_idempotent_request_is_retried_after_a_5xx(
        status in 500_u16..600,
        attempts_failed in 0_u32..12,
        max_attempts in 0_u32..12,
        retry_after_secs in proptest::option::of(0_u64..600),
    ) {
        let policy = RetryPolicy::new(max_attempts);
        let error = api_error(status, retry_after_secs.map(Duration::from_secs));
        let attempt = Attempt::classify(&error);
        let action = policy.decide(attempt, Repeatability::NotIdempotent, attempts_failed);

        prop_assert!(
            !matches!(action, Action::Retry(_)),
            "retried a 5xx on a non-idempotent request (status {status}, attempt {attempts_failed}, budget {max_attempts})"
        );
        prop_assert_eq!(action, Action::Ambiguous);
    }

    /// The same rule for a request that was written but never answered.
    #[test]
    fn a_lost_answer_never_retries_a_non_idempotent_request(
        attempts_failed in 0_u32..12,
        max_attempts in 0_u32..12,
    ) {
        let policy = RetryPolicy::new(max_attempts);
        let action = policy.decide(Attempt::NoAnswer, Repeatability::NotIdempotent, attempts_failed);
        prop_assert_eq!(action, Action::Ambiguous);
    }

    /// Whatever the attempt, a retry only happens inside the budget and never
    /// waits longer than the ceiling; terminal attempts never retry.
    #[test]
    fn every_retry_decision_respects_budget_and_ceiling(
        attempt in arb_attempt(),
        attempts_failed in 0_u32..10,
        max_attempts in 0_u32..10,
        base_ms in 0_u64..500,
        max_ms in 0_u64..5_000,
        repeatability in prop_oneof![
            Just(Repeatability::Idempotent),
            Just(Repeatability::NotIdempotent)
        ],
    ) {
        let policy = RetryPolicy::new(max_attempts)
            .with_base_delay(Duration::from_millis(base_ms))
            .with_max_delay(Duration::from_millis(max_ms));
        let action = policy.decide(attempt, repeatability, attempts_failed);

        if let Action::Retry(delay) = action {
            prop_assert!(delay <= policy.max_delay);
            prop_assert!(attempts_failed < max_attempts.max(1));
            prop_assert!(attempt != Attempt::Terminal);
        }

        if attempt == Attempt::Terminal {
            prop_assert_eq!(action, Action::Surface);
        }
    }

    /// Classification is total and only depends on the status and phase.
    #[test]
    fn attempt_classification_is_total(status in any::<u16>(), after_write in any::<bool>()) {
        let api = Attempt::classify(&api_error(status, None));
        let expected = match status {
            429 => Attempt::RateLimited { retry_after: None },
            500..=599 => Attempt::ServerError { retry_after: None },
            _ => Attempt::Terminal,
        };
        prop_assert_eq!(api, expected);

        let transport_error = if after_write {
            Error::transport_after_write("x")
        } else {
            Error::transport("x")
        };
        prop_assert_eq!(
            Attempt::classify(&transport_error),
            if after_write { Attempt::NoAnswer } else { Attempt::NotSent }
        );

        // Everything that is not an API or transport failure is terminal.
        prop_assert_eq!(
            Attempt::classify(&Error::validation("f", "m")),
            Attempt::Terminal
        );
    }

    /// `GET`/`DELETE` are idempotent, `POST` never is.
    #[test]
    fn repeatability_follows_the_verb(_unused in 0_u8..1) {
        prop_assert!(Repeatability::of_method(Method::Get).is_idempotent());
        prop_assert!(Repeatability::of_method(Method::Delete).is_idempotent());
        prop_assert!(!Repeatability::of_method(Method::Post).is_idempotent());
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

    /// Every query value survives the round trip through the wire form, so a
    /// cursor the provider handed out comes back byte for byte.
    #[test]
    fn template_query_values_decode_back_to_the_input(
        cursor in ".{0,64}",
        name in ".{0,64}",
        language in ".{0,16}",
    ) {
        let rendered = TemplateQuery::new("1")
            .after(&cursor)
            .name(&name)
            .language(&language)
            .query_string();
        let value_of = |key: &str| {
            rendered
                .split('&')
                .find_map(|part| part.strip_prefix(key))
                .and_then(|rest| rest.strip_prefix('='))
                .map(str::to_owned)
        };
        for (key, expected) in [("after", &cursor), ("name", &name), ("language", &language)] {
            let raw = value_of(key).unwrap();
            let decoded = percent_decode_str(&raw).decode_utf8().unwrap();
            prop_assert_eq!(decoded.as_ref(), expected.as_str());
        }
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

    /// Quality ratings round-trip through their wire spelling, and an unknown
    /// spelling is preserved rather than dropped.
    #[test]
    fn quality_ratings_round_trip(raw in ".{0,40}") {
        let rating = QualityRating::parse(&raw);
        let reparsed = QualityRating::parse(rating.as_str());
        prop_assert_eq!(&rating, &reparsed);
        prop_assert_eq!(
            serde_json::to_string(&rating).unwrap(),
            serde_json::to_string(&reparsed).unwrap()
        );
        if !matches!(rating, QualityRating::Unknown(_)) {
            prop_assert!(matches!(
                rating.as_str(),
                "GREEN" | "YELLOW" | "RED" | "NA"
            ));
        }
    }

    /// Messaging tiers round-trip, and the bounded ones increase with the tier.
    #[test]
    fn messaging_tiers_round_trip_and_increase(raw in ".{0,40}") {
        let tier = MessagingLimitTier::parse(&raw);
        prop_assert_eq!(MessagingLimitTier::parse(tier.as_str()), tier.clone());

        let ladder = [
            MessagingLimitTier::Tier50,
            MessagingLimitTier::Tier250,
            MessagingLimitTier::Tier1K,
            MessagingLimitTier::Tier10K,
            MessagingLimitTier::Tier100K,
        ];
        let limits: Vec<u64> = ladder
            .iter()
            .filter_map(MessagingLimitTier::unique_contacts_per_24h)
            .collect();
        prop_assert_eq!(limits.len(), ladder.len());
        prop_assert!(limits.windows(2).all(|pair| pair[1] > pair[0]));
        prop_assert_eq!(MessagingLimitTier::Unlimited.unique_contacts_per_24h(), None);
        prop_assert!(MessagingLimitTier::Unlimited.is_unlimited());
    }

    /// A blocklist number is accepted exactly when it is 7–15 digits with an
    /// optional leading `+`; an accepted list always renders.
    #[test]
    fn block_user_acceptance_matches_the_arithmetic(raw in ".{0,40}") {
        let digits = raw.strip_prefix('+').unwrap_or(&raw);
        let expected = (BLOCK_USER_MIN_DIGITS..=BLOCK_USER_MAX_DIGITS).contains(&digits.len())
            && digits.bytes().all(|byte| byte.is_ascii_digit());
        prop_assert_eq!(validate_block_user(&raw).is_ok(), expected);

        if expected {
            let payload = block_users_payload(&[raw.as_str()]).unwrap();
            prop_assert_eq!(&payload["block_users"][0]["user"], &Value::String(raw));
        }
    }

    /// Field lists accept only lowercase identifiers, so nothing can smuggle a
    /// query separator into the URL.
    #[test]
    fn field_lists_accept_only_the_safe_alphabet(fields in ".{0,80}") {
        let accepted = validate_field_list(&fields).is_ok();
        let manual = !fields.is_empty()
            && !fields.starts_with(',')
            && !fields.ends_with(',')
            && !fields.contains(",,")
            && fields.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b','
            });
        prop_assert_eq!(accepted, manual);
    }

    /// WABA ids are digits only, so they cannot escape their URL segment.
    #[test]
    fn waba_ids_are_numeric(waba in ".{0,40}") {
        let accepted = validate_waba_id(&waba).is_ok();
        let expected = !waba.is_empty()
            && waba.len() <= 32
            && waba.bytes().all(|byte| byte.is_ascii_digit());
        prop_assert_eq!(accepted, expected);
    }

    /// The usage header is untrusted input: parsing is total, and anything we
    /// accept re-serialises into something we accept again.
    #[test]
    fn usage_header_parsing_is_total(header in ".{0,200}") {
        if let Some(usage) = BusinessUseCaseUsage::from_header(&header) {
            let rendered = serde_json::to_string(&usage).unwrap();
            let reparsed = BusinessUseCaseUsage::from_header(&rendered).unwrap();
            prop_assert_eq!(usage, reparsed);
        }
        // A header that is not an object is always rejected.
        if !header.trim_start().starts_with('{') {
            prop_assert!(BusinessUseCaseUsage::from_header(&header).is_none());
        }
    }

    /// Profile parsing is total over arbitrary JSON.
    #[test]
    fn business_profile_parsing_is_total(value in arb_json()) {
        if let Ok(profile) = BusinessProfile::parse(&value) {
            let rendered = serde_json::to_value(&profile).unwrap();
            prop_assert!(rendered.is_object());
        }
    }

    /// Subscription parsing is total over arbitrary JSON.
    #[test]
    fn subscription_parsing_is_total(value in arb_json()) {
        if let Ok(subs) = WebhookSubscriptions::parse(&value) {
            let rendered = serde_json::to_value(&subs).unwrap();
            prop_assert!(rendered.is_object());
        }
    }

    /// Whatever survives validation renders with `messaging_product` and never
    /// with an unset field.
    #[test]
    fn profile_updates_render_only_what_validated(
        about in ".{0,150}",
        email in ".{0,140}",
        website in ".{0,140}",
    ) {
        let update = BusinessProfileUpdate::new()
            .about(&about)
            .email(&email)
            .website(&website);
        if let Ok(payload) = update.to_json() {
            prop_assert_eq!(&payload["messaging_product"], &Value::String("whatsapp".into()));
            prop_assert_eq!(&payload["about"], &Value::String(about.clone()));
            prop_assert!(payload.get("address").is_none());
            prop_assert!(payload["websites"][0].is_string());
        }
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

#[test]
fn the_retry_rule_table_is_pinned_for_every_status_class() {
    // Deterministic companion to the property: one representative per class.
    let policy = RetryPolicy::new(3);
    let cases = [
        (429_u16, true, true),
        (400, false, false),
        (401, false, false),
        (404, false, false),
        (500, false, true),
        (503, false, true),
        (599, false, true),
    ];
    for (status, send_retried, read_retried) in cases {
        let send = policy.decide(
            Attempt::classify(&api_error(status, None)),
            Repeatability::NotIdempotent,
            1,
        );
        let read = policy.decide(
            Attempt::classify(&api_error(status, None)),
            Repeatability::Idempotent,
            1,
        );
        assert_eq!(
            matches!(send, Action::Retry(_)),
            send_retried,
            "send {status}: {send:?}"
        );
        assert_eq!(
            matches!(read, Action::Retry(_)),
            read_retried,
            "read {status}: {read:?}"
        );
        if (500..600).contains(&status) {
            assert_eq!(send, Action::Ambiguous);
        }
    }
}
