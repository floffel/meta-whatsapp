//! Phone-number lifecycle (#13): register, deregister and the two-step
//! verification PIN.
//!
//! The wire shapes come from the Cloud API reference: the `register` and
//! `deregister` edges of `/{PHONE_NUMBER_ID}`, and `POST /{PHONE_NUMBER_ID}`
//! with a `pin` field for two-step verification. What the reference leaves open
//! is read permissively (see `docs/contracts.md`).
//!
//! Two contracts get the most attention here:
//!
//! * the PIN is a secret — it cannot exist unless it is exactly six ASCII
//!   digits, and it stays out of every `Debug`, `Display` and error message,
//! * what goes on the wire is exactly what the reference documents — asserted
//!   per call, including the keys that must *not* be there.

mod common;

use common::{client, json_response, response, Body, MockTransport};
use meta_whatsapp::transport::{Method, Request};
use meta_whatsapp::{
    Error, RegisterRequest, RegistrationBackup, SuccessOutcome, TwoStepVerificationRequest,
    VerificationPin, WhatsAppClient, VERIFICATION_PIN_LEN,
};

const PHONE_NUMBER_URL: &str = "https://graph.facebook.com/v21.0/106540352242922";
const PIN: &str = "123456";

/// The answer Meta gives for all three calls.
fn success_fixture() -> serde_json::Value {
    serde_json::json!({ "success": true })
}

/// Every spelling of "not six ASCII digits", including the empty string and
/// digits from scripts the API would refuse.
const REJECTED_PINS: &[&str] = &[
    "",             // fail closed on empty
    "1",            // far too short
    "12345",        // just outside, below
    "1234567",      // just outside, above
    "12345a",       // a letter
    "123 456",      // a space
    "-12345",       // a sign
    "１２３４５６", // fullwidth digits: six characters, not ASCII
    "١٢٣٤٥٦",       // Arabic-Indic digits: six characters, not ASCII
];

#[tokio::test]
async fn registering_posts_the_documented_body_to_the_register_edge() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&success_fixture()));
    let client = client(transport.clone());

    let outcome = client
        .register(&VerificationPin::new(PIN).unwrap())
        .await
        .unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert_eq!(call.url, format!("{PHONE_NUMBER_URL}/register"));
    assert_eq!(call.token, "EAAG-test-token");
    assert_eq!(
        call.body,
        Body::Json(serde_json::json!({ "messaging_product": "whatsapp", "pin": PIN })),
        "exactly the documented keys, nothing else"
    );
    assert!(outcome.success);
}

#[tokio::test]
async fn registering_with_a_backup_adds_the_blob_in_the_same_call() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&success_fixture()));
    let client = client(transport.clone());

    let backup = RegistrationBackup::new("backup-password", "BASE64BLOB").unwrap();
    let outcome = client
        .register_with_backup(&VerificationPin::new(PIN).unwrap(), &backup)
        .await
        .unwrap();

    assert!(outcome.success);
    let call = transport.only_call();
    assert_eq!(call.url, format!("{PHONE_NUMBER_URL}/register"));
    assert_eq!(
        call.body,
        Body::Json(serde_json::json!({
            "messaging_product": "whatsapp",
            "pin": PIN,
            "backup": { "password": "backup-password", "data": "BASE64BLOB" }
        })),
        "one call, migration included"
    );
}

#[tokio::test]
async fn deregistering_is_a_bodyless_delete_on_the_deregister_edge() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&success_fixture()));
    let client = client(transport.clone());

    let outcome = client.deregister().await.unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Delete);
    assert_eq!(call.url, format!("{PHONE_NUMBER_URL}/deregister"));
    assert_eq!(call.token, "EAAG-test-token");
    assert_eq!(call.body, Body::Empty, "deregister carries no body");
    assert!(outcome.success);
}

#[tokio::test]
async fn setting_the_pin_posts_only_the_pin_to_the_number_node() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&success_fixture()));
    let client = client(transport.clone());

    let outcome = client
        .set_two_step_verification_pin(&VerificationPin::new("654321").unwrap())
        .await
        .unwrap();

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert_eq!(call.url, PHONE_NUMBER_URL, "the node itself, no sub-edge");
    let Body::Json(body) = &call.body else {
        panic!("registering the PIN must send a JSON body: {:?}", call.body);
    };
    assert_eq!(body.as_object().unwrap().len(), 1, "no other key: {body}");
    assert_eq!(body["pin"], "654321");
    assert!(
        body.get("messaging_product").is_none(),
        "the reference sends the pin alone here: {body}"
    );
    assert!(outcome.success);
}

