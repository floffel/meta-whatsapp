//! `WhatsApp` Flows: what the outbound flow message puts on the wire, what the
//! parser makes of the inbound `nfm_reply`, and the boundary of every limit in
//! between.

mod common;

use common::{client, json_response, Body, Call, MockTransport};
use meta_whatsapp::flow::{
    validate_flow_id, validate_screen_data, validate_screen_name, Flow,
    FlowEncryptionPublicKeyField, FlowEncryptionPublicKeyRequest, FlowEncryptionPublicKeyResponse,
    FlowGetRequest, FlowPreviewRequest, FlowPublicKeySignatureStatus, FlowStatus,
    FLOW_CTA_MAX_CHARS, FLOW_ID_MAX_BYTES, FLOW_MESSAGE_VERSION, FLOW_SCREEN_MAX_CHARS,
    FLOW_TOKEN_MAX_BYTES,
};
use meta_whatsapp::transport::Method;
use meta_whatsapp::{
    Error, FlowAction, FlowMessage, FlowReply, MessageRequest, WebhookPayload, WebhookVerifier,
};
use serde_json::{json, Map, Value};

const TOKEN_FIELD: &str = "interactive.action.parameters.flow_token";
const CTA_FIELD: &str = "interactive.action.parameters.flow_cta";
const SCREEN_FIELD: &str = "interactive.action.parameters.flow_action_payload.screen";
const DATA_FIELD: &str = "interactive.action.parameters.flow_action_payload.data";
const VERSION_FIELD: &str = "interactive.action.parameters.flow_message_version";

fn json_body(call: &Call) -> &Value {
    match &call.body {
        Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

/// Renders a flow message against a throwaway recipient.
fn render(flow: FlowMessage) -> Value {
    MessageRequest::new("4915112345678", flow)
        .to_json()
        .expect("the message must render")
}

/// The field named by a [`Error::Validation`], for a message that must be
/// refused.
#[track_caller]
fn rejected(flow: FlowMessage) -> &'static str {
    match MessageRequest::new("4915112345678", flow).to_json() {
        Err(Error::Validation { field, .. }) => field,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

/// A signed `nfm_reply` webhook body whose `response_json` is `response`.
fn reply_body(response: &Value) -> String {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "102290129340398",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "15550001234",
                        "phone_number_id": "106540352242922"
                    },
                    "contacts": [{ "wa_id": "4915112345678", "profile": { "name": "Ada" } }],
                    "messages": [{
                        "id": "wamid.reply",
                        "from": "4915112345678",
                        "timestamp": "1793000000",
                        "type": "interactive",
                        "interactive": {
                            "type": "nfm_reply",
                            "nfm_reply": {
                                "name": "flow",
                                "body": "Sent",
                                "response_json": response.to_string()
                            }
                        }
                    }]
                }
            }]
        }]
    })
    .to_string()
}

fn parse(body: &str) -> WebhookPayload {
    let verifier = WebhookVerifier::new("secret").expect("secret");
    verifier
        .verify_and_parse(body.as_bytes(), &verifier.sign(body.as_bytes()))
        .expect("the body must parse")
}

// ---------------------------------------------------------------------------
// Outbound
// ---------------------------------------------------------------------------

#[test]
fn a_data_exchange_message_carries_no_action_payload() {
    let json = render(
        FlowMessage::new("flow-1", "Termin buchen", FlowAction::DataExchange)
            .flow_token("order-42")
            .body("Wählen Sie einen Termin"),
    );

    let parameters = &json["interactive"]["action"]["parameters"];
    assert_eq!(json["interactive"]["type"], "flow");
    assert_eq!(json["interactive"]["action"]["name"], "flow");
    assert_eq!(
        json["interactive"]["body"]["text"],
        "Wählen Sie einen Termin"
    );
    assert_eq!(parameters["flow_id"], "flow-1");
    assert_eq!(parameters["flow_token"], "order-42");
    assert_eq!(parameters["flow_cta"], "Termin buchen");
    assert_eq!(parameters["flow_action"], "data_exchange");
    assert_eq!(parameters["flow_message_version"], FLOW_MESSAGE_VERSION);
    assert_eq!(parameters["mode"], "published");
    assert!(
        parameters.get("flow_action_payload").is_none(),
        "data_exchange must not send flow_action_payload"
    );
}

