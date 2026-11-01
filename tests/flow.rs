//! `WhatsApp` Flows: what the outbound flow message puts on the wire, what the
//! parser makes of the inbound `nfm_reply`, and the boundary of every limit in
//! between.

mod common;

use common::{client, Body, Call, MockTransport};
use meta_whatsapp::flow::{
    validate_screen_data, validate_screen_name, FLOW_CTA_MAX_CHARS, FLOW_MESSAGE_VERSION,
    FLOW_SCREEN_MAX_CHARS, FLOW_TOKEN_MAX_BYTES,
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


