//! Template management (#10): create, edit and delete.
//!
//! The contract under test:
//!
//! * create → `POST /{phone_number_id}/message_templates` with
//!   `name`, `language`, `category`, `parameter_format` and the component
//!   texts with their examples.
//! * update → `PUT` with the name and language that identify the template.
//! * delete → `DELETE ...?name=…` with the name percent-encoded.
//! * create is a write: a 5xx is one call and [`Error::Ambiguous`], a 429 is
//!   retried.
//! * update and delete are idempotent: a 5xx is retried.
//! * every validation rule is checked before the request, and for each limit
//!   the accepted value, the boundary and the rejected value are named.

mod common;

use common::{client, instant_retry, json_response, response, MockTransport, Reply};
use meta_whatsapp::template::{
    TemplateDefinitionComponent, TemplateHeaderFormat, TemplateMediaKind, TemplateOtpType,
    TemplateParameter, TEMPLATE_BODY_MAX_BYTES, TEMPLATE_BUTTONS_MAX,
    TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS, TEMPLATE_BUTTON_TEXT_MAX_CHARS,
    TEMPLATE_BUTTON_URL_MAX_CHARS, TEMPLATE_CODE_EXPIRATION_MINUTES_MAX, TEMPLATE_COMPONENTS_MAX,
    TEMPLATE_EXAMPLES_MAX, TEMPLATE_FOOTER_MAX_CHARS, TEMPLATE_HEADER_MAX_CHARS,
    TEMPLATE_NAME_MAX_BYTES, TEMPLATE_URL_BUTTONS_MAX,
};
use meta_whatsapp::{
    Error, TemplateAck, TemplateButton, TemplateCategory, TemplateDefinition, TemplateDeleteAck,
    TemplateQuery, TemplateUpdate, WhatsAppClient,
};

const SERVER_ERROR: &[u8] = b"{\"error\":{\"code\":2,\"message\":\"internal\"}}";

/// A definition that validates: header with one example, body, footer, button.
fn definition() -> TemplateDefinition {
    TemplateDefinition::new("order_update", "de_DE", TemplateCategory::Utility)
        .header_text("Ihre Bestellung {{1}}")
        .header_example("1234")
        .body_text("Hallo {{1}}, die Bestellung {{2}} ist unterwegs.")
        .body_example("Florian")
        .body_example("1234")
        .footer_text("Danke für Ihren Einkauf")
        .button(TemplateButton::quick_reply("Alles klar"))
}

#[tokio::test]
async fn create_posts_every_documented_field() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "id": "594425479261596",
        "status": "PENDING",
        "category": "UTILITY"
    })));
    let client = client(transport.clone());

    let ack = client.create_template(&definition()).await.unwrap();

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/message_templates"
    );
    assert_eq!(call.token, "EAAG-test-token");

    let body = match call.body {
        common::Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    };
    assert_eq!(body["name"], "order_update");
    assert_eq!(body["language"], "de_DE");
    assert_eq!(body["category"], "UTILITY");
    assert_eq!(body["parameter_format"], "POSITIONAL");

    let components = body["components"].as_array().unwrap();
    assert_eq!(components.len(), 4);

    assert_eq!(components[0]["type"], "HEADER");
    assert_eq!(components[0]["format"], "TEXT");
    assert_eq!(components[0]["text"], "Ihre Bestellung {{1}}");
    assert_eq!(components[0]["example"]["header_text"][0], "1234");

    assert_eq!(components[1]["type"], "BODY");
    assert_eq!(
        components[1]["example"]["body_text"][0],
        serde_json::json!(["Florian", "1234"])
    );

    assert_eq!(components[2]["type"], "FOOTER");
    assert_eq!(components[2]["text"], "Danke für Ihren Einkauf");

    assert_eq!(components[3]["type"], "BUTTONS");
    assert_eq!(components[3]["buttons"][0]["type"], "QUICK_REPLY");
    assert_eq!(components[3]["buttons"][0]["text"], "Alles klar");

    assert_eq!(ack.id.as_deref(), Some("594425479261596"));
    assert_eq!(ack.status.as_deref(), Some("PENDING"));
    assert!(ack.is_success(), "no success flag is still a success");
}

#[tokio::test]
async fn create_reports_a_rejected_ack() {
    let transport = MockTransport::new();
    transport.push_response(json_response(
        &serde_json::json!({ "success": false, "unknown": 1 }),
    ));
    let client = client(transport.clone());

    let ack = client.create_template(&definition()).await.unwrap();
    assert!(!ack.is_success());
    assert_eq!(ack.extra["unknown"], 1);
}

