//! Flow completion replies (`nfm_reply`): the documented wire shape, the
//! decoded `response_json`, and the boundary cases around it.
//!
//! The fixture is the example from Meta's flows-webhooks reference
//! (`https://developers.facebook.com/docs/whatsapp/flows/reference/flowswebhooks/`),
//! which documents exactly three string fields: `name`, `body`, `response_json`.

use std::time::{SystemTime, UNIX_EPOCH};

use meta_whatsapp::{
    Error, FlowAction, FlowMessage, FlowReply, Message, MessageRequest, WebhookPayload,
    WebhookVerifier,
};
use proptest::prelude::*;
use serde_json::{json, Map, Value};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default()
}

/// The webhook envelope Meta posts for a completed flow, with the documented
/// `nfm_reply` block.
fn nfm_payload(token: &str, timestamp: u64) -> Vec<u8> {
    let response_json = json!({ "flow_token": token, "city": "Berlin" }).to_string();
    json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "1",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {"display_phone_number": "15550001234", "phone_number_id": "2"},
                    "messages": [{
                        "from": "4915112345678",
                        "id": "wamid.1",
                        "timestamp": timestamp.to_string(),
                        "type": "interactive",
                        "interactive": {
                            "type": "nfm_reply",
                            "nfm_reply": {
                                "name": "flow",
                                "body": "Sent",
                                "response_json": response_json,
                            }
                        }
                    }]
                }
            }]
        }]
    })
    .to_string()
    .into_bytes()
}

fn parse(body: &[u8]) -> WebhookPayload {
    let verifier = WebhookVerifier::new("secret").unwrap();
    let header = verifier.sign(body);
    verifier.verify_and_parse(body, &header).unwrap()
}

fn reply_from(raw: &str) -> FlowReply {
    serde_json::from_value(json!({ "response_json": raw })).unwrap()
}

// ---------------------------------------------------------------------------
// The documented shape
// ---------------------------------------------------------------------------

#[test]
fn documented_nfm_reply_shape_parses() {
    let payload = parse(&nfm_payload("tok-1", now()));
    let (phone_number_id, message) = payload.inbound_messages().next().unwrap();
    assert_eq!(phone_number_id, "2");
    assert_eq!(message.message_type, "interactive");

    let interactive = message.interactive.as_ref().unwrap();
    assert_eq!(interactive.reply_type, "nfm_reply");
    assert!(interactive.button_reply.is_none());

    let reply = interactive.nfm_reply.as_ref().unwrap();
    assert_eq!(reply.name.as_deref(), Some("flow"));
    assert_eq!(reply.body.as_deref(), Some("Sent"));
    assert_eq!(
        reply.response_json.as_deref(),
        Some(r#"{"city":"Berlin","flow_token":"tok-1"}"#)
    );

    let response = reply.response().unwrap();
    assert_eq!(response.flow_token(), Some("tok-1"));
    assert_eq!(response.field("city"), Some("Berlin"));
    assert_eq!(response.get("city"), Some(&json!("Berlin")));
    assert_eq!(response.get("missing"), None);
}

#[test]
fn message_helper_returns_the_reply_and_ignores_other_types() {
    let payload = parse(&nfm_payload("tok-2", now()));
    let (_, message) = payload.inbound_messages().next().unwrap();
    assert!(message.flow_reply().is_some());

    let text: Message =
        serde_json::from_str(r#"{"id":"1","from":"2","type":"text","text":{"body":"hi"}}"#)
            .unwrap();
    assert!(text.flow_reply().is_none());

    let button: Message = serde_json::from_str(
        r#"{"id":"1","from":"2","type":"interactive","interactive":{"type":"button_reply",
            "button_reply":{"id":"yes","title":"Ja"}}}"#,
    )
    .unwrap();
    assert!(button.flow_reply().is_none());
}

#[test]
fn outbound_flow_message_round_trips_into_the_typed_reply() {
    let outbound = MessageRequest::new(
        "4915112345678",
        FlowMessage::new("flow-123", "Buchen", FlowAction::navigate("WELCOME"))
            .flow_token("tok-42"),
    )
    .to_json()
    .unwrap();
    let parameters = &outbound["interactive"]["action"]["parameters"];
    assert_eq!(parameters["flow_id"], "flow-123");
    assert_eq!(parameters["flow_message_version"], "3");
    let token = parameters["flow_token"].as_str().unwrap();

    let payload = parse(&nfm_payload(token, now()));
    let (_, message) = payload.inbound_messages().next().unwrap();
    let response = message.flow_reply().unwrap().response().unwrap();
    assert_eq!(response.flow_token(), Some(token));
    assert_eq!(response.field("city"), Some("Berlin"));
}

