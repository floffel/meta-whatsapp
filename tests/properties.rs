//! Property tests: the invariants that must hold for *all* inputs, not just the
//! fixtures above.

use std::time::Duration;

use meta_whatsapp::business::{
    block_users_payload, validate_block_user, validate_field_list, validate_verification_pin,
    BlockUsersQuery, BusinessProfile, BusinessProfileUpdate, MessagingLimitTier, QualityRating,
    SuccessOutcome, VerificationPin, BLOCK_USER_MAX_DIGITS, BLOCK_USER_MIN_DIGITS,
    VERIFICATION_PIN_LEN,
};
use meta_whatsapp::media::{validate_media_id, MAX_MEDIA_ID_LEN};
use meta_whatsapp::message::{CAPTION_MAX_BYTES, HEADER_MAX_CHARS};
use meta_whatsapp::retry::{Action, Attempt, Repeatability};
use meta_whatsapp::subscription::{validate_waba_id, WebhookSubscriptions};
use meta_whatsapp::template::{
    TemplateDefinitionComponent, TEMPLATE_HEADER_MAX_CHARS, TEMPLATE_NAME_MAX_BYTES,
};
use meta_whatsapp::transport::Method;
use meta_whatsapp::usage::BusinessUseCaseUsage;
use meta_whatsapp::{
    Error, MessageRequest, MultiProductMessage, ProductSection, RetryPolicy, SingleProductMessage,
    TemplateCategory, TemplateDefinition, TemplateQuery, TemplateUpdate, TextMessage,
    WebhookPayload, WebhookVerifier,
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

    /// A multi-product payload is accepted exactly inside the documented
    /// envelope: 1..=10 sections, 1..=30 products each, 30 at most in total.
    #[test]
    fn multi_product_acceptance_matches_the_documented_envelope(
        sections in prop::collection::vec(0_usize..33, 0..13),
    ) {
        let mut message = MultiProductMessage::new("catalog", "Katalog", "Bitte wählen");
        for (index, count) in sections.iter().enumerate() {
            let ids = (0..*count).map(|product| format!("sku-{index}-{product}"));
            message = message.section(ProductSection::from_ids(ids).title(format!("S{index}")));
        }

        let total: usize = sections.iter().sum();
        let expected = (1..=10).contains(&sections.len())
            && sections.iter().all(|count| (1..=30).contains(count))
            && total <= 30;
        prop_assert_eq!(
            message.validate().is_ok(),
            expected,
            "sections {:?}, total {}",
            sections,
            total
        );
    }

    /// A single-product payload is accepted exactly inside the documented
    /// envelope: both ids present and printable, the optional body 1..=1024
    /// **bytes**, the optional footer at most 60 **characters**. Characters and
    /// bytes differ across the whole input space, so this pins which unit each
    /// limit is counted in.
    #[test]
    fn single_product_acceptance_matches_the_documented_envelope(
        catalog_id in prop::collection::vec(prop::char::any(), 0..8),
        retailer_id in prop::collection::vec(prop::char::any(), 0..8),
        body in prop::option::of(prop::collection::vec(prop::char::any(), 0..1_100)),
        footer in prop::option::of(prop::collection::vec(prop::char::any(), 0..70)),
    ) {
        let catalog_id: String = catalog_id.into_iter().collect();
        let retailer_id: String = retailer_id.into_iter().collect();
        let body: Option<String> = body.map(|chars| chars.into_iter().collect());
        let footer: Option<String> = footer.map(|chars| chars.into_iter().collect());

        let mut message = SingleProductMessage::new(catalog_id.clone(), retailer_id.clone());
        if let Some(text) = &body {
            message = message.body(text.clone());
        }
        if let Some(text) = &footer {
            message = message.footer(text.clone());
        }

        let printable = |value: &str| !value.is_empty() && !value.chars().any(char::is_control);
        let expected = printable(&catalog_id)
            && printable(&retailer_id)
            && body
                .as_ref()
                .is_none_or(|text| !text.is_empty() && text.len() <= CAPTION_MAX_BYTES)
            && footer
                .as_ref()
                .is_none_or(|text| text.chars().count() <= HEADER_MAX_CHARS);

        prop_assert_eq!(
            message.validate().is_ok(),
            expected,
            "ids {:?}/{:?}, body {:?}, footer {:?}",
            catalog_id,
            retailer_id,
            body.as_deref().map(|text| (text.len(), text.chars().count())),
            footer
                .as_deref()
                .map(|text| (text.len(), text.chars().count()))
        );
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

    /// A hint that fits under the ceiling is waited out verbatim: the server's
    /// schedule wins over the curve in both directions.
    #[test]
    fn a_retry_after_within_the_ceiling_is_waited_verbatim(
        attempts in 1_u32..10,
        hint_ms in 0_u64..5_000,
        max_ms in 0_u64..5_000,
        base_ms in 0_u64..500,
        failed in 0_u32..12,
    ) {
        let ceiling = max_ms.max(hint_ms);
        let policy = RetryPolicy::new(attempts)
            .with_base_delay(Duration::from_millis(base_ms))
            .with_max_delay(Duration::from_millis(ceiling));
        let hint = Duration::from_millis(hint_ms);

        if failed < attempts.max(1) {
            prop_assert_eq!(policy.delay(failed, Some(hint)), Some(hint));
        }
    }

    /// A hint above the ceiling never produces a wait — and never a shorter one.
    /// The 429 surfaces with the hint intact; a 5xx on a send keeps its own
    /// rule (`Ambiguous`), hint or not.
    #[test]
    fn a_retry_after_above_the_ceiling_never_waits(
        max_ms in 0_u64..5_000,
        over_ms in 1_u64..86_400_000,
        attempts in 0_u32..10,
        failed in 0_u32..12,
        status in prop_oneof![Just(429_u16), 500_u16..600],
        repeatability in prop_oneof![
            Just(Repeatability::Idempotent),
            Just(Repeatability::NotIdempotent)
        ],
    ) {
        let policy = RetryPolicy::new(attempts).with_max_delay(Duration::from_millis(max_ms));
        let hint = Duration::from_millis(max_ms.saturating_add(over_ms));
        prop_assert!(
            hint > policy.max_delay,
            "the case must actually be over the ceiling"
        );

        let attempt = if status == 429 {
            Attempt::RateLimited {
                retry_after: Some(hint),
            }
        } else {
            Attempt::ServerError {
                retry_after: Some(hint),
            }
        };
        let action = policy.decide(attempt, repeatability, failed);

        prop_assert!(
            !matches!(action, Action::Retry(_)),
            "waited {action:?} for a {hint:?} hint above a {:?} ceiling",
            policy.max_delay
        );
        if status == 429 || repeatability.is_idempotent() {
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

    /// Blocklist cursors survive the wire form, and the rendered query stays
    /// inside a conservative URL alphabet.
    #[test]
    fn blocklist_queries_are_percent_encoded(cursor in ".{1,64}", limit in 1_u32..10_000) {
        let rendered = BlockUsersQuery::new()
            .limit(limit)
            .after(&cursor)
            .query_string();
        prop_assert!(rendered.chars().all(|c| c.is_ascii_alphanumeric()
            || matches!(c, '%' | '&' | '=' | '-' | '_' | '.' | '~')));
        let limit_part = format!("limit={limit}");
        prop_assert!(rendered.contains(&limit_part));
        let encoded = rendered
            .split('&')
            .find_map(|part| part.strip_prefix("after="))
            .unwrap();
        let decoded = percent_decode_str(encoded).decode_utf8().unwrap();
        prop_assert_eq!(decoded.as_ref(), cursor.as_str());
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

    /// The routing iterator is exactly the view it promises: every status of a
    /// change that carries `metadata`, paired with that change's
    /// `phone_number_id` and in order — while `statuses()` still sees all of
    /// them, attributable or not.
    #[test]
    fn status_routing_pairs_every_attributable_status(
        changes in prop::collection::vec(
            (
                prop::option::of("[0-9]{1,20}"),
                prop::collection::vec("[a-z]{1,10}", 0..3),
            ),
            0..6,
        ),
    ) {
        let mut entries = Vec::new();
        let mut expected: Vec<(String, String)> = Vec::new();
        let mut total = 0_usize;
        for (index, (phone_number_id, ids)) in changes.iter().enumerate() {
            let statuses: Vec<Value> = ids
                .iter()
                .map(|id| serde_json::json!({"id": id, "status": "delivered", "timestamp": "10"}))
                .collect();
            total += ids.len();
            let mut value = serde_json::json!({ "statuses": statuses });
            if let Some(phone_number_id) = phone_number_id {
                value["metadata"] = serde_json::json!({
                    "display_phone_number": "1",
                    "phone_number_id": phone_number_id,
                });
                for id in ids {
                    expected.push((phone_number_id.clone(), id.clone()));
                }
            }
            entries.push(serde_json::json!({
                "id": format!("waba-{index}"),
                "changes": [{ "field": "messages", "value": value }],
            }));
        }
        let body = serde_json::json!({
            "object": "whatsapp_business_account",
            "entry": entries,
        });
        let payload: WebhookPayload = serde_json::from_value(body).unwrap();

        let routed: Vec<(String, String)> = payload
            .statuses_with_phone_number_id()
            .map(|(phone_number_id, status)| (phone_number_id.to_string(), status.id.clone()))
            .collect();
        prop_assert_eq!(&routed, &expected);
        prop_assert_eq!(payload.statuses().count(), total);
        prop_assert!(routed.len() <= total);
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

    /// The PIN gate is exactly "six ASCII digits": a string passes
    /// `validate_verification_pin` if and only if the predicate holds, so no
    /// surprising spelling of a PIN ever gets through and none gets locked out.
    #[test]
    fn the_pin_rule_is_exactly_six_ascii_digits(raw in ".{0,12}") {
        let expected = raw.chars().count() == VERIFICATION_PIN_LEN
            && raw.bytes().all(|byte| byte.is_ascii_digit());
        prop_assert_eq!(
            validate_verification_pin(&raw).is_ok(),
            expected,
            "the function disagreed for raw: {:?}",
            raw
        );
        prop_assert_eq!(
            VerificationPin::new(&raw).is_ok(),
            expected,
            "the type and the function must agree for raw: {:?}",
            raw
        );
        if !expected && raw.len() >= 2 && raw.bytes().all(|byte| byte.is_ascii_digit()) {
            let message = validate_verification_pin(&raw).unwrap_err().to_string();
            prop_assert!(!message.contains(&raw), "the value leaked: {}", message);
        }
    }

    /// Six ASCII digits always build, and the value never shows up in a
    /// rendered form of the type.
    #[test]
    fn a_valid_pin_builds_and_stays_redacted(digits in "[0-9]{6}") {
        let pin = VerificationPin::new(&digits)?;
        prop_assert_eq!(&format!("{pin:?}"), "VerificationPin([redacted])");
        prop_assert_eq!(&format!("{pin}"), "6-digit PIN (redacted)");
        prop_assert_eq!(pin.clone(), VerificationPin::try_from(digits.clone())?);
        let request = meta_whatsapp::RegisterRequest::new(&pin);
        prop_assert_eq!(&request.to_json()["pin"], &Value::String(digits.clone()));
        prop_assert!(!format!("{request:?}").contains(&digits), "the PIN leaked into Debug");
    }

    /// Success decoding is total: it either produces an outcome or a payload
    /// error, and it only ever succeeds for an object.
    #[test]
    fn success_outcome_parsing_is_total(value in arb_json()) {
        match SuccessOutcome::from_value(&value) {
            Ok(outcome) => {
                prop_assert!(value.is_object(), "non-object accepted: {}", value);
                prop_assert!(!outcome.extra.contains_key("success"));
                prop_assert!(!outcome.extra.contains_key("messaging_product"));
            }
            Err(err) => prop_assert!(
                matches!(err, Error::Payload(_)),
                "only a payload error may result: {:?}",
                err
            ),
        }
    }

    /// A template definition is accepted exactly when its name is non-empty and
    /// at most `TEMPLATE_NAME_MAX_BYTES` bytes long.
    #[test]
    fn template_name_acceptance_matches_the_boundary(name in ".{0,600}") {
        let definition = TemplateDefinition::new(&name, "de", TemplateCategory::Utility)
            .body_text("Danke für Ihren Einkauf");
        let expected = !name.is_empty() && name.len() <= TEMPLATE_NAME_MAX_BYTES;
        prop_assert_eq!(definition.validate().is_ok(), expected);
    }

    /// A header text is accepted exactly when it is non-empty, at most
    /// `TEMPLATE_HEADER_MAX_CHARS` characters long and free of placeholders
    /// (which would need an example).
    #[test]
    fn header_text_acceptance_matches_the_boundary(text in ".{0,80}") {
        let accepted = TemplateDefinitionComponent::header_text(&text)
            .validate()
            .is_ok();
        let expected = !text.is_empty()
            && text.chars().count() <= TEMPLATE_HEADER_MAX_CHARS
            && !text.contains("{{");
        prop_assert_eq!(accepted, expected);
    }

    /// Whatever validates renders with the same name, language and body text —
    /// the payload cannot drift from what was checked.
    #[test]
    fn template_definitions_render_what_they_validated(text in "[^{}]{1,200}") {
        let definition = TemplateDefinition::new("order_update", "de_DE", TemplateCategory::Utility)
            .body_text(&text);
        let payload = definition.render().unwrap();
        prop_assert_eq!(&payload["name"], &Value::String("order_update".into()));
        prop_assert_eq!(&payload["language"], &Value::String("de_DE".into()));
        prop_assert_eq!(&payload["components"][0]["text"], &Value::String(text));
        prop_assert_eq!(payload["components"].as_array().unwrap().len(), 1);
    }

    /// An edit is accepted exactly when it carries a category or a component,
    /// on top of a valid name and language.
    #[test]
    fn template_update_acceptance_matches_the_rule(with_category in any::<bool>(), with_body in any::<bool>()) {
        let mut update = TemplateUpdate::new("order_update", "de");
        if with_category {
            update = update.category(TemplateCategory::Utility);
        }
        if with_body {
            update = update.component(TemplateDefinitionComponent::body_text("Danke"));
        }
        prop_assert_eq!(update.validate().is_ok(), with_category || with_body);
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
