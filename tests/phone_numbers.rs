//! WABA phone-number listing and onboarding (#24).
//!
//! Every request is scripted through the recording transport, so these tests
//! assert the exact method, URL, body and retry class without opening a socket.

mod common;

use common::{client, instant_retry, json_response, response, Body, MockTransport, Reply};
use meta_whatsapp::phone_number::{
    CodeLanguage, CodeMethod, PhoneNumberCreateRequest, PhoneNumberFilter, PhoneNumberQuery,
    PhoneNumberSort, RequestCodeRequest, VerificationCode, VerifyCodeRequest,
    VERIFIED_NAME_MAX_CHARS, VERIFIED_NAME_MIN_CHARS,
};
use meta_whatsapp::transport::Method;
use meta_whatsapp::Error;

const WABA: &str = "102290129340398";
const PHONE_ID: &str = "1906385232743451";
const SERVER_ERROR: &[u8] = br#"{"error":{"code":2,"message":"internal"}}"#;

#[tokio::test]
async fn list_hits_the_waba_phone_numbers_edge_with_the_documented_query() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "data": [{
            "id": PHONE_ID,
            "display_phone_number": "+1 631-555-5555",
            "verified_name": "Jasper's Market",
            "status": "LINKED",
            "quality_rating": "GREEN",
            "country_code": "US",
            "country_dial_code": "1",
            "code_verification_status": "VERIFIED",
            "unified_cert_status": "APPROVED",
            "account_mode": "LIVE",
            "host_platform": "CLOUD_API",
            "messaging_limit_tier": "TIER_1K",
            "is_official_business_account": true,
            "username": "jaspers_market",
            "new_field": {"kept": true}
        }],
        "paging": {"cursors": {"after": "QVFIU"}}
    })));
    let query = PhoneNumberQuery::new()
        .limit(25)
        .fields(["id", "display_phone_number", "status"])
        .filter(PhoneNumberFilter::equal("account_mode", "LIVE").unwrap())
        .sort(PhoneNumberSort::LastOnboardedDesc);
    let client = client(transport.clone());

    let page = client.waba_phone_numbers(WABA, &query).await.unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Get);
    assert_eq!(call.token, "EAAG-test-token");
    let expected = concat!(
        "https://graph.facebook.com/v21.0/102290129340398/phone_numbers?",
        "limit=25&fields=id%2Cdisplay_phone_number%2Cstatus&",
        "filtering=%5B%7B%22field%22%3A%22account_mode%22%2C%22operator%22%3A%22EQUAL%22%2C%22value%22%3A%22LIVE%22%7D%5D&",
        "sort=last_onboarded_time.desc"
    );
    assert_eq!(call.url, expected);
    assert_eq!(call.body, Body::Empty);
    assert_eq!(page.data[0].id.as_deref(), Some(PHONE_ID));
    assert_eq!(
        page.data[0].quality_rating,
        Some(meta_whatsapp::business::QualityRating::Green)
    );
    assert_eq!(page.data[0].extra["new_field"]["kept"], true);
    assert_eq!(page.next_cursor(), Some("QVFIU"));
}

#[tokio::test]
async fn the_list_walk_follows_cursors_and_stops_on_a_repeated_cursor() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "data": [{"id": "1"}],
        "paging": {"cursors": {"after": "NEXT"}}
    })));
    transport.push_response(json_response(&serde_json::json!({
        "data": [{"id": "2"}],
        "paging": {"cursors": {"after": "NEXT"}}
    })));
    let client = client(transport.clone());

    let numbers = client
        .all_waba_phone_numbers(WABA, &PhoneNumberQuery::new().limit(2))
        .await
        .unwrap();

    assert_eq!(
        numbers
            .iter()
            .filter_map(|n| n.id.as_deref())
            .collect::<Vec<_>>(),
        ["1", "2"]
    );
    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].url,
        "https://graph.facebook.com/v21.0/102290129340398/phone_numbers?limit=2"
    );
    assert_eq!(
        calls[1].url,
        "https://graph.facebook.com/v21.0/102290129340398/phone_numbers?limit=2&after=NEXT"
    );
}

