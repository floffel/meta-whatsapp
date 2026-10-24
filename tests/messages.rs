//! Every outbound message family: what lands on the wire, and what is refused
//! before the request is built.

mod common;

use common::{accepted, client, instant_retry, Body, Call, MockTransport};
use meta_whatsapp::{
    AudioMessage, ButtonMessage, ContactCard, ContactOrg, ContactsMessage, DocumentMessage,
    EmailType, FlowAction, FlowMessage, FlowMode, HeaderText, ImageMessage, InteractiveMessage,
    ListMessage, ListRow, ListSection, LocationMessage, MediaRef, MessageBody, MessageRequest,
    PhoneType, ReactionMessage, ReplyButton, StickerMessage, TextMessage, UrlType, VideoMessage,
};
use serde_json::{json, Value};

fn json_body(call: &Call) -> &Value {
    match &call.body {
        Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

#[allow(clippy::needless_pass_by_value)] // the request is consumed by to_json semantics
fn assert_rejected(request: MessageRequest, field: &str) {
    let err = request.to_json().unwrap_err();
    match err {
        meta_whatsapp::Error::Validation {
            field: actual,
            message,
        } => {
            assert_eq!(actual, field, "wrong field ({message})");
        }
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn text_message_hits_the_messages_endpoint() {
    let transport = MockTransport::new();
    transport.push_response(accepted("wamid.1"));
    let client = client(transport.clone()).with_retry(instant_retry(1));

    let response = client.send_text("4915112345678", "Hallo").await.unwrap();
    assert_eq!(response["messages"][0]["id"], "wamid.1");

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/messages"
    );
    assert_eq!(call.token, "EAAG-test-token");

    let body = json_body(&call);
    assert_eq!(body["messaging_product"], "whatsapp");
    assert_eq!(body["recipient_type"], "individual");
    assert_eq!(body["to"], "4915112345678");
    assert_eq!(body["type"], "text");
    assert_eq!(body["text"]["body"], "Hallo");
    assert_eq!(body["text"]["preview_url"], false);
    assert!(body.get("context").is_none());
    assert!(body.get("biz_opaque_callback_data").is_none());
}

#[tokio::test]
async fn reply_context_and_callback_data_are_top_level_keys() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    client
        .send(
            &MessageRequest::new("4915112345678", TextMessage::new("Ja"))
                .reply_to("wamid.inbound")
                .biz_opaque_callback_data("ticket-42"),
        )
        .await
        .unwrap();

    let body = json_body(&transport.only_call()).clone();
    assert_eq!(body["context"], json!({"message_id": "wamid.inbound"}));
    assert_eq!(body["biz_opaque_callback_data"], "ticket-42");
}

#[tokio::test]
async fn image_by_id_and_by_link_render_the_right_key() {
    for (reference, key, value) in [
        (MediaRef::id("987654321"), "id", "987654321"),
        (
            MediaRef::link("https://cdn.example.com/cat.png"),
            "link",
            "https://cdn.example.com/cat.png",
        ),
    ] {
        let transport = MockTransport::new();
        let client = client(transport.clone());
        let message = ImageMessage::new(reference).caption("Katze");
        client
            .send(&MessageRequest::new("4915112345678", message))
            .await
            .unwrap();

        let body = json_body(&transport.only_call()).clone();
        assert_eq!(body["type"], "image");
        assert_eq!(body["image"][key], value);
        assert_eq!(body["image"]["caption"], "Katze");
        assert!(body["image"].get("preview_url").is_none());
    }
}

#[tokio::test]
async fn video_audio_document_and_sticker_render_their_blocks() {
    let cases: Vec<(MessageBody, &str)> = vec![
        (
            VideoMessage::new(MediaRef::id("1")).caption("Clip").into(),
            "video",
        ),
        (
            AudioMessage::new(MediaRef::id("2")).voice(true).into(),
            "audio",
        ),
        (
            DocumentMessage::new(MediaRef::id("3"))
                .filename("Rechnung.pdf")
                .caption("Oktober")
                .into(),
            "document",
        ),
        (StickerMessage::new(MediaRef::id("4")).into(), "sticker"),
    ];

    for (body, kind) in cases {
        let transport = MockTransport::new();
        let client = client(transport.clone());
        client
            .send(&MessageRequest::new("4915112345678", body))
            .await
            .unwrap();

        let sent = json_body(&transport.only_call()).clone();
        assert_eq!(sent["type"], kind);
        let expected_id = match kind {
            "video" => "1",
            "audio" => "2",
            "document" => "3",
            _ => "4",
        };
        assert_eq!(sent[kind]["id"], expected_id);
        match kind {
            "video" => assert_eq!(sent["video"]["caption"], "Clip"),
            "audio" => assert_eq!(sent["audio"]["voice"], true),
            "document" => {
                assert_eq!(sent["document"]["filename"], "Rechnung.pdf");
                assert_eq!(sent["document"]["caption"], "Oktober");
            }
            _ => assert!(sent["sticker"].get("caption").is_none()),
        }
    }
}

#[tokio::test]
async fn location_renders_coordinates_and_optional_text() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    client
        .send(&MessageRequest::new(
            "1",
            LocationMessage::new(48.7665, 11.4258)
                .name("Hauptbahnhof")
                .address("Bahnhofplatz 1"),
        ))
        .await
        .unwrap();

    let body = json_body(&transport.only_call()).clone();
    assert_eq!(body["location"]["latitude"], json!(48.7665));
    assert_eq!(body["location"]["longitude"], json!(11.4258));
    assert_eq!(body["location"]["name"], "Hauptbahnhof");
    assert_eq!(body["location"]["address"], "Bahnhofplatz 1");
}

#[tokio::test]
async fn contact_cards_render_name_phone_email_org_and_url() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let card = ContactCard::new("Florian Floffel")
        .phone("+4915112345678", PhoneType::Cell)
        .email("florian@example.com", EmailType::Work)
        .url("https://floffel.dev", UrlType::Work)
        .org(ContactOrg::new("Acme").department("R&D").title("Engineer"))
        .birthday("1990-01-01");

    client
        .send(&MessageRequest::new("1", ContactsMessage::new(vec![card])))
        .await
        .unwrap();

    let body = json_body(&transport.only_call()).clone();
    assert_eq!(
        body["contacts"][0]["name"]["formatted_name"],
        "Florian Floffel"
    );
    assert_eq!(body["contacts"][0]["phones"][0]["phone"], "+4915112345678");
    assert_eq!(body["contacts"][0]["phones"][0]["type"], "CELL");
    assert_eq!(body["contacts"][0]["emails"][0]["type"], "WORK");
    assert_eq!(body["contacts"][0]["org"]["company"], "Acme");
    assert_eq!(body["contacts"][0]["urls"][0]["url"], "https://floffel.dev");
    assert_eq!(body["contacts"][0]["birthday"], "1990-01-01");
}

