//! Address messages: the exact body that goes on the wire, every documented
//! limit with its boundary pair, and the typed form of the reply.

mod common;

use common::{accepted, client, Body, Call};
use meta_whatsapp::{
    AddressMessage, AddressReply, AddressValidationError, AddressValues, Error, HeaderText,
    InteractiveMessage, Message, MessageBody, MessageRequest, SavedAddress, ADDRESS_COUNTRY_LEN,
    ADDRESS_FIELD_MAX_BYTES, ADDRESS_PIN_CODE_MAX_CHARS,
    types::FlowReply,
};
use serde_json::{json, Value};

/// The JSON body the transport recorded.
fn json_body(call: &Call) -> &Value {
    match &call.body {
        Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

/// Asserts that rendering fails with a validation error naming `field`.
fn assert_rejected(request: &MessageRequest, field: &str) {
    match request.to_json() {
        Err(Error::Validation {
            field: actual,
            message,
        }) => assert_eq!(actual, field, "wrong field ({message})"),
        other => panic!("expected a validation error for {field}, got {other:?}"),
    }
}

/// A full form: every parameter the API defines is populated.
fn full_message() -> AddressMessage {
    AddressMessage::new("IN", "Thanks for your order! Where should it go?")
        .header(HeaderText::new("Order 42"))
        .footer("Delivery inside India only")
        .values(
            AddressValues::new()
                .name("Ada Lovelace")
                .phone_number("+911234567890")
                .in_pin_code("400063")
                .house_number("12B")
                .floor_number("8")
                .tower_number("A")
                .building_name("Cello Triumph")
                .address("Wing A, IB Patel Rd")
                .landmark_area("Goregaon")
                .city("Mumbai")
                .state("Maharashtra"),
        )
        .saved_address(SavedAddress::new(
            "address1",
            AddressValues::new()
                .name("Ada Lovelace")
                .in_pin_code("411001")
                .city("Pune"),
        ))
        .validation_error(AddressValidationError::new(
            "in_pin_code",
            "We could not locate this pin code.",
        ))
}

#[tokio::test]
async fn the_full_form_lands_on_the_wire_unchanged() {
    let transport = common::MockTransport::new();
    transport.push_response(accepted("wamid.1"));
    let client = client(transport.clone());

    client
        .send(&MessageRequest::new("919876543210", full_message()))
        .await
        .expect("the full form is valid");

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/messages"
    );
    assert_eq!(
        json_body(&call),
        &json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": "919876543210",
            "type": "interactive",
            "interactive": {
                "type": "address_message",
                "header": { "type": "text", "text": "Order 42" },
                "body": { "text": "Thanks for your order! Where should it go?" },
                "footer": { "text": "Delivery inside India only" },
                "action": {
                    "name": "address_message",
                    "parameters": {
                        "country": "IN",
                        "values": {
                            "name": "Ada Lovelace",
                            "phone_number": "+911234567890",
                            "in_pin_code": "400063",
                            "house_number": "12B",
                            "floor_number": "8",
                            "tower_number": "A",
                            "building_name": "Cello Triumph",
                            "address": "Wing A, IB Patel Rd",
                            "landmark_area": "Goregaon",
                            "city": "Mumbai",
                            "state": "Maharashtra",
                        },
                        "saved_addresses": [{
                            "id": "address1",
                            "value": {
                                "name": "Ada Lovelace",
                                "in_pin_code": "411001",
                                "city": "Pune",
                            },
                        }],
                        "validation_errors": {
                            "in_pin_code": "We could not locate this pin code.",
                        },
                    },
                },
            },
        })
    );
}

#[tokio::test]
async fn an_empty_form_omits_every_optional_parameter() {
    let transport = common::MockTransport::new();
    let client = client(transport.clone());

    client
        .send(&MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "Where?"),
        ))
        .await
        .expect("country and body are enough");

    assert_eq!(
        json_body(&transport.only_call()),
        &json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": "919876543210",
            "type": "interactive",
            "interactive": {
                "type": "address_message",
                "body": { "text": "Where?" },
                "action": { "name": "address_message", "parameters": { "country": "IN" } },
            },
        })
    );
}