#[tokio::test]
async fn create_ack_and_delete_ack_decode_permissively() {
    let ack = TemplateAck::from_value(&serde_json::json!({
        "id": "1",
        "status": "APPROVED",
        "category": "MARKETING",
        "success": true,
        "brand_new_field": {"nested": [1, 2]}
    }))
    .unwrap();
    assert_eq!(ack.category.as_deref(), Some("MARKETING"));
    assert_eq!(ack.extra["brand_new_field"]["nested"][0], 1);

    // An empty body is what the client sees for a 204.
    let empty = TemplateAck::from_value(&serde_json::json!({})).unwrap();
    assert_eq!(empty, TemplateAck::default());
    assert!(empty.is_success());

    // Not an object is a decoding error, not a panic.
    for value in [
        serde_json::json!([1, 2]),
        serde_json::json!("nope"),
        serde_json::Value::Null,
    ] {
        assert!(matches!(
            TemplateAck::from_value(&value),
            Err(Error::Json { .. })
        ));
        assert!(matches!(
            TemplateDeleteAck::from_value(&value),
            Err(Error::Json { .. })
        ));
    }

    let deleted = TemplateDeleteAck::from_value(&serde_json::json!({"success": true})).unwrap();
    assert_eq!(deleted.success, Some(true));
    assert!(deleted.is_success());
    assert!(TemplateDeleteAck::default().success.is_none());
    assert!(TemplateDeleteAck::default().is_success());
}

#[tokio::test]
async fn a_category_only_edit_sends_no_components() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({"success": true})));
    let client = client(transport.clone());

    let update = TemplateUpdate::new("order_update", "de").category(TemplateCategory::Marketing);
    let ack = client.update_template(&update).await.unwrap();
    assert_eq!(ack.success, Some(true));

    let call = transport.only_call();
    assert_eq!(
        call.method.as_str(),
        "POST",
        "edit is a POST on the same node"
    );
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/message_templates"
    );
    let body = match call.body {
        common::Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    };
    assert_eq!(body["name"], "order_update");
    assert_eq!(body["language"], "de");
    assert_eq!(body["category"], "MARKETING");
    assert!(body.get("components").is_none(), "nothing else is sent");
}

#[tokio::test]
async fn an_edit_sends_the_components_it_changes() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    let update = TemplateUpdate::new("order_update", "de")
        .component(TemplateDefinitionComponent::footer_text("Neu"))
        .component(TemplateDefinitionComponent::buttons(vec![
            TemplateButton::url("Sendung", "https://x.test/{{1}}").with_example("https://x.test/1"),
        ]));
    let ack = client.update_template(&update).await.unwrap();
    assert!(ack.is_success());

    let body = match transport.only_call().body {
        common::Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    };
    let components = body["components"].as_array().unwrap();
    assert_eq!(components.len(), 2);
    assert_eq!(components[0]["type"], "FOOTER");
    assert_eq!(components[1]["buttons"][0]["type"], "URL");
    assert_eq!(
        components[1]["buttons"][0]["example"][0],
        "https://x.test/1"
    );
}

#[tokio::test]
async fn delete_puts_the_name_in_the_query_string_encoded() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    let ack = client.delete_template("order update & co").await.unwrap();
    assert!(ack.is_success());

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "DELETE");
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/message_templates\
         ?name=order%20update%20%26%20co"
    );
    assert_eq!(call.body, common::Body::Empty, "DELETE carries no body");
}

#[tokio::test]
async fn create_is_never_retried_after_a_5xx() {
    let transport = MockTransport::new();
    for _ in 0..4 {
        transport.push_response(response(500, &[], SERVER_ERROR));
    }
    let client = client(transport.clone()).with_retry(instant_retry(5));

    let err = client.create_template(&definition()).await.unwrap_err();

    assert_eq!(transport.call_count(), 1, "never re-sent");
    assert!(err.is_ambiguous(), "{err:?}");
    assert!(!err.is_retryable());
    match &err {
        Error::Ambiguous { operation, .. } => {
            assert!(operation.contains("/message_templates"), "{operation}");
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
}

#[tokio::test]
async fn create_retries_a_429_because_nothing_was_written() {
    let transport = MockTransport::new();
    transport.push_response(response(429, &[], b"{}"));
    transport.push_response(json_response(&serde_json::json!({"id": "1"})));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let ack = client.create_template(&definition()).await.unwrap();
    assert_eq!(ack.id.as_deref(), Some("1"));
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn update_and_delete_are_retried_because_repeating_them_is_safe() {
    let update_transport = MockTransport::new();
    for _ in 0..3 {
        update_transport.push_response(response(503, &[], SERVER_ERROR));
    }
    let update_client = client(update_transport.clone()).with_retry(instant_retry(3));
    let err = update_client
        .update_template(&TemplateUpdate::new("t", "de").category(TemplateCategory::Utility))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Api { status: 503, .. }), "{err:?}");
    assert_eq!(update_transport.call_count(), 3, "retried up to the budget");

    let delete_transport = MockTransport::new();
    for _ in 0..3 {
        delete_transport.push_response(response(503, &[], SERVER_ERROR));
    }
    let delete_client = client(delete_transport.clone()).with_retry(instant_retry(3));
    let err = delete_client.delete_template("t").await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 503, .. }), "{err:?}");
    assert_eq!(delete_transport.call_count(), 3);
}