#[test]
fn a_navigate_message_carries_the_screen_and_its_data() {
    let json = render(FlowMessage::new(
        "flow-1",
        "Weiter",
        FlowAction::navigate_with("CHECKOUT", json!({"zip": "01067", "items": 2})),
    ));

    let payload = &json["interactive"]["action"]["parameters"]["flow_action_payload"];
    assert_eq!(payload["screen"], "CHECKOUT");
    assert_eq!(payload["data"]["zip"], "01067");
}

#[test]
fn an_empty_token_is_left_off_the_wire() {
    let json = render(FlowMessage::new(
        "flow-1",
        "Weiter",
        FlowAction::DataExchange,
    ));
    let parameters = &json["interactive"]["action"]["parameters"];
    assert!(
        parameters.get("flow_token").is_none(),
        "Meta applies its own \"unused\" default; \"\" is not a token"
    );
}

#[test]
fn the_message_version_defaults_to_the_documented_one_and_can_be_overridden() {
    let default = render(FlowMessage::new("f", "c", FlowAction::DataExchange));
    assert_eq!(
        default["interactive"]["action"]["parameters"]["flow_message_version"],
        FLOW_MESSAGE_VERSION
    );

    let overridden =
        render(FlowMessage::new("f", "c", FlowAction::DataExchange).flow_message_version("4"));
    assert_eq!(
        overridden["interactive"]["action"]["parameters"]["flow_message_version"],
        "4"
    );
}

#[test]
fn header_and_footer_render_around_the_body() {
    let json = render(
        FlowMessage::new("f", "c", FlowAction::DataExchange)
            .header(meta_whatsapp::HeaderText::new("Termin"))
            .footer("Antworten Sie nicht auf diese Nachricht"),
    );
    assert_eq!(json["interactive"]["header"]["text"], "Termin");
    assert_eq!(
        json["interactive"]["footer"]["text"],
        "Antworten Sie nicht auf diese Nachricht"
    );
}

// ---------------------------------------------------------------------------
// Boundary matrix: just valid, just invalid, for every limit.
// ---------------------------------------------------------------------------

#[test]
fn flow_token_boundary_is_1024_bytes() {
    let longest = "t".repeat(FLOW_TOKEN_MAX_BYTES);
    let accepted =
        render(FlowMessage::new("f", "c", FlowAction::DataExchange).flow_token(longest.clone()));
    assert_eq!(
        accepted["interactive"]["action"]["parameters"]["flow_token"]
            .as_str()
            .map(str::len),
        Some(FLOW_TOKEN_MAX_BYTES)
    );

    assert_eq!(
        rejected(
            FlowMessage::new("f", "c", FlowAction::DataExchange)
                .flow_token("t".repeat(FLOW_TOKEN_MAX_BYTES + 1))
        ),
        TOKEN_FIELD
    );
}

#[test]
fn flow_cta_boundary_is_30_characters_not_bytes() {
    // 30 characters, 60 bytes: counted as characters, so accepted.
    let umlauts = "ä".repeat(FLOW_CTA_MAX_CHARS);
    assert_eq!(umlauts.len(), 60);
    let accepted = render(FlowMessage::new(
        "f",
        umlauts.clone(),
        FlowAction::DataExchange,
    ));
    assert_eq!(
        accepted["interactive"]["action"]["parameters"]["flow_cta"],
        umlauts.as_str()
    );

    assert_eq!(
        rejected(FlowMessage::new(
            "f",
            "ä".repeat(31),
            FlowAction::DataExchange
        )),
        CTA_FIELD
    );
    // An empty CTA is not "optional": the schema lists flow_cta as required.
    assert_eq!(
        rejected(FlowMessage::new("f", "", FlowAction::DataExchange)),
        CTA_FIELD
    );
}