#[test]
fn the_url_builders_point_at_the_documented_edges() {
    let client = client(MockTransport::new());

    assert_eq!(
        client.register_url(),
        format!("{PHONE_NUMBER_URL}/register")
    );
    assert_eq!(
        client.deregister_url(),
        format!("{PHONE_NUMBER_URL}/deregister")
    );
    assert_eq!(client.two_step_verification_url(), PHONE_NUMBER_URL);
    assert_eq!(
        client.two_step_verification_url(),
        client.phone_number_url(),
        "the PIN is set on the number node, not on an edge below it"
    );
}

#[tokio::test]
async fn a_success_flag_of_any_documented_spelling_is_accepted() {
    // The reference says boolean; the field has also been seen as a number, a
    // string, or absent. All of them decode, none of them panic.
    for (body, expected) in [
        (serde_json::json!({"success": true}), true),
        (serde_json::json!({"success": false}), false),
        (serde_json::json!({"success": "true"}), true),
        (serde_json::json!({"success": "TRUE"}), true),
        (serde_json::json!({"success": "1"}), true),
        (serde_json::json!({"success": "0"}), false),
        (serde_json::json!({"success": 1}), true),
        (serde_json::json!({"success": 0}), false),
        (serde_json::json!({"success": null}), false),
        (serde_json::json!({}), false),
    ] {
        let transport = MockTransport::new();
        transport.push_response(json_response(&body));
        let outcome = client(transport)
            .deregister()
            .await
            .unwrap_or_else(|err| panic!("{body} must decode: {err:?}"));
        assert_eq!(outcome.success, expected, "{body}");
    }
}

#[tokio::test]
async fn fields_the_crate_does_not_know_survive_in_the_outcome() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "success": true,
        "messaging_product": "whatsapp",
        "a_field_invented_tomorrow": {"kept": true}
    })));

    let outcome = client(transport).deregister().await.unwrap();

    assert!(outcome.success);
    assert_eq!(outcome.messaging_product.as_deref(), Some("whatsapp"));
    assert_eq!(outcome.extra["a_field_invented_tomorrow"]["kept"], true);
    assert!(
        !outcome.extra.contains_key("success"),
        "known keys are consumed"
    );
}

#[tokio::test]
async fn an_answer_that_cannot_be_a_result_is_a_payload_error() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!([])));

    let err = client(transport.clone()).deregister().await.unwrap_err();

    assert!(matches!(err, Error::Payload(_)), "{err:?}");
    assert_eq!(transport.call_count(), 1, "the reply was read once");
}

#[test]
fn a_shape_error_names_the_shape_and_never_the_value() {
    // A hostile or buggy reply field could contain the PIN we just sent. The
    // payload errors therefore describe the JSON type that arrived, never its
    // contents, so the error cannot carry a secret into a log line.
    let surprises: Vec<serde_json::Value> = vec![
        serde_json::json!({"success": {"pin": PIN}}),
        serde_json::json!({"success": ["123456"]}),
        serde_json::json!({"success": null, "messaging_product": {"pin": PIN}}),
        serde_json::json!({"success": "hunter2", "messaging_product": "BASE64BLOB"}),
    ];

    for value in surprises {
        let err = SuccessOutcome::from_value(&value).unwrap_err();
        assert!(matches!(err, Error::Payload(_)), "{err:?}");
        for rendered in [format!("{err:?}"), format!("{err}")] {
            assert!(!rendered.contains(PIN), "PIN leaked: {rendered}");
            assert!(!rendered.contains("hunter2"), "password leaked: {rendered}");
            assert!(!rendered.contains("BASE64BLOB"), "blob leaked: {rendered}");
        }
    }

    // The message is still useful: it says which field and what arrived.
    let err = SuccessOutcome::from_value(&serde_json::json!({"success": {"pin": PIN}}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("success"), "{err}");
    assert!(err.contains("an object"), "{err}");
}

#[tokio::test]
async fn a_rejected_write_stays_the_api_error_it_was() {
    // A 4xx is terminal: not wrapped in Ambiguous, not retried.
    let transport = MockTransport::new();
    transport.push_response(response(
        400,
        &[],
        br#"{"error":{"code":100,"message":"invalid pin"}}"#,
    ));
    let client = client(transport.clone());

    let err = client
        .register(&VerificationPin::new(PIN).unwrap())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Api { status: 400, .. }), "{err:?}");
    assert!(!err.is_ambiguous());
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn a_pin_that_is_not_six_ascii_digits_cannot_be_built() {
    // The gate is the type: without a `VerificationPin` the client methods are
    // not callable, so a malformed PIN cannot reach the network by construction.
    assert_eq!(VERIFICATION_PIN_LEN, 6);
    assert!(VerificationPin::new("123456").is_ok(), "just inside");
    assert!(VerificationPin::new("000000").is_ok(), "zeros are a PIN");

    for rejected in REJECTED_PINS {
        let err = VerificationPin::new(rejected)
            .err()
            .unwrap_or_else(|| panic!("{rejected:?} must be rejected"));
        assert!(
            matches!(err, Error::Validation { .. }),
            "{rejected:?}: {err:?}"
        );
        assert!(
            VerificationPin::try_from(*rejected).is_err(),
            "TryFrom<&str> applies the same rule for {rejected:?}"
        );
        assert!(
            VerificationPin::try_from((*rejected).to_string()).is_err(),
            "TryFrom<String> applies the same rule for {rejected:?}"
        );
        // The message names the field and the rule, never the value.
        let message = err.to_string();
        assert!(message.contains("pin"), "{message}");
        if !rejected.is_empty() {
            assert!(!message.contains(*rejected), "value leaked: {message}");
        }
    }
}