#[tokio::test]
async fn a_4xx_is_terminal_and_not_retried() {
    let transport = MockTransport::new();
    transport.push_response(response(
        400,
        &[],
        b"{\"error\":{\"code\":100,\"message\":\"Invalid parameter\"}}",
    ));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.create_template(&definition()).await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 400, .. }), "{err:?}");
    assert!(!err.is_retryable());
    assert!(!err.is_ambiguous());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn a_lost_response_after_the_write_is_ambiguous() {
    let transport = MockTransport::new();
    transport.push(Reply::after_write("connection reset"));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.create_template(&definition()).await.unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn an_invalid_definition_never_reaches_the_network() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    let cases = [
        // No body at all.
        TemplateDefinition::new("t", "de", TemplateCategory::Utility),
        // Empty name.
        TemplateDefinition::new("", "de", TemplateCategory::Utility).body_text("x"),
        // Bad language code.
        TemplateDefinition::new("t", "", TemplateCategory::Utility).body_text("x"),
        TemplateDefinition::new("t", "de DE", TemplateCategory::Utility).body_text("x"),
        // Invalid component.
        TemplateDefinition::new("t", "de", TemplateCategory::Utility).body_text("Hallo {{1}}"),
    ];
    for definition in cases {
        let err = client.create_template(&definition).await.unwrap_err();
        assert!(matches!(err, Error::Validation { .. }), "{err:?}");
    }

    let update = TemplateUpdate::new("t", "de");
    assert!(matches!(
        client.update_template(&update).await.unwrap_err(),
        Error::Validation { .. }
    ));
    assert!(matches!(
        client.delete_template("").await.unwrap_err(),
        Error::Validation { .. }
    ));
    assert!(matches!(
        client
            .delete_template(&"x".repeat(TEMPLATE_NAME_MAX_BYTES + 1))
            .await
            .unwrap_err(),
        Error::Validation { .. }
    ));

    assert_eq!(transport.call_count(), 0, "validation happens first");
}

#[tokio::test]
async fn an_unsafe_phone_number_id_is_refused_before_the_request() {
    let transport = MockTransport::new();
    let client = WhatsAppClient::new("../../me", "EAAG", transport.clone());

    assert!(matches!(
        client.create_template(&definition()).await.unwrap_err(),
        Error::Validation {
            field: "phone_number_id",
            ..
        }
    ));
    assert_eq!(transport.call_count(), 0);

    // The documented id width is 32 digits; 33 is refused.
    let long = WhatsAppClient::new("1".repeat(33), "EAAG", transport.clone());
    assert!(long.create_template(&definition()).await.is_err());
    let empty = WhatsAppClient::new("", "EAAG", transport.clone());
    assert!(empty.create_template(&definition()).await.is_err());
    let boundary = WhatsAppClient::new("1".repeat(32), "EAAG", transport.clone());
    assert!(boundary.create_template(&definition()).await.is_ok());
    assert_eq!(transport.call_count(), 1);
}

#[test]
fn the_template_name_stops_at_the_documented_boundary() {
    let name = |len: usize| {
        TemplateDefinition::new("x".repeat(len), "de", TemplateCategory::Utility)
            .body_text("Danke")
            .validate()
    };
    assert!(name(TEMPLATE_NAME_MAX_BYTES - 1).is_ok());
    assert!(
        name(TEMPLATE_NAME_MAX_BYTES).is_ok(),
        "the limit is inclusive"
    );
    assert!(name(TEMPLATE_NAME_MAX_BYTES + 1).is_err());
    assert!(name(0).is_err());
}

#[test]
fn the_header_text_stops_at_the_documented_boundary() {
    let header = |len: usize| {
        TemplateDefinition::new("t", "de", TemplateCategory::Utility)
            .header_text("x".repeat(len))
            .body_text("Danke")
            .validate()
    };
    assert!(header(TEMPLATE_HEADER_MAX_CHARS - 1).is_ok());
    assert!(header(TEMPLATE_HEADER_MAX_CHARS).is_ok());
    assert!(header(TEMPLATE_HEADER_MAX_CHARS + 1).is_err());
    assert!(header(0).is_err());
}

#[test]
fn the_body_text_stops_at_the_documented_boundary() {
    let body = |len: usize| {
        TemplateDefinition::new("t", "de", TemplateCategory::Utility)
            .body_text("x".repeat(len))
            .validate()
    };
    assert!(body(TEMPLATE_BODY_MAX_BYTES - 1).is_ok());
    assert!(body(TEMPLATE_BODY_MAX_BYTES).is_ok());
    assert!(body(TEMPLATE_BODY_MAX_BYTES + 1).is_err());
    assert!(body(0).is_err());

    // The body limit counts bytes, not characters: 512 two-byte characters
    // are exactly 1024 bytes and still fit.
    let exact = "ä".repeat(TEMPLATE_BODY_MAX_BYTES / 2);
    assert_eq!(exact.len(), TEMPLATE_BODY_MAX_BYTES);
    assert!(body(0).is_err());
    assert!(
        TemplateDefinition::new("t", "de", TemplateCategory::Utility)
            .body_text(exact)
            .validate()
            .is_ok()
    );
    assert!(
        TemplateDefinition::new("t", "de", TemplateCategory::Utility)
            .body_text("ä".repeat(TEMPLATE_BODY_MAX_BYTES / 2 + 1))
            .validate()
            .is_err()
    );
}