#[test]
fn screen_name_boundary_is_64_characters_and_an_identifier_alphabet() {
    let longest = "A".repeat(FLOW_SCREEN_MAX_CHARS);
    assert!(validate_screen_name(&longest).is_ok());
    assert!(validate_screen_name(&"A".repeat(FLOW_SCREEN_MAX_CHARS + 1)).is_err());

    assert!(validate_screen_name("WELCOME_SCREEN").is_ok());
    assert!(validate_screen_name("screen-2").is_ok());
    assert!(validate_screen_name("screen_2").is_ok());
    for bad in ["", "WELCOME SCREEN", "SCREEN.1", "SCREEN/1", "BILDSCHIRM_Ä"] {
        assert!(
            validate_screen_name(bad).is_err(),
            "{bad:?} must not pass as a screen id"
        );
    }

    // The same rule through the message that carries it.
    assert_eq!(
        rejected(FlowMessage::new(
            "f",
            "c",
            FlowAction::navigate("WELCOME SCREEN")
        )),
        SCREEN_FIELD
    );
}

#[test]
fn screen_data_must_be_a_non_empty_object() {
    assert!(validate_screen_data(&json!({"a": 1})).is_ok());
    assert!(validate_screen_data(&json!({"nested": {"a": [1, 2]}})).is_ok());

    assert!(
        validate_screen_data(&json!({})).is_err(),
        "the schema forbids an empty object"
    );
    assert!(validate_screen_data(&json!("a string")).is_err());
    assert!(validate_screen_data(&json!([1, 2])).is_err());
    assert!(validate_screen_data(&Value::Null).is_err());

    assert_eq!(
        rejected(FlowMessage::new(
            "f",
            "c",
            FlowAction::navigate_with("S", json!({}))
        )),
        DATA_FIELD
    );
    assert_eq!(
        rejected(FlowMessage::new(
            "f",
            "c",
            FlowAction::navigate_with("S", json!("data"))
        )),
        DATA_FIELD
    );
}

#[test]
fn empty_ids_and_versions_are_refused() {
    assert_eq!(
        rejected(FlowMessage::new("", "c", FlowAction::DataExchange)),
        "interactive.action.parameters.flow_id"
    );
    assert_eq!(
        rejected(FlowMessage::new("f", "c", FlowAction::DataExchange).flow_message_version("")),
        VERSION_FIELD
    );
    assert_eq!(
        rejected(FlowMessage::new("f", "c", FlowAction::navigate(""))),
        SCREEN_FIELD
    );
}

// ---------------------------------------------------------------------------
// Inbound
// ---------------------------------------------------------------------------

#[test]
fn the_documented_nfm_reply_decodes_its_stringified_response() {
    let body = reply_body(&json!({
        "flow_token": "order-42",
        "screen": "SUMMARY",
        "data": {"slot": "2026-11-03T09:00", "name": "Ada"}
    }));
    let payload = parse(&body);
    let mut inbound = payload.inbound_messages();
    let (phone_number_id, message) = inbound.next().expect("one inbound message");
    assert_eq!(phone_number_id, "106540352242922");
    assert!(inbound.next().is_none());

    let reply = message.flow_reply().expect("one flow reply");
    assert_eq!(reply.name.as_deref(), Some("flow"));
    assert_eq!(reply.body.as_deref(), Some("Sent"));
    // `response_json` stays the string Meta sent; decoding is explicit.
    assert!(reply
        .response_json
        .as_deref()
        .is_some_and(|raw| raw.starts_with('{')));

    let response = reply.response().expect("a valid response document");
    assert_eq!(response.flow_token(), Some("order-42"));
    assert_eq!(response.field("screen"), Some("SUMMARY"));
    assert_eq!(response.field("missing"), None);
    assert_eq!(
        response.get("data"),
        Some(&json!({"slot": "2026-11-03T09:00", "name": "Ada"}))
    );
    assert_eq!(
        response
            .get("data")
            .and_then(Value::as_object)
            .map(Map::len),
        Some(2)
    );
}