#[test]
fn country_is_exactly_two_ascii_letters() {
    assert_eq!(ADDRESS_COUNTRY_LEN, 2);
    for accepted in ["IN", "in", "In"] {
        assert!(
            AddressMessage::new(accepted, "b").validate().is_ok(),
            "{accepted} should be accepted"
        );
    }
    for rejected in ["", "I", "IND", "1N", "I ", "IN "] {
        assert_rejected(
            &MessageRequest::new("919876543210", AddressMessage::new(rejected, "b")),
            "interactive.action.parameters.country",
        );
    }
}

#[test]
fn body_is_required_and_capped_at_one_kibibyte() {
    assert_rejected(
        &MessageRequest::new("919876543210", AddressMessage::new("IN", "")),
        "interactive.body.text",
    );

    let at_limit = "x".repeat(1024);
    assert!(AddressMessage::new("IN", at_limit).validate().is_ok());
    assert_rejected(
        &MessageRequest::new("919876543210", AddressMessage::new("IN", "x".repeat(1025))),
        "interactive.body.text",
    );

    // A limit in bytes, not characters: 512 two-byte characters are exactly
    // 1024 bytes, 513 are one byte too many.
    assert!(AddressMessage::new("IN", "ä".repeat(512))
        .validate()
        .is_ok());
    assert_rejected(
        &MessageRequest::new("919876543210", AddressMessage::new("IN", "ä".repeat(513))),
        "interactive.body.text",
    );
}

#[test]
fn header_and_footer_keep_their_own_limits() {
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").header(HeaderText::new("h".repeat(61))),
        ),
        "interactive.header.text",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").header(HeaderText::new("")),
        ),
        "interactive.header.text",
    );
    assert!(AddressMessage::new("IN", "b")
        .header(HeaderText::new("h".repeat(60)))
        .validate()
        .is_ok());
    assert!(AddressMessage::new("IN", "b")
        .footer("f".repeat(60))
        .validate()
        .is_ok());
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").footer("f".repeat(61)),
        ),
        "interactive.footer.text",
    );
}

#[test]
fn free_form_values_are_capped_at_one_kibibyte() {
    assert_eq!(ADDRESS_FIELD_MAX_BYTES, 1024);
    assert!(AddressValues::new()
        .address("a".repeat(1024))
        .validate()
        .is_ok());
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").values(AddressValues::new().address("a".repeat(1025))),
        ),
        "interactive.action.parameters.values.address",
    );

    // Bytes, not characters — the same pair with two-byte characters.
    assert!(AddressValues::new()
        .city("ä".repeat(512))
        .validate()
        .is_ok());
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").values(AddressValues::new().city("ä".repeat(513))),
        ),
        "interactive.action.parameters.values.city",
    );
}

#[test]
fn an_empty_value_is_legal_because_meta_sends_one() {
    // Meta's own saved-address example carries `"building_name": ""`.
    let message = AddressMessage::new("IN", "b")
        .values(AddressValues::new().building_name("").city("Mumbai"));
    assert!(message.validate().is_ok());

    let json = MessageRequest::new("919876543210", message)
        .to_json()
        .expect("valid");
    assert_eq!(
        json["interactive"]["action"]["parameters"]["values"]["building_name"],
        ""
    );
}

#[test]
fn the_pin_code_limit_counts_characters() {
    assert_eq!(ADDRESS_PIN_CODE_MAX_CHARS, 6);
    for accepted in ["400063", "123456", "①②③④⑤⑥"] {
        assert!(AddressValues::new()
            .in_pin_code(accepted)
            .validate()
            .is_ok());
    }
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").values(AddressValues::new().in_pin_code("4000631")),
        ),
        "interactive.action.parameters.values.in_pin_code",
    );
}

#[test]
fn phone_numbers_are_an_optional_plus_and_seven_to_fifteen_digits() {
    for accepted in [
        "+911234567890",
        "911234567890",
        "1234567",
        "+123456789012345",
    ] {
        assert!(
            AddressValues::new()
                .phone_number(accepted)
                .validate()
                .is_ok(),
            "{accepted} should be accepted"
        );
    }
    for rejected in [
        "",
        "+",
        "123456",
        "+1234567890123456",
        "+91 1234567",
        "++911234567",
    ] {
        assert_rejected(
            &MessageRequest::new(
                "919876543210",
                AddressMessage::new("IN", "b").values(AddressValues::new().phone_number(rejected)),
            ),
            "interactive.action.parameters.values.phone_number",
        );
    }
}