#[test]
fn the_footer_text_stops_at_the_documented_boundary() {
    let footer = |len: usize| {
        TemplateDefinition::new("t", "de", TemplateCategory::Utility)
            .body_text("Danke")
            .footer_text("x".repeat(len))
            .validate()
    };
    assert!(footer(TEMPLATE_FOOTER_MAX_CHARS - 1).is_ok());
    assert!(footer(TEMPLATE_FOOTER_MAX_CHARS).is_ok());
    assert!(footer(TEMPLATE_FOOTER_MAX_CHARS + 1).is_err());
    assert!(footer(0).is_err());
}

#[test]
fn a_media_header_needs_exactly_one_handle() {
    let header = |handles: usize| TemplateDefinitionComponent::Header {
        format: TemplateHeaderFormat::Image,
        text: None,
        example: (0..handles)
            .map(|index| TemplateParameter::text(format!("handle-{index}")))
            .collect(),
    };
    assert!(header(0).validate().is_err());
    assert!(header(1).validate().is_ok());
    assert!(header(2).validate().is_err());

    // The wire key is `header_handle`, not `header_text`.
    let payload = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .header_media(TemplateHeaderFormat::Document, "4:handle")
        .body_text("Danke")
        .render()
        .unwrap();
    assert_eq!(
        payload["components"][0]["example"]["header_handle"][0],
        "4:handle"
    );
    assert_eq!(payload["components"][0]["format"], "DOCUMENT");

    // A media header carries no text; a location header carries neither text
    // nor example.
    assert!(TemplateDefinitionComponent::Header {
        format: TemplateHeaderFormat::Video,
        text: Some("x".into()),
        example: vec![TemplateParameter::text("h")],
    }
    .validate()
    .is_err());
    assert!(TemplateDefinitionComponent::Header {
        format: TemplateHeaderFormat::Location,
        text: None,
        example: Vec::new(),
    }
    .validate()
    .is_ok());
    assert!(TemplateDefinitionComponent::Header {
        format: TemplateHeaderFormat::Location,
        text: Some("x".into()),
        example: Vec::new(),
    }
    .validate()
    .is_err());
    assert!(TemplateDefinitionComponent::Header {
        format: TemplateHeaderFormat::Location,
        text: None,
        example: vec![TemplateParameter::text("h")],
    }
    .validate()
    .is_err());
}

#[test]
fn placeholders_and_examples_stay_in_step() {
    // One placeholder, no example.
    assert!(TemplateDefinitionComponent::body_text("Hallo {{1}}")
        .validate()
        .is_err());
    // An example without a placeholder.
    assert!(TemplateDefinitionComponent::body_text("Hallo")
        .with_example("Florian")
        .validate()
        .is_err());
    // Two placeholders, two examples.
    assert!(TemplateDefinitionComponent::body_text("{{1}} und {{2}}")
        .with_example("a")
        .with_example("b")
        .validate()
        .is_ok());
    // More examples than placeholders.
    assert!(TemplateDefinitionComponent::body_text("{{1}}")
        .with_example("a")
        .with_example("b")
        .validate()
        .is_err());
    // Named placeholders count the same way.
    assert!(TemplateDefinitionComponent::body_text("Hallo {{name}}")
        .with_example("Florian")
        .validate()
        .is_ok());
    // A non-text example is rejected before it is rendered.
    assert!(TemplateDefinitionComponent::body_text("{{1}}")
        .with_example(TemplateParameter::currency("$1", "USD", 1_000))
        .validate()
        .is_err());
    // An empty example value is rejected by the parameter rules.
    assert!(TemplateDefinitionComponent::body_text("{{1}}")
        .with_example("")
        .validate()
        .is_err());

    // At most TEMPLATE_EXAMPLES_MAX values, and every value counts.
    let placeholders = "{{1}} ".repeat(TEMPLATE_EXAMPLES_MAX);
    let with = |count: usize| {
        let mut component = TemplateDefinitionComponent::body_text(placeholders.clone());
        for index in 0..count {
            component = component.with_example(format!("v{index}"));
        }
        component.validate()
    };
    assert!(with(TEMPLATE_EXAMPLES_MAX - 1).is_ok());
    assert!(with(TEMPLATE_EXAMPLES_MAX).is_ok());
    assert!(with(TEMPLATE_EXAMPLES_MAX + 1).is_err());
}