#[test]
fn a_response_json_that_is_not_json_is_kept_instead_of_dropped() {
    let reply: FlowReply = serde_json::from_str(r#"{"name":"flow","response_json":"{broken"}"#)
        .expect("a reply decodes whatever the string holds");
    assert_eq!(reply.response_json.as_deref(), Some("{broken"));
    assert!(
        reply.response().is_err(),
        "a broken document must be reported, not silently dropped"
    );
}

#[test]
fn a_reply_without_a_response_document_still_parses() {
    let reply: FlowReply =
        serde_json::from_str(r#"{"name":"flow","body":"Sent"}"#).expect("a reply decodes");
    assert_eq!(reply.response_json, None);
    assert_eq!(reply.name.as_deref(), Some("flow"));
    assert!(reply.response().is_err());
}

#[test]
fn unknown_keys_survive_and_a_reply_serialises_back_to_its_wire_shape() {
    let nfm = json!({
        "name": "flow",
        "body": "Sent",
        "response_json": "{\"screen\":\"SUMMARY\"}",
        "future_key": {"nested": true}
    });
    let reply: FlowReply = serde_json::from_value(nfm.clone()).expect("a reply decodes");
    assert_eq!(
        reply.extra.get("future_key"),
        Some(&json!({"nested": true})),
        "an unmodelled key was dropped"
    );
    assert_eq!(
        serde_json::to_value(&reply).expect("a reply serialises"),
        nfm,
        "a parsed webhook must be forwardable unchanged"
    );
}

#[test]
fn a_message_without_a_flow_reply_reports_none() {
    let body = json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "1",
            "changes": [{
                "field": "messages",
                "value": {
                    "metadata": {"display_phone_number": "1", "phone_number_id": "2"},
                    "messages": [
                        {"id": "m1", "from": "3", "timestamp": "1", "type": "text",
                         "text": {"body": "Hallo"}},
                        {"id": "m2", "from": "3", "timestamp": "2", "type": "interactive",
                         "interactive": {"type": "button_reply",
                                         "button_reply": {"id": "yes", "title": "Ja"}}}
                    ]
                }
            }]
        }]
    })
    .to_string();

    let payload = parse(&body);
    let messages: Vec<_> = payload.inbound_messages().collect();
    assert_eq!(messages.len(), 2);
    for (_, message) in messages {
        assert!(message.flow_reply().is_none());
    }
}

// ---------------------------------------------------------------------------
// Round trip: the token sent out is the token that comes back.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_flow_round_trips_from_send_to_reply() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    client
        .send(&MessageRequest::new(
            "4915112345678",
            FlowMessage::new("flow-1", "Termin buchen", FlowAction::DataExchange)
                .flow_token("order-42"),
        ))
        .await
        .expect("the message is accepted");

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert!(
        call.url.ends_with("/106540352242922/messages"),
        "{}",
        call.url
    );
    let sent_token = json_body(&call)["interactive"]["action"]["parameters"]["flow_token"]
        .as_str()
        .map(str::to_string);
    assert_eq!(sent_token.as_deref(), Some("order-42"));

    // The customer finishes the flow; the screen's final payload comes back as
    // `response_json`, token included.
    let body = reply_body(&json!({
        "flow_token": sent_token,
        "screen": "SUMMARY",
        "data": {"slot": "2026-11-03T09:00"}
    }));
    let payload = parse(&body);

    let reply = payload
        .inbound_messages()
        .find_map(|(_, message)| message.flow_reply())
        .expect("one flow reply");
    let response = reply.response().expect("a valid response document");
    assert_eq!(response.flow_token().map(str::to_string), sent_token);
    assert_eq!(response.field("screen"), Some("SUMMARY"));
    assert_eq!(
        response.get("data").and_then(|data| data.get("slot")),
        Some(&json!("2026-11-03T09:00"))
    );
}