#[test]
fn saved_addresses_need_a_usable_id_and_at_least_one_field() {
    assert!(
        SavedAddress::new("address1", AddressValues::new().city("Pune"))
            .validate()
            .is_ok()
    );
    // The id ceiling itself is legal — 1024 bytes is the boundary.
    assert!(SavedAddress::new(
        "i".repeat(ADDRESS_FIELD_MAX_BYTES),
        AddressValues::new().city("Pune"),
    )
    .validate()
    .is_ok());

    for (saved, field) in [
        (
            SavedAddress::new("", AddressValues::new().city("Pune")),
            "interactive.action.parameters.saved_addresses.id",
        ),
        (
            SavedAddress::new("bad\nid", AddressValues::new().city("Pune")),
            "interactive.action.parameters.saved_addresses.id",
        ),
        (
            SavedAddress::new(
                "x".repeat(ADDRESS_FIELD_MAX_BYTES + 1),
                AddressValues::new().city("Pune"),
            ),
            "interactive.action.parameters.saved_addresses.id",
        ),
        (
            SavedAddress::new("address1", AddressValues::new()),
            "interactive.action.parameters.saved_addresses.value",
        ),
        (
            SavedAddress::new(
                "address1",
                AddressValues::new().city("c".repeat(ADDRESS_FIELD_MAX_BYTES + 1)),
            ),
            "interactive.action.parameters.values.city",
        ),
    ] {
        let message = AddressMessage::new("IN", "b").saved_address(saved);
        assert_rejected(&MessageRequest::new("919876543210", message), field);
    }
}

#[test]
fn saved_address_ids_must_be_unique() {
    let message = AddressMessage::new("IN", "b")
        .saved_address(SavedAddress::new("home", AddressValues::new().city("Pune")))
        .saved_address(SavedAddress::new(
            "home",
            AddressValues::new().city("Mumbai"),
        ));
    assert_rejected(
        &MessageRequest::new("919876543210", message),
        "interactive.action.parameters.saved_addresses.id",
    );
}

#[test]
fn validation_errors_need_a_named_field_and_a_message() {
    // Both strings sit at their ceiling — 1024 bytes is legal on each.
    assert!(AddressValidationError::new(
        "f".repeat(ADDRESS_FIELD_MAX_BYTES),
        "m".repeat(ADDRESS_FIELD_MAX_BYTES),
    )
    .validate()
    .is_ok());
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b")
                .validation_error(AddressValidationError::new("city", "")),
        ),
        "interactive.action.parameters.validation_errors.message",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b")
                .validation_error(AddressValidationError::new("", "Unknown")),
        ),
        "interactive.action.parameters.validation_errors.field",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b")
                .validation_error(AddressValidationError::new("thin\u{7}king", "Unknown")),
        ),
        "interactive.action.parameters.validation_errors.field",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").validation_error(AddressValidationError::new(
                "f".repeat(ADDRESS_FIELD_MAX_BYTES + 1),
                "Unknown",
            )),
        ),
        "interactive.action.parameters.validation_errors.field",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b").validation_error(AddressValidationError::new(
                "city",
                "m".repeat(ADDRESS_FIELD_MAX_BYTES + 1),
            )),
        ),
        "interactive.action.parameters.validation_errors.message",
    );
    assert_rejected(
        &MessageRequest::new(
            "919876543210",
            AddressMessage::new("IN", "b")
                .validation_error(AddressValidationError::new("city", "Unknown"))
                .validation_error(AddressValidationError::new("city", "Still unknown")),
        ),
        "interactive.action.parameters.validation_errors.field",
    );
}