#[test]
fn the_security_recommendation_body_is_exclusive() {
    let definition = TemplateDefinition::new("otp", "de", TemplateCategory::Authentication)
        .security_recommendation();
    let payload = definition.render().unwrap();
    assert_eq!(
        payload["components"][0]["add_security_recommendation"],
        true
    );
    assert!(payload["components"][0].get("text").is_none());

    // A text and the recommendation exclude each other.
    assert!(TemplateDefinitionComponent::Body {
        text: Some("Hallo".into()),
        security_recommendation: true,
        example: Vec::new(),
    }
    .validate()
    .is_err());
    assert!(TemplateDefinitionComponent::Body {
        text: None,
        security_recommendation: true,
        example: vec![TemplateParameter::text("x")],
    }
    .validate()
    .is_err());

    // A text body that is replaced by the recommendation loses its examples.
    let replaced = TemplateDefinition::new("otp", "de", TemplateCategory::Authentication)
        .body_text("Hallo {{1}}")
        .body_example("Florian")
        .security_recommendation();
    assert!(replaced.render().is_ok());
    assert_eq!(
        replaced.render().unwrap()["components"][0]["add_security_recommendation"],
        true
    );
}

#[test]
fn the_code_expiration_stops_at_the_documented_boundary() {
    let expiring =
        |minutes: u16| TemplateDefinitionComponent::footer_code_expiration(minutes).validate();
    assert!(expiring(0).is_err());
    assert!(expiring(1).is_ok());
    assert!(expiring(TEMPLATE_CODE_EXPIRATION_MINUTES_MAX).is_ok());
    assert!(expiring(TEMPLATE_CODE_EXPIRATION_MINUTES_MAX + 1).is_err());

    // Text and expiration exclude each other.
    assert!(TemplateDefinitionComponent::Footer {
        text: Some("Danke".into()),
        code_expiration_minutes: Some(10),
    }
    .validate()
    .is_err());
    assert!(TemplateDefinitionComponent::Footer {
        text: None,
        code_expiration_minutes: None,
    }
    .validate()
    .is_err());

    let payload = TemplateDefinition::new("otp", "de", TemplateCategory::Authentication)
        .security_recommendation()
        .code_expiration_minutes(10)
        .render()
        .unwrap();
    assert_eq!(
        payload["components"][1]["code_expiration_minutes"], 10,
        "the footer follows the body"
    );
}

#[test]
fn button_labels_and_destinations_stop_at_their_limits() {
    assert!(TemplateButton::quick_reply("").validate().is_err());
    assert!(
        TemplateButton::quick_reply("x".repeat(TEMPLATE_BUTTON_TEXT_MAX_CHARS - 1))
            .validate()
            .is_ok()
    );
    assert!(
        TemplateButton::quick_reply("x".repeat(TEMPLATE_BUTTON_TEXT_MAX_CHARS))
            .validate()
            .is_ok()
    );
    assert!(
        TemplateButton::quick_reply("x".repeat(TEMPLATE_BUTTON_TEXT_MAX_CHARS + 1))
            .validate()
            .is_err()
    );

    let url = |len: usize, scheme: &str| format!("{}://x.test/{}", scheme, "a".repeat(len));
    assert!(TemplateButton::url("Web", "").validate().is_err());
    assert!(TemplateButton::url("Web", "ftp://x.test")
        .validate()
        .is_err());
    assert!(TemplateButton::url("Web", url(0, "http"))
        .validate()
        .is_ok());
    assert!(TemplateButton::url("Web", url(0, "https"))
        .validate()
        .is_ok());

    let prefix = "https://x.test/";
    let at_limit = format!(
        "{prefix}{}",
        "a".repeat(TEMPLATE_BUTTON_URL_MAX_CHARS - prefix.len())
    );
    assert_eq!(at_limit.len(), TEMPLATE_BUTTON_URL_MAX_CHARS);
    assert!(TemplateButton::url("Web", &at_limit).validate().is_ok());
    assert!(TemplateButton::url("Web", format!("{at_limit}a"))
        .validate()
        .is_err());

    // A variable needs an example, and the example has its own ceiling.
    assert!(TemplateButton::url("Web", "https://x.test/{{1}}")
        .validate()
        .is_err());
    assert!(TemplateButton::url("Web", "https://x.test/{{1}}")
        .with_example("https://x.test/1")
        .validate()
        .is_ok());
    assert!(TemplateButton::url("Web", "https://x.test/{{1}}")
        .with_example("")
        .validate()
        .is_err());

    let code = |len: usize| TemplateButton::copy_code("Code", "C".repeat(len)).validate();
    assert!(code(0).is_err());
    assert!(code(TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS).is_ok());
    assert!(code(TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS + 1).is_err());

    let number = |digits: usize| {
        TemplateButton::phone_number("Anrufen", format!("+{}", "1".repeat(digits))).validate()
    };
    assert!(number(6).is_err());
    assert!(number(7).is_ok());
    assert!(number(15).is_ok());
    assert!(number(16).is_err());
    assert!(TemplateButton::phone_number("Anrufen", "++4915112345678")
        .validate()
        .is_err());
}