#[tokio::test]
async fn all_waba_phone_numbers_stops_when_next_exists_without_a_cursor() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "data": [{"id": "1"}],
        "paging": {"next": "https://graph.facebook.com/next"}
    })));
    let numbers = client(transport.clone())
        .all_waba_phone_numbers(WABA, &PhoneNumberQuery::new())
        .await
        .unwrap();
    assert_eq!(numbers.len(), 1);
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn create_posts_the_exact_documented_body() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({"id": PHONE_ID})));
    let request = PhoneNumberCreateRequest::new("16315551000", "My Business")
        .unwrap()
        .country_dial_code("1")
        .migrate()
        .preverified_id("preverified_12345");
    let outcome = client(transport.clone())
        .create_waba_phone_number(WABA, &request)
        .await
        .unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/102290129340398/phone_numbers"
    );
    assert_eq!(
        call.body,
        Body::Json(serde_json::json!({
            "phone_number": "16315551000",
            "verified_name": "My Business",
            "cc": "1",
            "migrate_phone_number": true,
            "preverified_id": "preverified_12345"
        }))
    );
    assert_eq!(outcome.id, PHONE_ID);
}

#[tokio::test]
async fn request_code_posts_exactly_the_method_and_language() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({"success": true})));
    let request = RequestCodeRequest::new(CodeMethod::Voice, CodeLanguage::new("es_ES").unwrap());
    let outcome = client(transport.clone())
        .request_phone_number_code(PHONE_ID, &request)
        .await
        .unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert_eq!(
        call.url,
        format!("https://graph.facebook.com/v21.0/{PHONE_ID}/request_code")
    );
    assert_eq!(
        call.body,
        Body::Json(serde_json::json!({"code_method": "VOICE", "language": "es_ES"}))
    );
    assert!(outcome.success);
}

#[tokio::test]
async fn verify_code_posts_only_the_code_and_redacts_transport_debug() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "success": true,
        "id": PHONE_ID,
        "future_field": 7
    })));
    let request = VerifyCodeRequest::new(&VerificationCode::new("123456").unwrap());
    let outcome = client(transport.clone())
        .verify_phone_number_code(PHONE_ID, &request)
        .await
        .unwrap();

    let call = transport.only_call();
    assert_eq!(
        call.url,
        format!("https://graph.facebook.com/v21.0/{PHONE_ID}/verify_code")
    );
    assert_eq!(call.body, Body::Json(serde_json::json!({"code": "123456"})));
    assert_eq!(outcome.id.as_deref(), Some(PHONE_ID));
    assert_eq!(outcome.extra["future_field"], 7);
    assert!(!format!("{request:?}").contains("123456"));
    let secret = serde_json::json!({"code": "123456"});
    let debug_request = meta_whatsapp::transport::Request::post_json(
        "https://graph.facebook.com/v21.0/1/verify_code",
        "token",
        &secret,
    );
    assert!(!format!("{debug_request:?}").contains("123456"));
}

#[tokio::test]
async fn malformed_inputs_never_reach_the_transport() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    assert!(client
        .waba_phone_numbers("../me", &PhoneNumberQuery::new())
        .await
        .is_err());
    assert!(client
        .waba_phone_numbers(WABA, &PhoneNumberQuery::new().limit(0))
        .await
        .is_err());
    assert!(client
        .request_phone_number_code(
            "../me",
            &RequestCodeRequest::new(CodeMethod::Sms, CodeLanguage::new("en_US").unwrap())
        )
        .await
        .is_err());
    assert!(client
        .verify_phone_number_code(
            "../me",
            &VerifyCodeRequest::new(&VerificationCode::new("123456").unwrap())
        )
        .await
        .is_err());
    let create = PhoneNumberCreateRequest::new("16315551000", "My Business").unwrap();
    assert!(client
        .create_waba_phone_number("../me", &create)
        .await
        .is_err());

    assert_eq!(transport.call_count(), 0);
}

#[test]
fn limits_have_boundary_pairs() {
    for digits in 7..=15 {
        assert!(
            PhoneNumberCreateRequest::new("1".repeat(digits), "AB").is_ok(),
            "{digits} digits"
        );
    }
    assert!(PhoneNumberCreateRequest::new("123456", "AB").is_err());
    assert!(PhoneNumberCreateRequest::new("1234567890123456", "AB").is_err());
    assert!(PhoneNumberCreateRequest::new("06315551000", "AB").is_err());
    assert!(PhoneNumberCreateRequest::new("+16315551000", "AB").is_err());

    let min_name = "A".repeat(VERIFIED_NAME_MIN_CHARS);
    let max_name = "A".repeat(VERIFIED_NAME_MAX_CHARS);
    assert!(PhoneNumberCreateRequest::new("16315551000", min_name).is_ok());
    assert!(PhoneNumberCreateRequest::new("16315551000", max_name).is_ok());
    assert!(PhoneNumberCreateRequest::new("16315551000", "A").is_err());
    assert!(PhoneNumberCreateRequest::new("16315551000", "A".repeat(76)).is_err());

    let page = |limit| PhoneNumberQuery::new().limit(limit).validate();
    assert!(page(1).is_ok());
    assert!(page(100).is_ok());
    assert!(page(0).is_err());
    assert!(page(101).is_err());
}