#[test]
fn the_envelope_reports_what_it_is() {
    let message = AddressMessage::new("IN", "Where?");
    assert_eq!(message.body, "Where?");
    assert_eq!(message.country, "IN");
    assert!(message.values.is_empty());
    assert_eq!(message.saved_addresses.len(), 0);
    assert_eq!(message.validation_errors.len(), 0);

    assert_eq!(
        InteractiveMessage::Address(message.clone()).kind(),
        "address_message"
    );
    assert!(InteractiveMessage::Address(message.clone())
        .validate()
        .is_ok());

    let body: MessageBody = message.into();
    assert_eq!(body.message_type(), "interactive");
    assert!(matches!(
        body,
        MessageBody::Interactive(InteractiveMessage::Address(_))
    ));
}

#[test]
fn values_expose_the_documented_field_names() {
    let values = AddressValues::new()
        .name("Ada")
        .phone_number("+911234567890")
        .in_pin_code("400063")
        .house_number("12B")
        .floor_number("8")
        .tower_number("A")
        .building_name("Cello")
        .address("IB Patel Rd")
        .landmark_area("Goregaon")
        .city("Mumbai")
        .state("Maharashtra");

    for (field, expected) in [
        ("name", "Ada"),
        ("phone_number", "+911234567890"),
        ("in_pin_code", "400063"),
        ("house_number", "12B"),
        ("floor_number", "8"),
        ("tower_number", "A"),
        ("building_name", "Cello"),
        ("address", "IB Patel Rd"),
        ("landmark_area", "Goregaon"),
        ("city", "Mumbai"),
        ("state", "Maharashtra"),
    ] {
        assert_eq!(values.get(field), Some(expected), "field {field}");
    }
    assert_eq!(values.get("zip"), None, "the wire name is in_pin_code");
    assert_eq!(values.get(""), None);
    assert!(!values.is_empty());
    assert!(AddressValues::default().is_empty());
}

#[test]
fn an_unmodelled_value_key_survives_rendering() {
    let mut values = AddressValues::new().city("Pune");
    values
        .extra
        .insert("district".to_string(), json!("Pune district"));

    let json = MessageRequest::new(
        "919876543210",
        AddressMessage::new("IN", "b").values(values),
    )
    .to_json()
    .expect("extra keys are forwarded, not validated");

    assert_eq!(
        json["interactive"]["action"]["parameters"]["values"]["district"],
        "Pune district"
    );
}

#[test]
fn a_saved_address_reply_uses_the_documented_shape() {
    // Verbatim from Meta's "Address message submission" webhook example.
    let payload: Message = serde_json::from_str(
        r#"{
          "context": {
            "from": "FROM_PHONE_NUMBER_ID",
            "id": "wamid.HBgLMTIwNjU1NTAxMDcVAgARGBI3NjNFN0U5QzMzNDlCQjY0M0QA"
          },
          "from": "919876543210",
          "id": "wamid.HBgLMTIwNjU1NTAxMDcVAgASGCA5RDhBNENEMEQ3RENEOEEzMEI0RUExRDczN0I1NThFQwA=",
          "timestamp": "1671498855",
          "type": "interactive",
          "interactive": {
            "type": "nfm_reply",
            "nfm_reply": {
              "response_json": "{\"saved_address_id\":\"address1\",\"values\":{\"in_pin_code\":\"400063\",\"building_name\":\"\",\"landmark_area\":\"Goregaon\",\"address\":\"Wing A, Cello Triumph, IB Patel Rd\",\"city\":\"Mumbai\",\"name\":\"CUSTOMER_NAME\",\"phone_number\":\"+91xxxxxxxxxx\",\"floor_number\":\"8\"}}",
              "body": "CUSTOMER_NAME\n +91xxxxxxxxxx\n 400063, Goregaon, Wing A, Cello Triumph,IB Patel Rd, Mumbai, 8",
              "name": "address_message"
            }
          }
        }"#,
    )
    .expect("the documented payload parses");

    let reply = AddressReply::from_message(&payload)
        .expect("it is an address reply")
        .expect("its response_json parses");

    assert_eq!(reply.saved_address_id.as_deref(), Some("address1"));
    assert_eq!(reply.values.name.as_deref(), Some("CUSTOMER_NAME"));
    assert_eq!(reply.values.phone_number.as_deref(), Some("+91xxxxxxxxxx"));
    assert_eq!(reply.values.in_pin_code.as_deref(), Some("400063"));
    assert_eq!(reply.values.floor_number.as_deref(), Some("8"));
    assert_eq!(reply.values.building_name.as_deref(), Some(""));
    assert_eq!(reply.values.landmark_area.as_deref(), Some("Goregaon"));
    assert_eq!(reply.values.city.as_deref(), Some("Mumbai"));
    assert_eq!(reply.values.state, None);
}