#[test]
fn otp_buttons_need_a_label() {
    assert!(TemplateButton::otp(TemplateOtpType::CopyCode)
        .validate()
        .is_err());
    assert!(TemplateButton::otp(TemplateOtpType::CopyCode)
        .with_text("Code kopieren")
        .validate()
        .is_ok());
    assert!(TemplateButton::otp(TemplateOtpType::OneTap)
        .with_autofill_text("Code einsetzen")
        .validate()
        .is_ok());
    assert!(TemplateButton::otp(TemplateOtpType::OneTap)
        .with_text("x".repeat(TEMPLATE_BUTTON_TEXT_MAX_CHARS + 1))
        .validate()
        .is_err());
    assert!(TemplateButton::otp(TemplateOtpType::OneTap)
        .with_text("")
        .validate()
        .is_err());

    // Non-OTP kinds ignore the label setters, so their own label survives.
    let untouched = TemplateButton::quick_reply("Ja")
        .with_text("Nein")
        .with_autofill_text("Egal");
    assert_eq!(untouched, TemplateButton::quick_reply("Ja"));

    let payload = TemplateDefinition::new("otp", "de", TemplateCategory::Authentication)
        .security_recommendation()
        .button(TemplateButton::otp(TemplateOtpType::OneTap).with_text("Autofill"))
        .render()
        .unwrap();
    assert_eq!(payload["components"][1]["buttons"][0]["type"], "OTP");
    assert_eq!(
        payload["components"][1]["buttons"][0]["otp_type"],
        "ONE_TAP"
    );
}

#[test]
fn the_button_count_and_mix_stay_inside_the_limits() {
    let quick_replies = |count: usize| {
        let buttons = (0..count)
            .map(|index| TemplateButton::quick_reply(format!("B{index}")))
            .collect();
        TemplateDefinitionComponent::buttons(buttons).validate()
    };
    assert!(quick_replies(0).is_err());
    assert!(quick_replies(TEMPLATE_BUTTONS_MAX).is_ok());
    assert!(quick_replies(TEMPLATE_BUTTONS_MAX + 1).is_err());

    let url_buttons = |count: usize| {
        let buttons = (0..count)
            .map(|index| TemplateButton::url(format!("U{index}"), "https://x.test"))
            .collect();
        TemplateDefinitionComponent::buttons(buttons).validate()
    };
    assert!(url_buttons(TEMPLATE_URL_BUTTONS_MAX).is_ok());
    assert!(url_buttons(TEMPLATE_URL_BUTTONS_MAX + 1).is_err());

    let copy_codes = |count: usize| {
        let buttons = (0..count)
            .map(|_| TemplateButton::copy_code("Code", "ABC"))
            .collect();
        TemplateDefinitionComponent::buttons(buttons).validate()
    };
    assert!(copy_codes(1).is_ok());
    assert!(copy_codes(2).is_err());

    // Every button kind reaches the wire with its own type name.
    let kinds: Vec<&str> = [
        TemplateButton::quick_reply("a"),
        TemplateButton::url("b", "https://x.test"),
        TemplateButton::copy_code("c", "ABC"),
        TemplateButton::phone_number("d", "+4915112345678"),
        TemplateButton::otp(TemplateOtpType::CopyCode).with_text("e"),
    ]
    .iter()
    .map(TemplateButton::kind)
    .collect();
    assert_eq!(
        kinds,
        ["QUICK_REPLY", "URL", "COPY_CODE", "PHONE_NUMBER", "OTP"]
    );
}

#[test]
fn the_component_set_has_at_most_four_distinct_kinds() {
    assert_eq!(TEMPLATE_COMPONENTS_MAX, 4);

    let full = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .header_text("Kopf")
        .body_text("Text")
        .footer_text("Fuß")
        .button(TemplateButton::quick_reply("Ja"));
    assert_eq!(full.components.len(), TEMPLATE_COMPONENTS_MAX);
    assert!(full.validate().is_ok());

    // A fifth component repeats a kind, which is rejected before the count.
    let too_many = full
        .clone()
        .component(TemplateDefinitionComponent::footer_text("Noch ein Fuß"));
    assert!(too_many.validate().is_err());
    assert!(too_many
        .validate()
        .unwrap_err()
        .to_string()
        .contains("components"));
}