// ---------------------------------------------------------------------------
// Flow get / preview: the read side of the API.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flow_get_records_the_default_path_and_parses_a_sparse_response() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({"id": "flow-1"})));
    let client = client(transport.clone());

    let flow = client.flow(&FlowGetRequest::new("flow-1")).await.unwrap();
    assert_eq!(flow.id.as_deref(), Some("flow-1"));
    assert!(flow.name.is_none());
    assert_eq!(flow.validation_errors, []);

    let call = transport.only_call();
    assert_eq!(call.method, Method::Get);
    assert_eq!(call.url, "https://graph.facebook.com/v21.0/flow-1");
    assert_eq!(call.body, Body::Empty);
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One full documented response keeps its assertions together.
async fn flow_get_encodes_nested_fields_and_decodes_the_full_documented_shape() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "id": "flow-1",
        "name": "Appointments",
        "status": "PAUSED",
        "categories": ["APPOINTMENT_BOOKING"],
        "json_version": "5.0",
        "data_api_version": "3.0",
        "data_channel_uri": "https://example.test/data",
        "endpoint_uri": "https://example.test/flow",
        "preview": {
            "preview_url": "https://business.facebook.com/wa/manage/flows/1/preview/?token=t",
            "expires_at": 1_735_689_600_u64
        },
        "validation_errors": [{
            "error": "INVALID_PROPERTY",
            "error_type": "JSON_SCHEMA_ERROR",
            "message": "The property is not allowed",
            "line_start": 46,
            "line_end": 46,
            "column_start": 17,
            "column_end": 30,
            "pointers": [{
                "line_start": 46,
                "column_start": 17,
                "path": "screens [0].layout.children [0].type"
            }]
        }, "legacy scalar detail"],
        "health_status": {
            "can_send_message": "BLOCKED",
            "entities": [{
                "id": "waba-1",
                "entity_type": "WABA",
                "can_send_message": "BLOCKED",
                "errors": [{
                    "error_code": 141_006,
                    "error_description": "The payment method is unavailable.",
                    "possible_solution": "Add a payment method."
                }]
            }]
        },
        "whatsapp_business_account": {
            "id": "waba-1",
            "name": "Example WABA",
            "business_type": "ent",
            "message_template_namespace": "namespace-1",
            "timezone_id": "54"
        },
        "application": {
            "id": "app-1",
            "name": "Example App",
            "link": "https://example.test/app"
        },
        "metric": {"name": "ENDPOINT_REQUEST_COUNT"}
    })));
    let client = client(transport.clone());
    let query = FlowGetRequest::new("flow-1")
        .fields(["id", "name", "health_status.phone_number(123)", "preview"])
        .date_format("U");

    let flow = client.flow(&query).await.unwrap();
    assert_eq!(flow.name.as_deref(), Some("Appointments"));
    assert_eq!(flow.status, Some(FlowStatus::Unknown("PAUSED".into())));
    assert_eq!(flow.categories, ["APPOINTMENT_BOOKING"]);
    assert_eq!(flow.json_version.as_deref(), Some("5.0"));
    assert_eq!(flow.data_api_version.as_deref(), Some("3.0"));
    assert_eq!(
        flow.data_channel_uri.as_deref(),
        Some("https://example.test/data")
    );
    assert_eq!(
        flow.endpoint_uri.as_deref(),
        Some("https://example.test/flow")
    );
    assert_eq!(flow.extra["metric"]["name"], "ENDPOINT_REQUEST_COUNT");

    let preview = flow.preview.as_ref().unwrap();
    assert!(preview.preview_url.as_deref().unwrap().contains("token=t"));
    assert_eq!(preview.expires_at_secs(), Some(1_735_689_600));

    let validation = &flow.validation_errors[0];
    assert_eq!(validation.line_start, Some(46));
    assert_eq!(validation.column_end, Some(30));
    assert_eq!(
        validation.pointers[0].path.as_deref(),
        Some("screens [0].layout.children [0].type")
    );
    assert_eq!(
        flow.validation_errors[1].extra["value"],
        "legacy scalar detail"
    );

    let health = flow.health_status.as_ref().unwrap();
    assert_eq!(health.can_send_message.as_deref(), Some("BLOCKED"));
    assert_eq!(health.entities[0].entity_type.as_deref(), Some("WABA"));
    assert_eq!(
        health.entities[0].errors[0]
            .error_code
            .as_ref()
            .and_then(serde_json::Number::as_u64),
        Some(141_006)
    );
    assert_eq!(
        flow.whatsapp_business_account
            .as_ref()
            .unwrap()
            .timezone_id
            .as_deref(),
        Some("54")
    );
    assert_eq!(
        flow.application.as_ref().unwrap().name.as_deref(),
        Some("Example App")
    );

    let call = transport.only_call();
    assert_eq!(call.method, Method::Get);
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/flow-1?fields=id%2Cname%2Chealth_status.phone_number%28123%29%2Cpreview&date_format=U"
    );
}