#[test]
fn a_typed_reply_keeps_keys_the_crate_does_not_model() {
    let reply = AddressReply::parse(
        r#"{
          "values": {"city": "Pune", "district": "Pune district"},
          "saved_address_id": null,
          "cart_id": "cart-7"
        }"#,
    )
    .expect("unknown keys are carried, not rejected");

    assert_eq!(reply.values.city.as_deref(), Some("Pune"));
    assert_eq!(
        reply.values.extra.get("district"),
        Some(&json!("Pune district"))
    );
    assert_eq!(reply.extra.get("cart_id"), Some(&json!("cart-7")));
    assert_eq!(reply.saved_address_id, None);
}

#[test]
fn a_reply_that_is_not_an_address_reply_is_not_claimed() {
    let text: Message = serde_json::from_str(
        r#"{"id":"wamid.1","from":"4915","type":"text","text":{"body":"hi"}}"#,
    )
    .expect("valid");
    assert!(AddressReply::from_message(&text).is_none());

    let button: Message = serde_json::from_str(
        r#"{"id":"wamid.1","from":"4915","type":"interactive",
            "interactive":{"type":"button_reply","button_reply":{"id":"a","title":"A"}}}"#,
    )
    .expect("valid");
    assert!(AddressReply::from_message(&button).is_none());

    let flow: Message = serde_json::from_str(
        r#"{"id":"wamid.1","from":"4915","type":"interactive",
            "interactive":{"type":"nfm_reply","nfm_reply":{
                "name":"flow","response_json":"{\"flow_token\":\"t\"}"}}}"#,
    )
    .expect("valid");
    assert!(
        AddressReply::from_message(&flow).is_none(),
        "another nfm_reply name is not an address reply"
    );

    let broken: Message = serde_json::from_str(
        r#"{"id":"wamid.1","from":"4915","type":"interactive",
            "interactive":{"type":"nfm_reply","nfm_reply":{
                "name":"address_message","response_json":"{not json"}}}"#,
    )
    .expect("valid");
    match AddressReply::from_message(&broken) {
        Some(Err(Error::Json { .. })) => {}
        other => panic!("expected a JSON error, got {other:?}"),
    }
}

#[test]
fn malformed_nfm_replies_are_typed_errors() {
    // `nfm_reply` is a typed `FlowReply` now, so the cases that used to be
    // "not a JSON object" or "response_json is a number" cannot reach this
    // function any more; what is left is the name check and a missing or
    // undecodable `response_json`.
    let wrong_name = FlowReply {
        name: Some("flow".to_string()),
        response_json: Some("{}".to_string()),
        body: None,
        extra: serde_json::Map::new(),
    };
    let missing_json = FlowReply {
        name: Some("address_message".to_string()),
        response_json: None,
        body: None,
        extra: serde_json::Map::new(),
    };
    for (reply, pattern) in [(&wrong_name, "not"), (&missing_json, "response_json")] {
        match AddressReply::from_nfm_reply(reply) {
            Err(Error::Payload(message)) => {
                assert!(message.contains(pattern), "{message:?} lacks {pattern:?}");
            }
            other => panic!("expected a payload error, got {other:?}"),
        }
    }

    let undecodable = FlowReply {
        name: None,
        response_json: Some("[1,2]".to_string()),
        body: None,
        extra: serde_json::Map::new(),
    };
    assert!(matches!(
        AddressReply::from_nfm_reply(&undecodable),
        Err(Error::Json { .. })
    ));
}

#[test]
fn a_reply_round_trips_through_serde() {
    let reply = AddressReply {
        saved_address_id: Some("address1".to_string()),
        values: AddressValues::new().city("Pune").in_pin_code("411001"),
        extra: serde_json::Map::new(),
    };
    let encoded = serde_json::to_string(&reply).expect("serializes");
    assert_eq!(
        AddressReply::parse(&encoded).expect("parses"),
        reply,
        "the typed reply survives an encode/decode cycle"
    );
}