#[test]
fn the_builders_replace_instead_of_duplicating() {
    // A second header replaces the first, including its format.
    let replaced = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .header_media(TemplateHeaderFormat::Image, "4:handle")
        .header_text("Neu");
    assert_eq!(replaced.components.len(), 1);
    assert!(matches!(
        replaced.components[0],
        TemplateDefinitionComponent::Header {
            format: TemplateHeaderFormat::Text,
            ..
        }
    ));

    // Examples accumulate on the component that is already there.
    let examples = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .header_text("{{1}} und {{2}}")
        .header_example("a")
        .header_example("b");
    assert_eq!(examples.components.len(), 1);
    match &examples.components[0] {
        TemplateDefinitionComponent::Header { example, .. } => assert_eq!(example.len(), 2),
        other => panic!("expected a header, got {other:?}"),
    }
    assert!(examples.components[0].validate().is_ok());
    assert!(examples.validate().is_err(), "the body is still missing");

    // A body example without a body leaves the text unset, so validation fails
    // instead of inventing a body.
    let orphan = TemplateDefinition::new("t", "de", TemplateCategory::Utility).body_example("x");
    assert!(orphan.validate().is_err());

    // Buttons accumulate into one BUTTONS component.
    let buttons = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .body_text("Danke")
        .button(TemplateButton::quick_reply("Ja"))
        .button(TemplateButton::quick_reply("Nein"));
    assert_eq!(buttons.components.len(), 2);
    assert_eq!(buttons.components[1].kind(), "BUTTONS");

    // A footer text replaces the code expiration and vice versa.
    let footer = TemplateDefinition::new("t", "de", TemplateCategory::Authentication)
        .security_recommendation()
        .code_expiration_minutes(10)
        .footer_text("Danke");
    assert!(footer.validate().is_ok());
    assert_eq!(footer.render().unwrap()["components"][1]["text"], "Danke");
    assert!(footer.render().unwrap()["components"][1]
        .get("code_expiration_minutes")
        .is_none());

    // A body text replaces the security recommendation.
    let body = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .security_recommendation()
        .body_text("Danke");
    assert_eq!(body.render().unwrap()["components"][0]["text"], "Danke");
}

#[test]
fn the_parameter_format_reaches_the_wire() {
    let payload = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .parameter_format(meta_whatsapp::template::TemplateParameterFormat::Named)
        .body_text("Hallo {{name}}")
        .body_example("Florian")
        .render()
        .unwrap();
    assert_eq!(payload["parameter_format"], "NAMED");

    for (format, expected) in [
        (
            meta_whatsapp::template::TemplateParameterFormat::Positional,
            "POSITIONAL",
        ),
        (
            meta_whatsapp::template::TemplateParameterFormat::Named,
            "NAMED",
        ),
    ] {
        assert_eq!(format.as_str(), expected);
    }
    for (format, expected) in [
        (TemplateHeaderFormat::Text, "TEXT"),
        (TemplateHeaderFormat::Image, "IMAGE"),
        (TemplateHeaderFormat::Video, "VIDEO"),
        (TemplateHeaderFormat::Document, "DOCUMENT"),
        (TemplateHeaderFormat::Location, "LOCATION"),
    ] {
        assert_eq!(format.as_str(), expected);
        assert_eq!(
            format.media_kind().is_some(),
            !matches!(
                format,
                TemplateHeaderFormat::Text | TemplateHeaderFormat::Location
            )
        );
    }
    assert_eq!(
        TemplateHeaderFormat::from(meta_whatsapp::TemplateMediaKind::Video),
        TemplateHeaderFormat::Video
    );
    for (otp, expected) in [
        (TemplateOtpType::CopyCode, "COPY_CODE"),
        (TemplateOtpType::OneTap, "ONE_TAP"),
    ] {
        assert_eq!(otp.as_str(), expected);
    }
}

#[test]
fn an_edit_must_change_something() {
    assert!(TemplateUpdate::new("t", "de").validate().is_err());
    assert!(TemplateUpdate::new("t", "de")
        .category(TemplateCategory::Utility)
        .validate()
        .is_ok());
    assert!(TemplateUpdate::new("t", "de")
        .component(TemplateDefinitionComponent::footer_text("Danke"))
        .validate()
        .is_ok());
    assert!(TemplateUpdate::new("", "de")
        .category(TemplateCategory::Utility)
        .validate()
        .is_err());
    assert!(TemplateUpdate::new("t", "")
        .category(TemplateCategory::Utility)
        .validate()
        .is_err());
    assert!(TemplateUpdate::new("t", "de")
        .component(TemplateDefinitionComponent::body_text("{{1}}"))
        .validate()
        .is_err());

    let payload = TemplateUpdate::new("t", "de")
        .component(TemplateDefinitionComponent::footer_text("Danke"))
        .render()
        .unwrap();
    assert!(
        payload.get("category").is_none(),
        "an unset field is not sent"
    );
}

