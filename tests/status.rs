//! Reading the delivery status of a message we sent (issue #12): the *Read a
//! Message* endpoint `GET /{phone-number-id}?id={wamid}`.
//!
//! Fixture per documented status, the wamid shape rules as boundary pairs, and
//! the permissive-decoding rules (§7) for unknown statuses and fields.

mod common;

use common::{client, instant_retry, json_response, response, MockTransport, Reply};
use meta_whatsapp::status::{validate_wamid, DeliveryStatus, MessageStatus, MAX_WAMID_LEN};
use meta_whatsapp::Error;
use serde_json::{json, Value};

const PHONE_NUMBER_ID: &str = "106540352242922";
const WAMID: &str = "wamid.HBgLMTU3MjkwMDg0NTU5NRMVAgEYGBQyQTc";

/// The documented envelope around one entry.
fn envelope(entry: &Value) -> Value {
    json!({ "messaging_product": "whatsapp", "messages": [entry] })
}

/// A client plus transport wired for a single canned body.
fn client_answering(body: &Value) -> (meta_whatsapp::WhatsAppClient<MockTransport>, MockTransport) {
    let transport = MockTransport::json(body);
    (client(transport.clone()), transport)
}

#[tokio::test]
async fn a_sent_message_is_read_over_get_on_the_phone_number_node() {
    let transport = MockTransport::json(&envelope(&json!({
        "id": WAMID,
        "status": "sent",
        "timestamp": "1729600000",
        "recipient_id": "4915112345678",
    })));
    let client = client(transport.clone());

    let status = client.message_status(WAMID).await.unwrap();

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "GET", "a read, not a write");
    assert_eq!(
        call.url,
        format!(
            "https://graph.facebook.com/v21.0/{PHONE_NUMBER_ID}?id={WAMID}&messaging_product=whatsapp"
        )
    );
    assert_eq!(call.token, "EAAG-test-token");
    assert_eq!(call.body, common::Body::Empty, "a read carries no body");

    assert_eq!(status.id, WAMID);
    assert_eq!(status.status, DeliveryStatus::Sent);
    assert_eq!(status.status.as_str(), "sent");
    assert_eq!(status.timestamp, "1729600000");
    assert_eq!(status.timestamp_secs(), Some(1_729_600_000));
    assert_eq!(status.recipient_id, "4915112345678");
    assert_eq!(status.errors.len(), 0, "a sent message carries no errors");
    assert_eq!(status.first_error(), None);
}

#[tokio::test]
async fn every_documented_status_decodes_to_its_variant() {
    for (wire, expected) in [
        ("sent", DeliveryStatus::Sent),
        ("delivered", DeliveryStatus::Delivered),
        ("read", DeliveryStatus::Read),
        ("failed", DeliveryStatus::Failed),
    ] {
        let (client, _transport) = client_answering(&envelope(&json!({
            "id": WAMID,
            "status": wire,
            "timestamp": "1729600001",
            "recipient_id": "4915112345678",
        })));

        let status = client.message_status(WAMID).await.unwrap();
        assert_eq!(status.status, expected, "wire status {wire:?}");
        assert_eq!(status.status.as_str(), wire);
    }
}

#[tokio::test]
async fn a_failed_message_carries_its_errors() {
    let body = envelope(&json!({
        "id": WAMID,
        "status": "failed",
        "timestamp": "1729600002",
        "recipient_id": "4915112345678",
        "errors": [
            {
                "code": 131_047,
                "title": "Re-engagement message",
                "message": "Re-engagement message",
                "error_data": { "details": "Message too old" },
                "href": "https://developers.facebook.com/docs/whatsapp/cloud-api/support/error-codes/",
            }
        ],
    }));
    let (client, _transport) = client_answering(&body);

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Failed);
    assert_eq!(status.errors.len(), 1);
    let error = &status.errors[0];
    assert_eq!(error.code, 131_047);
    assert_eq!(error.title, "Re-engagement message");
    assert_eq!(error.message.as_deref(), Some("Re-engagement message"));
    assert_eq!(
        error
            .error_data
            .as_ref()
            .and_then(Value::as_object)
            .map(serde_json::Map::len),
        Some(1)
    );
    // `href` is not modelled — it is ignored rather than rejected (§7).
    assert_eq!(status.first_error().map(|first| first.code), Some(131_047));
}