#[tokio::test]
async fn flow_preview_keeps_or_invalidates_the_link_as_requested() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "id": "flow-1",
        "preview": {
            "preview_url": "https://business.facebook.com/wa/manage/flows/1/preview/?token=a",
            "expires_at": "2026-12-06T00:00:00+0000"
        }
    })));
    transport.push_response(json_response(&json!({
        "id": "flow-1",
        "preview": {
            "preview_url": "https://business.facebook.com/wa/manage/flows/1/preview/?token=b",
            "expires_at": 1_798_848_000_u64
        },
        "field_added_later": true
    })));
    let client = client(transport.clone());

    let kept = client
        .flow_preview(&FlowPreviewRequest::new("flow-1"))
        .await
        .unwrap();
    assert!(kept
        .preview
        .as_ref()
        .unwrap()
        .preview_url
        .as_deref()
        .unwrap()
        .ends_with("token=a"));
    assert_eq!(kept.preview.as_ref().unwrap().expires_at_secs(), None);

    let refreshed = client
        .flow_preview(
            &FlowPreviewRequest::new("flow-1")
                .invalidate(true)
                .date_format("U"),
        )
        .await
        .unwrap();
    assert!(refreshed
        .preview
        .as_ref()
        .unwrap()
        .preview_url
        .as_deref()
        .unwrap()
        .ends_with("token=b"));
    assert_eq!(
        refreshed.preview.as_ref().unwrap().expires_at_secs(),
        Some(1_798_848_000)
    );
    assert_eq!(refreshed.extra["field_added_later"], true);

    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].url,
        "https://graph.facebook.com/v21.0/flow-1?fields=preview.invalidate%28false%29"
    );
    assert_eq!(
        calls[1].url,
        "https://graph.facebook.com/v21.0/flow-1?fields=preview.invalidate%28true%29&date_format=U"
    );
    assert!(calls
        .iter()
        .all(|call| call.method == Method::Get && call.body == Body::Empty));
}

// ---------------------------------------------------------------------------
// Flow data-exchange public key.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flow_encryption_public_key_records_the_flat_documented_round_trip() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "business_public_key": "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----",
        "business_public_key_signature_status": "VALID",
        "field_added_later": true
    })));
    let client = client(transport.clone());

    let response = client
        .flow_encryption_public_key(&FlowEncryptionPublicKeyRequest::new("106540352242922"))
        .await
        .unwrap();

    assert_eq!(
        response.business_public_key.as_deref(),
        Some("-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----")
    );
    assert_eq!(
        response.business_public_key_signature_status,
        Some(FlowPublicKeySignatureStatus::Valid)
    );
    assert_eq!(response.extra["field_added_later"], true);

    let call = transport.only_call();
    assert_eq!(call.method, Method::Get);
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/whatsapp_business_encryption"
    );
    assert_eq!(call.body, Body::Empty);
}