#[test]
fn every_button_kind_renders_its_wire_shape() {
    let payload = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .body_text("Danke")
        .button(TemplateButton::quick_reply("Ja"))
        .button(TemplateButton::url("Web", "https://x.test/{{1}}").with_example("https://x.test/1"))
        .button(TemplateButton::copy_code("Code", "ABC"))
        .button(TemplateButton::phone_number("Anruf", "+4915112345678"))
        .button(TemplateButton::otp(TemplateOtpType::OneTap).with_autofill_text("Autofill"))
        .render()
        .unwrap();

    let buttons = payload["components"][1]["buttons"].as_array().unwrap();
    assert_eq!(buttons.len(), 5);
    assert_eq!(buttons[0]["type"], "QUICK_REPLY");
    assert_eq!(buttons[1]["type"], "URL");
    assert_eq!(buttons[1]["url"], "https://x.test/{{1}}");
    assert_eq!(buttons[2]["type"], "COPY_CODE");
    assert_eq!(buttons[2]["example"], "ABC");
    assert_eq!(buttons[3]["type"], "PHONE_NUMBER");
    assert_eq!(buttons[3]["phone_number"], "+4915112345678");
    assert_eq!(buttons[4]["type"], "OTP");
    assert_eq!(buttons[4]["autofill_text"], "Autofill");
    assert!(
        buttons[4].get("text").is_none(),
        "an unset label is omitted"
    );
}

#[test]
fn the_label_is_checked_for_every_button_kind() {
    assert!(TemplateButton::url("", "https://x.test")
        .validate()
        .is_err());
    assert!(TemplateButton::copy_code("", "ABC").validate().is_err());
    assert!(TemplateButton::phone_number("", "+4915112345678")
        .validate()
        .is_err());
    assert!(TemplateButton::otp(TemplateOtpType::CopyCode)
        .with_autofill_text("")
        .validate()
        .is_err());

    // The URL example has the URL ceiling, not the copy-code one.
    assert!(TemplateButton::url("Web", "https://x.test/{{1}}")
        .with_example("x".repeat(TEMPLATE_BUTTON_URL_MAX_CHARS + 1))
        .validate()
        .is_err());

    // An invalid button inside a BUTTONS component is reported as well.
    assert!(
        TemplateDefinitionComponent::buttons(vec![TemplateButton::quick_reply("")])
            .validate()
            .is_err()
    );
}

#[test]
fn with_example_reaches_the_copy_code_button_and_ignores_the_rest() {
    let renamed = TemplateButton::copy_code("Code", "OLD").with_example("NEW");
    assert_eq!(renamed, TemplateButton::copy_code("Code", "NEW"));
    assert_eq!(
        TemplateButton::phone_number("Anruf", "+4915112345678").with_example("x"),
        TemplateButton::phone_number("Anruf", "+4915112345678")
    );
    assert_eq!(
        TemplateButton::otp(TemplateOtpType::CopyCode)
            .with_text("a")
            .with_example("x"),
        TemplateButton::otp(TemplateOtpType::CopyCode).with_text("a")
    );
}

#[test]
fn every_media_kind_converts_to_its_header_format() {
    for (kind, format) in [
        (TemplateMediaKind::Image, TemplateHeaderFormat::Image),
        (TemplateMediaKind::Video, TemplateHeaderFormat::Video),
        (TemplateMediaKind::Document, TemplateHeaderFormat::Document),
    ] {
        assert_eq!(TemplateHeaderFormat::from(kind), format);
        assert_eq!(format.media_kind(), Some(kind));
    }
}

#[test]
fn a_header_and_a_footer_can_be_replaced_after_being_set() {
    let replaced_header = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .header_text("Kopf")
        .header_media(TemplateHeaderFormat::Image, "4:handle")
        .body_text("Danke");
    assert!(matches!(
        replaced_header.components[0],
        TemplateDefinitionComponent::Header {
            format: TemplateHeaderFormat::Image,
            ..
        }
    ));
    assert!(replaced_header.validate().is_ok());

    let replaced_footer = TemplateDefinition::new("otp", "de", TemplateCategory::Authentication)
        .security_recommendation()
        .code_expiration_minutes(5)
        .code_expiration_minutes(10);
    assert_eq!(
        replaced_footer.render().unwrap()["components"][1]["code_expiration_minutes"],
        10
    );
}

#[test]
fn a_duplicate_component_kind_is_rejected_on_its_own() {
    let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
        .body_text("Danke")
        .component(TemplateDefinitionComponent::footer_text("a"))
        .component(TemplateDefinitionComponent::footer_text("b"));
    let err = definition.validate().unwrap_err();
    assert!(err.to_string().contains("duplicate FOOTER"), "{err}");
}

#[test]
fn the_list_query_is_unchanged_by_the_new_endpoints() {
    // The WABA-scoped list endpoint keeps its own path and query.
    let query = TemplateQuery::new("102290129340398").limit(2);
    assert_eq!(query.query_string(), "limit=2");
    assert!(query.validate().is_ok());
}

#[tokio::test]
async fn list_templates_still_hits_the_waba_path() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({"data": []})));
    let client = client(transport.clone());

    let page = client
        .list_templates(&TemplateQuery::new("102290129340398").limit(2))
        .await
        .unwrap();
    assert!(page.data.is_empty());
    assert!(transport
        .only_call()
        .url
        .ends_with("/102290129340398/message_templates?limit=2"));
}