#[tokio::test]
async fn an_unknown_status_is_carried_through_instead_of_failing() {
    let (client, _transport) = client_answering(&envelope(&json!({
        "id": WAMID,
        "status": "played",
        "timestamp": "1729600003",
    })));

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Unknown("played".into()));
    assert_eq!(status.status.as_str(), "played");
    // And it survives a re-serialisation with the raw string intact.
    let rendered = serde_json::to_string(&status.status).unwrap();
    assert_eq!(rendered, "\"played\"");
    let reparsed: DeliveryStatus = serde_json::from_str(&rendered).unwrap();
    assert_eq!(reparsed, DeliveryStatus::Unknown("played".into()));
}

#[tokio::test]
async fn unknown_fields_are_ignored_and_missing_ones_default() {
    let (client, _transport) = client_answering(&json!({
        "messaging_product": "whatsapp",
        "contact": "4915112345678",
        "messages": [{
            "id": WAMID,
            "status": "read",
            "future_object": {"a": [1, 2, 3]},
            "future_bool": true,
        }],
    }));

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Read);
    assert_eq!(status.timestamp, "");
    assert_eq!(status.timestamp_secs(), None);
    assert_eq!(status.recipient_id, "");
    assert_eq!(status.errors.len(), 0, "a read message carries no errors");
}

#[test]
fn field_order_does_not_matter() {
    let forward: MessageStatus = serde_json::from_str(
        r#"{"id":"wamid.1","status":"delivered","timestamp":"1729600004","recipient_id":"4915"}"#,
    )
    .unwrap();
    let backward: MessageStatus = serde_json::from_str(
        r#"{"recipient_id":"4915","timestamp":"1729600004","status":"delivered","id":"wamid.1"}"#,
    )
    .unwrap();
    assert_eq!(forward, backward);
    assert_eq!(forward.status, DeliveryStatus::Delivered);
}