#[tokio::test]
async fn flow_encryption_public_key_accepts_the_openapi_envelope_and_encodes_fields() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "data": [{
            "business_public_key": "-----BEGIN PUBLIC KEY-----\nMIIB\n-----END PUBLIC KEY-----",
            "business_public_key_signature_status": "MISMATCH",
            "new_member": 42
        }]
    })));
    let client = client(transport.clone());
    let request = FlowEncryptionPublicKeyRequest::new("106540352242922").fields([
        FlowEncryptionPublicKeyField::BusinessPublicKey,
        FlowEncryptionPublicKeyField::SignatureStatus,
    ]);

    let response = client.flow_encryption_public_key(&request).await.unwrap();
    assert_eq!(
        response.business_public_key_signature_status,
        Some(FlowPublicKeySignatureStatus::Mismatch)
    );
    assert_eq!(response.extra["new_member"], 42);

    let call = transport.only_call();
    assert_eq!(call.method, Method::Get);
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/whatsapp_business_encryption?fields=business_public_key%2Cbusiness_public_key_signature_status"
    );
    assert_eq!(call.body, Body::Empty);
}

#[test]
fn public_key_signature_statuses_are_typed_but_forward_compatible() {
    for (raw, expected) in [
        ("VALID", FlowPublicKeySignatureStatus::Valid),
        (" mismatch ", FlowPublicKeySignatureStatus::Mismatch),
        (
            "FUTURE_STATUS",
            FlowPublicKeySignatureStatus::Unknown("FUTURE_STATUS".into()),
        ),
    ] {
        assert_eq!(FlowPublicKeySignatureStatus::parse(raw), expected);
    }
}

#[test]
fn public_key_response_parsing_accepts_both_documented_envelopes() {
    let flat = FlowEncryptionPublicKeyResponse::parse(&json!({
        "business_public_key_signature_status": "FUTURE_STATUS"
    }))
    .unwrap();
    assert_eq!(
        flat.business_public_key_signature_status,
        Some(FlowPublicKeySignatureStatus::Unknown(
            "FUTURE_STATUS".into()
        ))
    );

    let wrapped = FlowEncryptionPublicKeyResponse::parse(&json!({
        "data": [{"business_public_key": "key"}]
    }))
    .unwrap();
    assert_eq!(wrapped.business_public_key.as_deref(), Some("key"));
    assert!(wrapped.business_public_key_signature_status.is_none());
}

#[test]
fn public_key_response_errors_name_the_unexpected_envelope_or_member_type() {
    for malformed in [
        json!("not an object"),
        json!({"data": {}}),
        json!({"data": []}),
        json!({"data": [42]}),
    ] {
        assert!(
            matches!(
                FlowEncryptionPublicKeyResponse::parse(&malformed),
                Err(Error::Payload(_))
            ),
            "{malformed} was not rejected as a payload"
        );
    }

    for malformed in [
        json!({"business_public_key": 42}),
        json!({"business_public_key_signature_status": 42}),
    ] {
        let error = FlowEncryptionPublicKeyResponse::parse(&malformed).unwrap_err();
        assert!(matches!(error, Error::Json { .. }));
        assert!(std::error::Error::source(&error).is_some());
    }
}