#[test]
fn verification_codes_are_exactly_six_ascii_digits() {
    for valid in ["000000", "123456", "999999"] {
        assert!(VerificationCode::new(valid).is_ok(), "{valid}");
    }
    for invalid in [
        "",
        "12345",
        "1234567",
        "12345a",
        "12 456",
        "１２３４５６",
        "١٢٣٤٥٦",
    ] {
        assert!(VerificationCode::new(invalid).is_err(), "{invalid:?}");
    }
}

#[test]
fn languages_follow_the_five_character_ll_cc_shape() {
    for valid in ["en_US", "de_DE", "pt_BR", "zh_CN"] {
        assert!(CodeLanguage::new(valid).is_ok(), "{valid}");
    }
    for invalid in ["", "en", "eng_US", "EN_us", "en-us", "en_USA", "12_34"] {
        assert!(CodeLanguage::new(invalid).is_err(), "{invalid:?}");
    }
}

#[tokio::test]
async fn create_5xx_is_ambiguous_and_never_retried() {
    let transport = MockTransport::new();
    transport.push_response(response(500, &[], SERVER_ERROR));
    transport.push_response(response(200, &[], br#"{"id":"2"}"#));
    let request = PhoneNumberCreateRequest::new("16315551000", "My Business").unwrap();
    let err = client(transport.clone())
        .create_waba_phone_number(WABA, &request)
        .await
        .unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn code_requests_retry_429_and_before_write_failures() {
    let transport = MockTransport::new();
    transport.push(Reply::not_sent("dns"));
    transport.push_response(json_response(&serde_json::json!({"success": true})));
    let request = RequestCodeRequest::new(CodeMethod::Sms, CodeLanguage::new("en_US").unwrap());
    let client = client(transport.clone()).with_retry(instant_retry(2));
    assert!(
        client
            .request_phone_number_code(PHONE_ID, &request)
            .await
            .unwrap()
            .success
    );
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn verification_after_write_failure_is_ambiguous_and_never_retried() {
    let transport = MockTransport::new();
    transport.push(Reply::after_write("timeout"));
    let request = VerifyCodeRequest::new(&VerificationCode::new("123456").unwrap());
    let err = client(transport.clone())
        .verify_phone_number_code(PHONE_ID, &request)
        .await
        .unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn list_retries_5xx_as_a_read() {
    let transport = MockTransport::new();
    transport.push_response(response(500, &[], SERVER_ERROR));
    transport.push_response(json_response(&serde_json::json!({"data": []})));
    let client = client(transport.clone()).with_retry(instant_retry(2));
    assert_eq!(
        client
            .waba_phone_numbers(WABA, &PhoneNumberQuery::new())
            .await
            .unwrap()
            .data,
        []
    );
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn malformed_success_bodies_are_payload_errors() {
    let cases = [
        serde_json::json!("not an object"),
        serde_json::json!({"success": "yes"}),
        serde_json::json!({"id": 7}),
        serde_json::json!({"data": {}}),
    ];
    for body in cases {
        let transport = MockTransport::new();
        transport.push_response(json_response(&body));
        let result = match body {
            _ if body.get("data").is_some() => client(transport)
                .waba_phone_numbers(WABA, &PhoneNumberQuery::new())
                .await
                .map(|_| ()),
            _ if body.get("id").is_some() => {
                let request = PhoneNumberCreateRequest::new("16315551000", "My Business").unwrap();
                client(transport)
                    .create_waba_phone_number(WABA, &request)
                    .await
                    .map(|_| ())
            }
            _ => client(transport)
                .request_phone_number_code(
                    PHONE_ID,
                    &RequestCodeRequest::new(CodeMethod::Sms, CodeLanguage::new("en_US").unwrap()),
                )
                .await
                .map(|_| ()),
        };
        assert!(
            matches!(result, Err(Error::Payload(_))),
            "{body}: {result:?}"
        );
    }
}