#[tokio::test]
async fn reaction_can_be_added_and_removed() {
    for (emoji, expected) in [("👍", "👍"), ("", "")] {
        let transport = MockTransport::new();
        let client = client(transport.clone());
        client
            .send(&MessageRequest::new(
                "1",
                ReactionMessage::new("wamid.abc", emoji),
            ))
            .await
            .unwrap();

        let body = json_body(&transport.only_call()).clone();
        assert_eq!(body["type"], "reaction");
        assert_eq!(body["reaction"]["message_id"], "wamid.abc");
        assert_eq!(body["reaction"]["emoji"], expected);
        // A reaction carries no text/caption leftovers.
        assert!(body["reaction"].get("caption").is_none());
    }
}

#[tokio::test]
async fn buttons_render_header_body_footer_and_action() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let buttons = ButtonMessage::new(
        "Möchten Sie bestellen?",
        vec![
            ReplyButton::new("yes", "Ja"),
            ReplyButton::new("no", "Nein"),
        ],
    )
    .header(HeaderText::new("Bestellung"))
    .footer("Antworten Sie mit einem Tipp");

    client
        .send(&MessageRequest::new("1", buttons))
        .await
        .unwrap();

    let body = json_body(&transport.only_call()).clone();
    let interactive = &body["interactive"];
    assert_eq!(interactive["type"], "button");
    assert_eq!(interactive["header"]["type"], "text");
    assert_eq!(interactive["header"]["text"], "Bestellung");
    assert_eq!(interactive["body"]["text"], "Möchten Sie bestellen?");
    assert_eq!(
        interactive["footer"]["text"],
        "Antworten Sie mit einem Tipp"
    );
    assert_eq!(interactive["action"]["buttons"][0]["type"], "reply");
    assert_eq!(interactive["action"]["buttons"][1]["reply"]["id"], "no");
}

#[tokio::test]
async fn list_renders_sections_and_button_label() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let list = ListMessage::new("Wählen Sie ein Gericht", "Speisekarte").section(
        ListSection::new(vec![
            ListRow::new("pizza", "Pizza").description("Mit Salami"),
            ListRow::new("salat", "Salat"),
        ])
        .title("Beliebt"),
    );

    client.send(&MessageRequest::new("1", list)).await.unwrap();

    let body = json_body(&transport.only_call()).clone();
    let interactive = &body["interactive"];
    assert_eq!(interactive["type"], "list");
    assert_eq!(interactive["action"]["button"], "Speisekarte");
    assert_eq!(interactive["action"]["sections"][0]["title"], "Beliebt");
    assert_eq!(
        interactive["action"]["sections"][0]["rows"][0]["description"],
        "Mit Salami"
    );
}