#[test]
fn a_status_that_is_not_a_string_is_carried_not_rejected() {
    for raw in ["7", "true", "null", "[1]", "{}"] {
        let parsed: Result<MessageStatus, serde_json::Error> =
            serde_json::from_str(&format!(r#"{{"id":"1","status":{raw}}}"#));
        let status = parsed.unwrap_or_else(|err| panic!("status {raw} must decode: {err}"));
        match &status.status {
            DeliveryStatus::Unknown(text) => assert!(!text.is_empty() || raw == "null"),
            other => panic!("only the null status may map to a known variant: {other:?}"),
        }
    }
}

#[test]
fn the_envelope_is_tolerant_and_the_empty_answer_is_an_error() {
    // Documented envelope.
    let entries =
        MessageStatus::parse_all(&envelope(&json!({"id": "a", "status": "sent"}))).unwrap();
    assert_eq!(entries.len(), 1);

    // Several entries: `parse` answers the one you asked about, `parse_all`
    // keeps the rest.
    let many = MessageStatus::parse_all(&json!({
        "messages": [{"id": "a", "status": "sent"}, {"id": "b", "status": "read"}],
    }))
    .unwrap();
    assert_eq!(many.len(), 2);
    assert_eq!(many[1].status, DeliveryStatus::Read);
    assert_eq!(many[0].id, "a");

    // A bare entry (what a proxy in front of the Graph API hands back).
    let bare = MessageStatus::parse_all(&json!({"id": "c", "status": "delivered"})).unwrap();
    assert_eq!(bare[0].id, "c");

    // An array of entries.
    let array = MessageStatus::parse_all(&json!([{"id": "d", "status": "read"}])).unwrap();
    assert_eq!(array[0].id, "d");

    // Nothing to report.
    let empty = MessageStatus::parse(&json!({"messages": []})).unwrap_err();
    assert!(matches!(empty, Error::Payload(_)), "{empty:?}");
    assert_eq!(
        MessageStatus::parse_all(&json!({"messages": []}))
            .unwrap()
            .len(),
        0
    );

    // Shapes that cannot carry a status at all.
    for impossible in [
        json!({"success": true}),
        json!("nope"),
        json!(7),
        json!(null),
        json!({"messages": "wamid.1"}),
    ] {
        let err = MessageStatus::parse_all(&impossible).unwrap_err();
        assert!(matches!(err, Error::Payload(_)), "{impossible} -> {err:?}");
    }
}

#[test]
fn an_unreadable_entry_is_a_json_error_not_a_payload_one() {
    // The boundary between "unfamiliar" and "unreadable": a status this crate
    // does not know is permissive (§7), but an entry whose shape cannot be read
    // at all is reported as a decoding failure rather than silently emptied.
    for malformed in [
        json!({"messages": ["wamid.1"]}),
        json!({"messages": [{"errors": "not a list"}]}),
        json!({"messages": [{"errors": [{"code": "131047"}]}]}),
    ] {
        let err = MessageStatus::parse_all(&malformed).unwrap_err();
        assert!(matches!(err, Error::Json { .. }), "{malformed} -> {err:?}");
    }

    // Just inside the line: an unknown status word and a non-string status are
    // carried, not rejected.
    for tolerated in [
        json!({"messages": [{"status": "played"}]}),
        json!({"messages": [{"status": 7}]}),
        json!({"messages": [{"status": null}]}),
        json!({"messages": [{"unknown_field": true}]}),
        // serde reads a struct from a sequence too, and every StatusError field
        // defaults, so this is unfamiliar rather than unreadable.
        json!({"id": "a", "errors": [[]]}),
    ] {
        let entries = MessageStatus::parse_all(&tolerated)
            .unwrap_or_else(|err| panic!("{tolerated} must decode: {err}"));
        assert_eq!(entries.len(), 1);
    }
}

#[test]
fn the_query_value_is_percent_encoded() {
    let transport = MockTransport::new();
    let client = client(transport);

    assert_eq!(
        client.message_status_url("wamid.HBgLMTU="),
        format!("https://graph.facebook.com/v21.0/{PHONE_NUMBER_ID}?id=wamid.HBgLMTU%3D&messaging_product=whatsapp")
    );
    assert_eq!(
        client.message_status_url("a+b/c"),
        format!("https://graph.facebook.com/v21.0/{PHONE_NUMBER_ID}?id=a%2Bb%2Fc&messaging_product=whatsapp"),
        "the base64 alphabet must not be read as form encoding"
    );
    assert_eq!(
        client.message_status_url("wamid.1-2_3:4"),
        format!("https://graph.facebook.com/v21.0/{PHONE_NUMBER_ID}?id=wamid.1-2_3%3A4&messaging_product=whatsapp"),
        "the separators a wamid may hold stay inside one query value"
    );
    assert_eq!(
        client.message_status_url("a~b-c_d"),
        format!("https://graph.facebook.com/v21.0/{PHONE_NUMBER_ID}?id=a~b-c_d&messaging_product=whatsapp"),
        "unreserved characters stay verbatim"
    );
}

#[tokio::test]
async fn a_malformed_wamid_is_refused_before_any_request() {
    // Just inside the boundary: one character, and the maximum length.
    for accepted in ["1", &"a".repeat(MAX_WAMID_LEN), WAMID] {
        let transport = MockTransport::json(&envelope(&json!({"id": accepted, "status": "sent"})));
        let client = client(transport.clone());
        let status = client
            .message_status(accepted)
            .await
            .unwrap_or_else(|err| panic!("{accepted:?} must be accepted: {err}"));
        assert_eq!(status.status, DeliveryStatus::Sent);
        assert_eq!(transport.call_count(), 1);
    }

    // Just outside it, plus every character class that must never reach a URL.
    let rejected = [
        String::new(),                  // nothing to ask about
        " ".to_string(),                // a space is not an id
        "\t".to_string(),               // tab
        "\n".to_string(),               // newline: request splitting
        "\0".to_string(),               // NUL
        "wamid.1 ".to_string(),         // trailing space
        "wamid 1".to_string(),          // inner space
        "wamid.1?x=2&id=3".to_string(), // query smuggling
        "wamid.1&x=2".to_string(),      // second parameter
        "wamid.1#frag".to_string(),     // fragment
        "w%41mid".to_string(),          // pre-encoded byte
        "wamid@1".to_string(),          // userinfo separator
        "wamid(1)".to_string(),         // punctuation
        "../etc/passwd".to_string(),    // path traversal attempts
        "wämid".to_string(),            // non-ASCII
        "' OR 1=1".to_string(),         // the classic
        "a".repeat(MAX_WAMID_LEN + 1),  // one byte over the cap
    ];
    for candidate in rejected {
        let transport = MockTransport::new();
        let client = client(transport.clone());

        let err = client
            .message_status(&candidate)
            .await
            .expect_err("must be refused");

        assert!(
            matches!(&err, Error::Validation { field, .. } if *field == "wamid"),
            "{candidate:?} -> {err:?}"
        );
        assert_eq!(
            transport.call_count(),
            0,
            "the id is checked before the request is built: {candidate:?}"
        );
    }
}

#[test]
fn the_validator_names_its_field_at_both_ends() {
    assert!(validate_wamid(&"a".repeat(MAX_WAMID_LEN)).is_ok());
    let err = validate_wamid(&"a".repeat(MAX_WAMID_LEN + 1)).unwrap_err();
    assert!(
        matches!(&err, Error::Validation { message, .. } if message.contains("256")),
        "{err}"
    );
    let empty = validate_wamid("").unwrap_err();
    assert!(
        matches!(&empty, Error::Validation { field, message } if *field == "wamid" && message.contains("empty")),
        "{empty}"
    );
}

#[tokio::test]
async fn a_5xx_on_the_read_is_retried() {
    let transport = MockTransport::new();
    transport.push_response(response(
        500,
        &[],
        b"{\"error\":{\"code\":2,\"message\":\"internal\"}}",
    ));
    transport.push_response(json_response(&envelope(
        &json!({"id": WAMID, "status": "delivered"}),
    )));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Delivered);
    assert_eq!(transport.call_count(), 2, "a GET may be repeated freely");
}

#[tokio::test]
async fn a_429_on_the_read_is_retried() {
    let transport = MockTransport::new();
    transport.push_response(response(
        429,
        &[],
        b"{\"error\":{\"code\":429,\"message\":\"rate limit\"}}",
    ));
    transport.push_response(json_response(&envelope(
        &json!({"id": WAMID, "status": "read"}),
    )));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Read);
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn a_read_that_loses_the_answer_is_retried() {
    let transport = MockTransport::new();
    transport.push(Reply::after_write("connection reset"));
    transport.push_response(json_response(&envelope(
        &json!({"id": WAMID, "status": "sent"}),
    )));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let status = client.message_status(WAMID).await.unwrap();

    assert_eq!(status.status, DeliveryStatus::Sent);
    assert_eq!(transport.call_count(), 2);
    assert!(
        !status.status.as_str().contains(' '),
        "the parsed status is the wire word"
    );
}

#[tokio::test]
async fn a_4xx_on_the_read_is_terminal_and_never_ambiguous() {
    let transport = MockTransport::new();
    transport.push_response(response(
        404,
        &[],
        b"{\"error\":{\"code\":100,\"message\":\"id not found\"}}",
    ));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.message_status(WAMID).await.unwrap_err();

    assert!(matches!(err, Error::Api { status: 404, .. }));
    assert!(!err.is_ambiguous(), "a read cannot have side effects");
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn a_body_without_entries_is_a_payload_error() {
    let (client, _transport) =
        client_answering(&json!({"messaging_product": "whatsapp", "messages": []}));

    let err = client.message_status(WAMID).await.unwrap_err();
    assert!(matches!(err, Error::Payload(_)), "{err:?}");
}

#[test]
fn the_new_surface_is_send_and_sync_and_reexported() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MessageStatus>();
    assert_send_sync::<DeliveryStatus>();
    assert_send_sync::<Vec<DeliveryStatus>>();

    // The root re-exports are the same types as the module's.
    let root: meta_whatsapp::MessageStatus = MessageStatus::default();
    let module: MessageStatus = root;
    assert_eq!(module.status, DeliveryStatus::Unknown(String::new()));
    let root_check: fn(&str) -> Result<(), Error> = meta_whatsapp::validate_wamid;
    assert!(root_check(WAMID).is_ok());
    assert_eq!(meta_whatsapp::MAX_WAMID_LEN, MAX_WAMID_LEN);
    assert_eq!(
        meta_whatsapp::DeliveryStatus::parse("delivered"),
        DeliveryStatus::Delivered
    );
}