#[tokio::test]
async fn public_key_phone_id_boundary_is_32_digits() {
    let accepted = "1".repeat(32);
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({})));
    let accepted_client = client(transport.clone());

    accepted_client
        .flow_encryption_public_key(&FlowEncryptionPublicKeyRequest::new(&accepted))
        .await
        .unwrap();
    let call = transport.only_call();
    assert!(call
        .url
        .contains(&format!("/{accepted}/whatsapp_business_encryption")));

    let rejected_transport = MockTransport::new();
    let rejected_client = client(rejected_transport.clone());
    for phone_number_id in [
        String::new(),
        format!("{accepted}1"),
        "../me".into(),
        "1234x".into(),
        "１２３４".into(),
    ] {
        let error = rejected_client
            .flow_encryption_public_key(&FlowEncryptionPublicKeyRequest::new(phone_number_id))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                field: "phone_number_id",
                ..
            }
        ));
    }
    assert_eq!(rejected_transport.call_count(), 0);
}

#[test]
fn flow_id_boundary_is_256_bytes_and_the_alphabet_is_path_safe() {
    let longest = "a".repeat(FLOW_ID_MAX_BYTES);
    assert!(validate_flow_id(&longest).is_ok());
    assert!(validate_flow_id(&format!("{longest}a")).is_err());
    assert!(validate_flow_id("A_b-9").is_ok());

    for bad in [
        "",
        "../me",
        "flow/1",
        "flow?fields=id",
        "flow#x",
        "flow id",
        "flow.1",
        "flow:1",
    ] {
        assert!(
            validate_flow_id(bad).is_err(),
            "{bad:?} was accepted as a Flow id"
        );
    }
}

#[tokio::test]
async fn invalid_flow_requests_never_reach_the_transport() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    for flow_id in [
        String::new(),
        "../me".to_string(),
        "x".repeat(FLOW_ID_MAX_BYTES + 1),
    ] {
        for result in [
            client
                .flow(&FlowGetRequest::new(&flow_id))
                .await
                .map(|_| ()),
            client
                .flow_preview(&FlowPreviewRequest::new(&flow_id))
                .await
                .map(|_| ()),
        ] {
            match result {
                Err(Error::Validation { field, .. }) => assert_eq!(field, "flow_id"),
                other => panic!("Flow id {flow_id:?} was not rejected: {other:?}"),
            }
        }
    }

    for fields in [
        vec![""],
        vec!["id&limit=1"],
        vec!["Name"],
        vec!["id;limit=1"],
    ] {
        let error = client
            .flow(&FlowGetRequest::new("flow-1").fields(fields))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                field: "fields",
                ..
            }
        ));
    }

    for format in ["", "\u{7f}"] {
        let error = client
            .flow(&FlowGetRequest::new("flow-1").date_format(format))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                field: "date_format",
                ..
            }
        ));
        let error = client
            .flow_preview(&FlowPreviewRequest::new("flow-1").date_format(format))
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::Validation {
                field: "date_format",
                ..
            }
        ));
    }

    assert_eq!(transport.call_count(), 0);
}

#[test]
fn flow_parsing_is_permissive_but_retains_serde_errors() {
    let flow = Flow::parse(&json!({
        "status": "FUTURE_STATUS",
        "validation_errors": [42],
        "health_status": {"entities": [{"future": true}]}
    }))
    .unwrap();
    assert_eq!(
        flow.status,
        Some(FlowStatus::Unknown("FUTURE_STATUS".into()))
    );
    assert_eq!(flow.validation_errors[0].extra["value"], 42);
    assert_eq!(
        flow.health_status.unwrap().entities[0].extra["future"],
        true
    );

    let error = Flow::parse(&json!({"categories": "not an array"})).unwrap_err();
    assert!(matches!(error, Error::Json { .. }));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn every_documented_flow_status_is_typed() {
    for (raw, expected) in [
        ("DRAFT", FlowStatus::Draft),
        ("PUBLISHED", FlowStatus::Published),
        ("DEPRECATED", FlowStatus::Deprecated),
        ("BLOCKED", FlowStatus::Blocked),
        ("THROTTLED", FlowStatus::Throttled),
    ] {
        let status = FlowStatus::parse(raw);
        assert_eq!(status, expected);
        assert_eq!(status.as_str(), raw);
    }
}