#[tokio::test]
async fn flow_renders_action_parameters() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let flow = FlowMessage::new("flow-1", "Buchen", FlowAction::navigate("WELCOME"))
        .flow_token("token-abc")
        .mode(FlowMode::Draft);

    client.send(&MessageRequest::new("1", flow)).await.unwrap();

    let body = json_body(&transport.only_call()).clone();
    let parameters = &body["interactive"]["action"]["parameters"];
    assert_eq!(body["interactive"]["type"], "flow");
    assert_eq!(body["interactive"]["action"]["name"], "flow");
    assert_eq!(parameters["flow_id"], "flow-1");
    assert_eq!(parameters["flow_cta"], "Buchen");
    assert_eq!(parameters["flow_token"], "token-abc");
    assert_eq!(parameters["flow_action"], "navigate");
    assert_eq!(parameters["flow_action_payload"]["screen"], "WELCOME");
    assert_eq!(parameters["mode"], "draft");
}

#[tokio::test]
async fn template_renders_name_language_and_components() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let template = meta_whatsapp::TemplateMessage::new("order_update", "de")
        .header_text("Bestellung 42")
        .body_text("Florian")
        .body_text("42")
        .button_text(0, "Details");

    client
        .send(&MessageRequest::new("1", template))
        .await
        .unwrap();

    let body = json_body(&transport.only_call()).clone();
    assert_eq!(body["type"], "template");
    assert_eq!(body["template"]["name"], "order_update");
    assert_eq!(body["template"]["language"]["code"], "de");
    assert_eq!(
        body["template"]["components"][0]["parameters"][0]["text"],
        "Bestellung 42"
    );
    assert_eq!(
        body["template"]["components"][1]["parameters"][1]["text"],
        "42"
    );
    assert_eq!(
        body["template"]["components"][2]["parameters"][0]["type"],
        "payload"
    );
}

// ---------------------------------------------------------------------------
// Boundary matrix: just valid, just invalid.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn text_length_boundary_is_4096_bytes() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    client
        .send_text("1", &"a".repeat(4096))
        .await
        .expect("4096 bytes is the limit and must be accepted");

    let err = client.send_text("1", &"a".repeat(4097)).await.unwrap_err();
    assert!(matches!(
        err,
        meta_whatsapp::Error::Validation {
            field: "text.body",
            ..
        }
    ));
    assert_eq!(
        transport.call_count(),
        1,
        "the invalid message was not sent"
    );
}

#[test]
fn empty_text_is_rejected_before_any_request() {
    assert_rejected(MessageRequest::new("1", TextMessage::new("")), "text.body");
}

#[test]
fn recipient_must_be_digits_only() {
    assert_rejected(
        MessageRequest::new("+49 151 12345678", TextMessage::new("hi")),
        "to",
    );
    assert_rejected(MessageRequest::new("", TextMessage::new("hi")), "to");
    assert_rejected(
        MessageRequest::new("1".repeat(33), TextMessage::new("hi")),
        "to",
    );
    assert!(MessageRequest::new("1".repeat(32), TextMessage::new("hi"))
        .to_json()
        .is_ok());
}

#[test]
fn media_ids_cannot_escape_the_path() {
    for bad in ["", "../me", "12/34", "a b", "id?x=1", "id#frag", "id%2f"] {
        assert_rejected(
            MessageRequest::new("1", StickerMessage::new(MediaRef::id(bad))),
            "media_id",
        );
    }
}

#[test]
fn media_links_must_be_https_and_whitespace_free() {
    for bad in [
        "http://example.com/a.png",
        "https://",
        "https://example.com/a b.png",
    ] {
        assert_rejected(
            MessageRequest::new("1", ImageMessage::new(MediaRef::link(bad))),
            "media.link",
        );
    }
}

#[test]
fn captions_and_filenames_are_bounded() {
    assert!(MessageRequest::new(
        "1",
        ImageMessage::new(MediaRef::id("1")).caption("c".repeat(1024))
    )
    .to_json()
    .is_ok());
    assert_rejected(
        MessageRequest::new(
            "1",
            ImageMessage::new(MediaRef::id("1")).caption("c".repeat(1025)),
        ),
        "image.caption",
    );
    assert_rejected(
        MessageRequest::new(
            "1",
            DocumentMessage::new(MediaRef::id("1")).filename("f".repeat(241)),
        ),
        "document.filename",
    );
    assert_rejected(
        MessageRequest::new(
            "1",
            DocumentMessage::new(MediaRef::id("1")).filename("bad\r\nname.pdf"),
        ),
        "document.filename",
    );
}