#[tokio::test]
async fn an_empty_backup_part_is_refused_before_the_call() {
    let pin = VerificationPin::new(PIN).unwrap();

    for (password, data) in [("", "blob"), ("pw", "")] {
        let err = RegistrationBackup::new(password, data).unwrap_err();
        assert!(matches!(err, Error::Validation { .. }), "{err:?}");
        assert!(
            !err.to_string().contains("blob"),
            "the blob stays out: {err}"
        );
    }

    // The client still works with a complete backup.
    let transport = MockTransport::new();
    transport.push_response(json_response(&success_fixture()));
    let client = client(transport.clone());
    let backup = RegistrationBackup::new("pw", "blob").unwrap();
    assert!(
        client
            .register_with_backup(&pin, &backup)
            .await
            .unwrap()
            .success
    );
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn the_bodies_hold_exactly_what_the_reference_documents() {
    let pin = VerificationPin::new(PIN).unwrap();
    assert_eq!(
        RegisterRequest::new(&pin).to_json(),
        serde_json::json!({"messaging_product": "whatsapp", "pin": PIN})
    );
    assert!(
        RegisterRequest::new(&pin).to_json().get("backup").is_none(),
        "the backup is opt-in"
    );
    assert_eq!(
        RegisterRequest::new(&pin)
            .with_backup(&RegistrationBackup::new("pw", "blob").unwrap())
            .to_json(),
        serde_json::json!({
            "messaging_product": "whatsapp",
            "pin": PIN,
            "backup": {"password": "pw", "data": "blob"}
        })
    );
    assert_eq!(
        TwoStepVerificationRequest::new(&pin).to_json(),
        serde_json::json!({"pin": PIN})
    );
}

#[test]
fn the_pin_never_appears_in_a_debug_or_display_string() {
    let pin = VerificationPin::new(PIN).unwrap();
    let backup = RegistrationBackup::new("hunter2", "BASE64BLOB").unwrap();
    let register = RegisterRequest::new(&pin).with_backup(&backup);
    let two_step = TwoStepVerificationRequest::new(&pin);
    let body = register.to_json();
    let url = PHONE_NUMBER_URL.to_string();
    let token = "EAAG-test-token".to_string();
    let request = Request::post_json(&url, &token, &body);

    let validation = VerificationPin::new("12345").unwrap_err();
    let ambiguous = Error::ambiguous("POST register", Some(500), Error::transport("timeout"));

    // The values are exactly where they belong: in the body that gets sent.
    assert_eq!(body["pin"], PIN);
    assert_eq!(body["backup"]["password"], "hunter2");
    assert_eq!(
        format!("{pin:?}"),
        "VerificationPin([redacted])",
        "the Debug form is fixed text"
    );
    assert_eq!(format!("{pin}"), "6-digit PIN (redacted)");
    assert!(format!("{request:?}").contains("[redacted]"));

    for rendered in [
        format!("{pin:?}"),
        format!("{pin}"),
        format!("{backup:?}"),
        format!("{register:?}"),
        format!("{two_step:?}"),
        format!("{request:?}"),
        format!("{validation:?}"),
        format!("{validation}"),
        format!("{ambiguous:?}"),
        format!("{ambiguous}"),
        format!("{:?}", client(MockTransport::new())),
    ] {
        assert!(!rendered.contains(PIN), "PIN leaked into: {rendered}");
        assert!(!rendered.contains("hunter2"), "password leaked: {rendered}");
        assert!(
            !rendered.contains("BASE64BLOB"),
            "backup blob leaked: {rendered}"
        );
        assert!(!rendered.contains("EAAG"), "token leaked: {rendered}");
    }
}

#[tokio::test]
async fn nothing_is_sent_for_a_number_state_that_cannot_be_built() {
    // The client methods only accept a `VerificationPin`, and that value only
    // exists if it validated — so a malformed PIN cannot reach the transport.
    let transport = MockTransport::new();
    let _client: WhatsAppClient<MockTransport> = client(transport.clone());

    for rejected in REJECTED_PINS {
        assert!(
            VerificationPin::new(rejected).is_err(),
            "{rejected:?} must not validate"
        );
    }
    assert_eq!(
        transport.call_count(),
        0,
        "no call was made, not even a doomed one"
    );
}