// ---------------------------------------------------------------------------
// Boundaries around response_json
// ---------------------------------------------------------------------------

#[test]
fn a_missing_response_json_is_reported() {
    let reply: FlowReply = serde_json::from_str(r#"{"name":"flow","body":"Sent"}"#).unwrap();
    let error = reply.response().unwrap_err();
    assert!(matches!(&error, Error::Payload(_)), "got {error:?}");
    assert!(error
        .to_string()
        .contains("nfm_reply.response_json is missing"));
}

#[test]
fn an_empty_or_truncated_response_json_is_reported() {
    for raw in ["", "{", "{\"flow_token\":}", "not json"] {
        let error = reply_from(raw).response().unwrap_err();
        assert!(matches!(&error, Error::Payload(_)), "raw {raw:?}");
        assert!(
            error.to_string().contains("not valid JSON"),
            "raw {raw:?} gave {error}"
        );
    }
}

#[test]
fn a_non_object_response_carries_no_fields() {
    for raw in ["null", "[1,2]", "\"text\"", "7"] {
        let response = reply_from(raw).response().unwrap();
        assert_eq!(response.get("flow_token"), None, "raw {raw:?}");
        assert_eq!(response.flow_token(), None, "raw {raw:?}");
        assert_eq!(response.field("anything"), None, "raw {raw:?}");
        assert!(!response.as_value().is_object(), "raw {raw:?}");
    }
}

#[test]
fn non_string_values_are_reachable_through_get_only() {
    let response = reply_from(r#"{"qty":3,"items":[1,2],"flag":true}"#)
        .response()
        .unwrap();
    assert_eq!(response.field("qty"), None);
    assert_eq!(response.get("qty"), Some(&json!(3)));
    assert_eq!(response.get("items"), Some(&json!([1, 2])));
    assert_eq!(response.get("flag"), Some(&json!(true)));
    assert_eq!(response.flow_token(), None);
}

#[test]
fn an_empty_object_response_is_valid_and_empty() {
    let response = reply_from("{}").response().unwrap();
    assert_eq!(response.as_value(), &json!({}));
    assert_eq!(response.flow_token(), None);
}

// ---------------------------------------------------------------------------
// Permissive parsing and re-serialisation
// ---------------------------------------------------------------------------

#[test]
fn unknown_members_of_the_reply_survive() {
    let reply: FlowReply = serde_json::from_str(
        r#"{"name":"flow","body":"Sent","response_json":"{}","extra_key":{"a":1}}"#,
    )
    .unwrap();
    assert_eq!(reply.extra.get("extra_key"), Some(&json!({"a": 1})));

    let rendered = serde_json::to_value(&reply).unwrap();
    assert_eq!(rendered["extra_key"], json!({"a": 1}));
    assert_eq!(rendered["response_json"], "{}");
}

#[test]
fn a_camel_case_response_json_is_tolerated() {
    let reply: FlowReply =
        serde_json::from_str(r#"{"responseJson":"{\"flow_token\":\"t\"}"}"#).unwrap();
    assert_eq!(reply.response().unwrap().flow_token(), Some("t"));
}

#[test]
fn a_reply_without_any_member_still_parses() {
    let reply: FlowReply = serde_json::from_str("{}").unwrap();
    assert_eq!(reply.name, None);
    assert_eq!(reply.body, None);
    assert_eq!(reply.response_json, None);
    assert_eq!(reply.extra, Map::new());
    assert!(reply.response().is_err());
}

#[test]
fn serde_round_trip_is_stable() {
    let reply = FlowReply {
        response_json: Some(r#"{"flow_token":"t","n":1}"#.to_string()),
        name: Some("flow".to_string()),
        body: Some("Sent".to_string()),
        extra: Map::new(),
    };
    let wire = serde_json::to_string(&reply).unwrap();
    let back: FlowReply = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, reply);
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

proptest! {
    /// Decoding never panics, and a success means the bytes really were JSON.
    #[test]
    fn response_decoding_is_total(raw in ".{0,96}") {
        let reply = FlowReply {
            response_json: Some(raw.clone()),
            ..FlowReply::default()
        };
        match reply.response() {
            Ok(response) => {
                prop_assert_eq!(
                    Some(response.as_value().clone()),
                    serde_json::from_str::<Value>(&raw).ok()
                );
            }
            Err(Error::Payload(message)) => prop_assert!(message.contains("response_json")),
            Err(other) => prop_assert!(false, "unexpected error: {other}"),
        }
    }

    /// `field` is exactly `get` restricted to strings, and `flow_token` is the
    /// `flow_token` field — no key can make the two disagree.
    #[test]
    fn accessors_agree_with_the_document(entries in prop::collection::vec(
        ("[a-z_]{0,12}", ".{0,16}"), 0..6
    )) {
        let mut object = Map::new();
        for (key, value) in &entries {
            object.insert(key.clone(), Value::String(value.clone()));
        }
        let reply = FlowReply {
            response_json: Some(Value::Object(object.clone()).to_string()),
            ..FlowReply::default()
        };
        let response = reply.response().unwrap();
        for (key, value) in &object {
            prop_assert_eq!(response.field(key.as_str()), value.as_str());
            prop_assert_eq!(response.get(key.as_str()), Some(value));
        }
        prop_assert_eq!(
            response.flow_token(),
            object.get("flow_token").and_then(Value::as_str)
        );
    }
}

// ---------------------------------------------------------------------------
// Outbound limits, one value on each side of every boundary
// ---------------------------------------------------------------------------

#[test]
fn cta_holds_thirty_characters_and_rejects_thirty_one() {
    let twenty_nine = FlowMessage::new("flow", "x".repeat(29), FlowAction::DataExchange);
    assert!(twenty_nine.validate().is_ok());
    let thirty = FlowMessage::new("flow", "x".repeat(30), FlowAction::DataExchange);
    assert!(thirty.validate().is_ok());
    let thirty_one = FlowMessage::new("flow", "x".repeat(31), FlowAction::DataExchange);
    assert!(matches!(
        thirty_one.validate(),
        Err(Error::Validation { .. })
    ));

    // Characters, not bytes: 30 multi-byte characters still pass.
    let umlauts = FlowMessage::new("flow", "ä".repeat(30), FlowAction::DataExchange);
    assert!(umlauts.validate().is_ok());
    let umlauts_too_long = FlowMessage::new("flow", "ä".repeat(31), FlowAction::DataExchange);
    assert!(umlauts_too_long.validate().is_err());
}

#[test]
fn flow_token_holds_1024_bytes_and_rejects_1025() {
    let exact =
        FlowMessage::new("flow", "c", FlowAction::DataExchange).flow_token("t".repeat(1024));
    assert!(exact.validate().is_ok());
    let over = FlowMessage::new("flow", "c", FlowAction::DataExchange).flow_token("t".repeat(1025));
    assert!(over.validate().is_err());
    // An empty token is the default and is allowed: Meta makes it optional.
    let empty = FlowMessage::new("flow", "c", FlowAction::DataExchange);
    assert!(empty.validate().is_ok());
    assert_eq!(empty.flow_token, "");
}

#[test]
fn flow_id_and_screen_are_non_empty() {
    assert!(FlowMessage::new("f", "c", FlowAction::navigate("S"))
        .validate()
        .is_ok());
    assert!(FlowMessage::new("", "c", FlowAction::navigate("S"))
        .validate()
        .is_err());
    assert!(FlowMessage::new("f", "c", FlowAction::navigate(""))
        .validate()
        .is_err());
}

#[test]
fn screen_data_must_be_a_json_object() {
    let object = FlowAction::navigate_with_data("CART", json!({"cart_id": "c-1"}));
    assert!(FlowMessage::new("f", "c", object).validate().is_ok());

    for wrong in [
        json!("c-1"),
        json!(["c-1"]),
        json!(7),
        json!(null),
        json!(true),
    ] {
        let action = FlowAction::navigate_with_data("CART", wrong.clone());
        let error = FlowMessage::new("f", "c", action).validate().unwrap_err();
        match error {
            Error::Validation { field, message } => {
                assert_eq!(
                    field,
                    "interactive.action.parameters.flow_action_payload.data"
                );
                assert_eq!(message, "must be a JSON object");
            }
            other => panic!("payload {wrong} produced {other:?}"),
        }
    }
}

#[test]
fn screen_data_is_rendered_inside_the_action_payload() {
    let request = MessageRequest::new(
        "4915112345678",
        FlowMessage::new(
            "flow-1",
            "Buchen",
            FlowAction::navigate_with_data("CART", json!({"cart_id": "c-1"})),
        ),
    );
    let json = request.to_json().unwrap();
    let payload = &json["interactive"]["action"]["parameters"]["flow_action_payload"];
    assert_eq!(payload["screen"], "CART");
    assert_eq!(payload["data"]["cart_id"], "c-1");
    assert_eq!(
        json["interactive"]["action"]["parameters"]["flow_action"],
        "navigate"
    );
}

#[test]
fn data_exchange_renders_no_action_payload() {
    let json = MessageRequest::new("1", FlowMessage::new("f", "c", FlowAction::DataExchange))
        .to_json()
        .unwrap();
    let parameters = &json["interactive"]["action"]["parameters"];
    assert_eq!(parameters["flow_action"], "data_exchange");
    assert!(parameters.get("flow_action_payload").is_none());
}