#[test]
fn coordinates_are_range_checked() {
    assert!(
        MessageRequest::new("1", LocationMessage::new(-90.0, -180.0))
            .to_json()
            .is_ok()
    );
    assert!(MessageRequest::new("1", LocationMessage::new(90.0, 180.0))
        .to_json()
        .is_ok());
    for (lat, lon) in [
        (90.000_001, 0.0),
        (0.0, 180.000_001),
        (f64::NAN, 0.0),
        (0.0, f64::INFINITY),
    ] {
        assert_rejected(
            MessageRequest::new("1", LocationMessage::new(lat, lon)),
            if lat.abs() > 90.0 || !lat.is_finite() {
                "location.latitude"
            } else {
                "location.longitude"
            },
        );
    }
}

#[test]
fn button_limits_are_one_to_three() {
    let three = vec![
        ReplyButton::new("a", "A"),
        ReplyButton::new("b", "B"),
        ReplyButton::new("c", "C"),
    ];
    assert!(
        MessageRequest::new("1", ButtonMessage::new("body", three.clone()))
            .to_json()
            .is_ok()
    );
    assert_rejected(
        MessageRequest::new("1", ButtonMessage::new("body", Vec::new())),
        "interactive.action.buttons",
    );
    let mut four = three;
    four.push(ReplyButton::new("d", "D"));
    assert_rejected(
        MessageRequest::new("1", ButtonMessage::new("body", four)),
        "interactive.action.buttons",
    );
    assert_rejected(
        MessageRequest::new(
            "1",
            ButtonMessage::new("body", vec![ReplyButton::new("a", "123456789012345678901")]),
        ),
        "interactive.action.buttons.title",
    );
    assert!(MessageRequest::new(
        "1",
        ButtonMessage::new("body", vec![ReplyButton::new("a", "12345678901234567890")])
    )
    .to_json()
    .is_ok());
}

#[test]
fn list_row_limit_is_ten_in_total() {
    let rows = |n: usize| {
        (0..n)
            .map(|index| ListRow::new(format!("r{index}"), "R"))
            .collect::<Vec<_>>()
    };
    let ok = ListMessage::new("body", "btn")
        .section(ListSection::new(rows(5)))
        .section(ListSection::new(rows(5)));
    assert!(MessageRequest::new("1", ok).to_json().is_ok());

    let too_many = ListMessage::new("body", "btn")
        .section(ListSection::new(rows(10)))
        .section(ListSection::new(rows(1)));
    assert_rejected(
        MessageRequest::new("1", too_many),
        "interactive.action.sections.rows",
    );

    assert_rejected(
        MessageRequest::new("1", ListMessage::new("body", "btn")),
        "interactive.action.sections",
    );
}

#[test]
fn interactive_enum_dispatches_to_each_variant() {
    let buttons =
        InteractiveMessage::Buttons(ButtonMessage::new("b", vec![ReplyButton::new("i", "t")]));
    assert_eq!(buttons.kind(), "button");
    assert!(MessageRequest::new("1", buttons).to_json().is_ok());

    let flow = InteractiveMessage::Flow(FlowMessage::new("f", "go", FlowAction::DataExchange));
    assert_eq!(flow.kind(), "flow");
    assert!(MessageRequest::new("1", flow).to_json().is_ok());
}

#[test]
fn message_type_matches_the_body_variant() {
    assert_eq!(
        MessageBody::Text(TextMessage::new("x")).message_type(),
        "text"
    );
    assert_eq!(
        MessageBody::Audio(AudioMessage::new(MediaRef::id("1"))).message_type(),
        "audio"
    );
    assert_eq!(
        MessageBody::from(ContactsMessage::new(vec![ContactCard::new("A")])).message_type(),
        "contacts"
    );
}

#[tokio::test]
async fn validation_happens_before_the_transport_runs() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let err = client
        .send(&MessageRequest::new("1", TextMessage::new("")))
        .await
        .unwrap_err();
    assert!(matches!(err, meta_whatsapp::Error::Validation { .. }));
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn empty_responses_are_treated_as_an_empty_object() {
    let transport = MockTransport::new();
    transport.push_response(common::response(202, &[], b""));
    let client = client(transport.clone());
    let value = client.send_text("1", "hi").await.unwrap();
    assert_eq!(value, json!({}));
}
