//! Surface sweep: wire spellings, getters and validation boundaries asserted
//! outside the doctests.
//!
//! Doctests run as separate, uninstrumented binaries, so `cargo llvm-cov` does
//! not count them. Every public item is doctested *and* exercised here, which
//! keeps the coverage gate honest and doubles as the edge matrix: for each rule
//! the accepted side, the boundary and the rejected side are named.

mod common;

use common::{client, json_response, response, MockTransport};
use meta_whatsapp::business::{
    BusinessProfile, BusinessProfileUpdate, MessagingLimitTier, PhoneNumberInfo, QualityRating,
    PROFILE_WEBSITE_MAX_CHARS,
};
use meta_whatsapp::media::{MediaBytes, MediaInfo, MediaRef, MediaUpload, MAX_LINK_LEN};
use meta_whatsapp::subscription::{SubscriptionConfig, SubscriptionState, WebhookSubscriptions};
use meta_whatsapp::template::{
    TemplateButtonType, TemplateComponent, TemplateLanguage, TemplateMediaKind, TemplateParameter,
    TemplateQuery,
};
use meta_whatsapp::usage::{BusinessUseCaseUsage, UseCaseUsage};
use meta_whatsapp::{
    AddressMessage, AudioMessage, ButtonMessage, CatalogMessage, ContactCard, ContactName,
    ContactOrg, ContactsMessage, DocumentMessage, EmailType, Error, FlowAction, FlowMessage,
    FlowMode, HeaderText, ImageMessage, InteractiveMessage, ListMessage, ListRow, ListSection,
    LocationMessage, MediaBytes as ReexportedMediaBytes, MessageBody, Order, OrderDetailsMessage,
    OrderItem, OrderStatus, OrderStatusMessage, PhoneType, ProductItem, ProductListMessage,
    ProductMessage, ProductSection, ReactionMessage, RecipientType, ReplyButton, StickerMessage,
    TemplateMessage, TextMessage, UrlType, VideoMessage,
};

#[test]
fn every_body_reports_its_wire_type() {
    let media = || MediaRef::id("1");
    let cases: Vec<(MessageBody, &str)> = vec![
        (TextMessage::new("x").into(), "text"),
        (ImageMessage::new(media()).into(), "image"),
        (VideoMessage::new(media()).into(), "video"),
        (AudioMessage::new(media()).voice(true).into(), "audio"),
        (
            DocumentMessage::new(media()).filename("a.pdf").into(),
            "document",
        ),
        (StickerMessage::new(media()).into(), "sticker"),
        (
            LocationMessage::new(52.5, 13.4).name("x").into(),
            "location",
        ),
        (
            ContactsMessage::new(vec![ContactCard::new("A")]).into(),
            "contacts",
        ),
        (ReactionMessage::new("wamid.1", "👍").into(), "reaction"),
        (
            InteractiveMessage::Buttons(ButtonMessage::new("b", vec![ReplyButton::new("i", "t")]))
                .into(),
            "interactive",
        ),
        (TemplateMessage::new("t", "de").into(), "template"),
        (CatalogMessage::new("p").into(), "catalog"),
        (ProductMessage::new("c", "p").into(), "interactive"),
        (
            ProductListMessage::new("c", vec![ProductSection::new(vec![ProductItem::new("p")])])
                .into(),
            "interactive",
        ),
        (
            OrderDetailsMessage::new(
                "r",
                Order::new("c", vec![OrderItem::new("p", 1, 1.0, "EUR")]),
            )
            .into(),
            "interactive",
        ),
        (
            OrderStatusMessage::new("r", OrderStatus::Shipped).into(),
            "interactive",
        ),
        (AddressMessage::new("DE").into(), "interactive"),
    ];
    for (body, expected) in cases {
        assert_eq!(body.message_type(), expected);
        assert!(body.validate().is_ok(), "{expected} must validate");
    }
}

#[test]
fn enum_spellings_cover_every_variant() {
    assert_eq!(RecipientType::Individual.as_str(), "individual");
    assert_eq!(PhoneType::Cell.as_str(), "CELL");
    assert_eq!(PhoneType::Main.as_str(), "MAIN");
    assert_eq!(PhoneType::Iphone.as_str(), "IPHONE");
    assert_eq!(PhoneType::Home.as_str(), "HOME");
    assert_eq!(PhoneType::Work.as_str(), "WORK");
    assert_eq!(PhoneType::Other.as_str(), "OTHER");
    assert_eq!(EmailType::Work.as_str(), "WORK");
    assert_eq!(EmailType::Home.as_str(), "HOME");
    assert_eq!(EmailType::Other.as_str(), "OTHER");
    assert_eq!(UrlType::Work.as_str(), "WORK");
    assert_eq!(UrlType::Home.as_str(), "HOME");
    assert_eq!(UrlType::Other.as_str(), "OTHER");
    assert_eq!(TemplateMediaKind::Image.as_str(), "image");
    assert_eq!(TemplateMediaKind::Video.as_str(), "video");
    assert_eq!(TemplateMediaKind::Document.as_str(), "document");
    assert_eq!(TemplateButtonType::QuickReply.as_str(), "quick_reply");
    assert_eq!(TemplateButtonType::Url.as_str(), "url");
    assert_eq!(TemplateButtonType::CopyCode.as_str(), "copy_code");
    assert_eq!(FlowMode::Draft.as_str(), "draft");
    assert_eq!(FlowMode::Published.as_str(), "published");
    assert_eq!(OrderStatus::Pending.as_str(), "pending");
    assert_eq!(OrderStatus::Processing.as_str(), "processing");
    assert_eq!(OrderStatus::PartiallyShipped.as_str(), "partially_shipped");
    assert_eq!(OrderStatus::Shipped.as_str(), "shipped");
    assert_eq!(OrderStatus::Completed.as_str(), "completed");
    assert_eq!(OrderStatus::Canceled.as_str(), "canceled");
    assert_eq!(OrderStatus::Cancelled.as_str(), "cancelled");
    assert_eq!(OrderStatus::Unknown("new".into()).as_str(), "new");
    assert_eq!(meta_whatsapp::AddressFieldType::Name.as_str(), "name");
    assert_eq!(meta_whatsapp::AddressFieldType::Address.as_str(), "address");
    assert_eq!(meta_whatsapp::AddressFieldType::Phone.as_str(), "phone");
    assert_eq!(meta_whatsapp::AddressFieldType::Email.as_str(), "email");
    assert_eq!(
        meta_whatsapp::AddressFieldType::Unknown("new".into()).as_str(),
        "new"
    );
    assert!(matches!(
        FlowAction::navigate("screen"),
        FlowAction::Navigate { .. }
    ));
    assert!(matches!(FlowAction::DataExchange, FlowAction::DataExchange));
    assert_eq!(meta_whatsapp::transport::Method::Get.as_str(), "GET");
}

#[test]
fn request_getters_expose_what_was_set() {
    let request = meta_whatsapp::MessageRequest::new("42", TextMessage::new("x").preview_url(true))
        .reply_to("wamid.1")
        .recipient_type(RecipientType::Individual);
    assert_eq!(request.to(), "42");
    assert_eq!(request.reply_to_id(), Some("wamid.1"));
    assert!(matches!(request.body(), MessageBody::Text(_)));
    assert!(request.validate().is_ok());
    assert_eq!(request.to_json().unwrap()["text"]["preview_url"], true);
}

#[test]
fn media_references_expose_their_shape() {
    let by_id = MediaRef::id("42");
    assert_eq!(by_id.as_id(), Some("42"));
    assert_eq!(by_id.as_link(), None);
    assert_eq!(by_id.field_name(), "id");

    let by_link = MediaRef::link("https://x.test/a.png");
    assert_eq!(by_link.as_id(), None);
    assert_eq!(by_link.as_link(), Some("https://x.test/a.png"));
    assert_eq!(by_link.field_name(), "link");
    assert!(by_link.validate().is_ok());
}

#[test]
fn media_links_stop_at_the_documented_boundaries() {
    let ok = format!("https://x.test/{}", "a".repeat(MAX_LINK_LEN - 19));
    assert!(MediaRef::link(&ok).validate().is_ok() || ok.len() > MAX_LINK_LEN);
    // Over the ceiling.
    let long = format!("https://x.test/{}", "a".repeat(MAX_LINK_LEN));
    assert!(MediaRef::link(&long).validate().is_err());
    // Wrong scheme, whitespace, control characters.
    for bad in [
        "http://x.test/a",
        "https://x.test/a b",
        "https://x.test/a\nb",
        "",
    ] {
        assert!(MediaRef::link(bad).validate().is_err(), "{bad:?}");
    }
}

#[test]
fn upload_validation_covers_every_rejection() {
    assert!(MediaUpload::new("a.txt", "text/plain", b"hi".to_vec())
        .unwrap()
        .validate()
        .is_ok());

    // Empty payload, empty filename, bad MIME, oversized payload, header injection.
    assert!(MediaUpload::new("a.txt", "text/plain", Vec::new()).is_err());
    assert!(MediaUpload::new("", "text/plain", b"x".to_vec()).is_err());
    assert!(MediaUpload::new("a.txt", "application/zip", b"x".to_vec()).is_err());
    assert!(MediaUpload::new("a".repeat(300), "text/plain", b"x".to_vec()).is_err());
    assert!(MediaUpload::new("a/../b.txt", "text/plain", b"x".to_vec()).is_err());
    assert!(MediaUpload::new("a\\b.txt", "text/plain", b"x".to_vec()).is_err());

    let upload = MediaUpload::new("a.txt", "text/plain", b"hello".to_vec()).unwrap();
    assert_eq!(upload.len(), 5);
    assert!(!upload.is_empty());
    let (content_type, body) = upload.encode();
    assert!(content_type.starts_with("multipart/form-data; boundary="));
    assert_ne!(body.len(), 0);
}

#[test]
fn media_bytes_and_info_expose_their_fields() {
    let bytes = MediaBytes::new(b"abc".to_vec(), Some("IMAGE/PNG".into()));
    assert_eq!(bytes.len(), 3);
    assert!(!bytes.is_empty());
    assert_eq!(bytes.content_type(), Some("IMAGE/PNG"));
    assert_eq!(bytes.bytes(), b"abc");
    assert_eq!(bytes.clone().into_bytes(), b"abc");

    let info: MediaInfo = serde_json::from_str(
        r#"{"messaging_product":"whatsapp","url":"https://lookaside.fbsbx.com/x","mime_type":"image/png","sha256":"ab","file_size":42,"id":"1"}"#,
    )
    .unwrap();
    assert_eq!(info.file_size, Some(42));
    assert_eq!(info.mime_type.as_deref(), Some("image/png"));
    assert!(MediaBytes::new(Vec::new(), None).is_empty());
}

#[test]
fn template_parameters_cover_every_kind() {
    let cases = vec![
        TemplateParameter::text("hi"),
        TemplateParameter::currency("$1", "USD", 1_000),
        TemplateParameter::date_time("2026-10-24"),
        TemplateParameter::media(MediaRef::id("1"), TemplateMediaKind::Document),
    ];
    for parameter in cases {
        assert!(parameter.validate().is_ok());
    }

    // Boundaries: empty text, over-long text, bad currency code, bad media.
    assert!(TemplateParameter::text("").validate().is_err());
    assert!(TemplateParameter::text("x".repeat(1025))
        .validate()
        .is_err());
    assert!(TemplateParameter::currency("$1", "usd", 1)
        .validate()
        .is_err());
    assert!(TemplateParameter::currency("$1", "USDX", 1)
        .validate()
        .is_err());
    assert!(
        TemplateParameter::media(MediaRef::link("http://x.test"), TemplateMediaKind::Image)
            .validate()
            .is_err()
    );
}

#[test]
fn template_components_and_languages_validate() {
    let header = TemplateComponent::header(vec![TemplateParameter::text("t")]);
    let body = TemplateComponent::body(vec![TemplateParameter::text("b")]);
    let button = TemplateComponent::button(TemplateButtonType::CopyCode, 0, vec![]);
    assert_eq!(header.kind(), "header");
    assert_eq!(body.kind(), "body");
    assert_eq!(button.kind(), "button");
    assert_eq!(header.parameters().len(), 1);
    assert_eq!(button.parameters().len(), 0);

    // Component validation delegates to every parameter.
    assert!(TemplateComponent::body(vec![TemplateParameter::text("")])
        .validate()
        .is_err());

    let language = TemplateLanguage::new("de").policy("deterministic");
    assert_eq!(language.policy.as_deref(), Some("deterministic"));
    // `TemplateLanguage::validate` is internal; it surfaces through the message.
    assert!(TemplateMessage::new("t", "de").validate().is_ok());
    assert!(TemplateMessage::new("t", "").validate().is_err());
    assert!(TemplateMessage::new("t", "x".repeat(33))
        .validate()
        .is_err());
    assert!(TemplateMessage::new("t", "de DE").validate().is_err());
    assert_eq!(TemplateLanguage::new("de").code, "de");
}

#[test]
fn template_queries_cover_every_filter() {
    let query = TemplateQuery::new("1")
        .limit(5)
        .after("a")
        .before("b")
        .status("APPROVED")
        .name("hello_world")
        .language("de")
        .fields(["name", "status"]);
    let rendered = query.query_string();
    for part in [
        "limit=5",
        "after=a",
        "before=b",
        "status=APPROVED",
        "name=hello_world",
        "language=de",
        "fields=name%2Cstatus",
    ] {
        assert!(rendered.contains(part), "{part} missing from {rendered}");
    }
    assert!(query.validate().is_ok());
    assert!(TemplateQuery::new("1").limit(0).validate().is_err());
    assert!(TemplateQuery::new("1").limit(101).validate().is_err());
    assert!(TemplateQuery::new("").validate().is_err());
    assert!(TemplateQuery::new("x".repeat(33)).validate().is_err());
}

#[test]
fn template_messages_cover_every_setter() {
    let template = TemplateMessage::new("hello", "de")
        .language_policy("deterministic")
        .header_text("h")
        .body_text("b")
        .button_text(0, "yes");
    let rendered = template.render().unwrap();
    assert_eq!(rendered["name"], "hello");
    assert!(template.validate().is_ok());
    assert_eq!(rendered["components"].as_array().unwrap().len(), 3);
    // A fourth component is rejected: header/body/button are the three slots.
    assert!(template
        .clone()
        .component(TemplateComponent::button(
            TemplateButtonType::QuickReply,
            1,
            vec![TemplateParameter::text("x")]
        ))
        .validate()
        .is_err());
    assert!(TemplateMessage::new("", "de").validate().is_err());
    assert!(TemplateMessage::new("t", "").validate().is_err());
}

#[test]
fn business_profile_display_and_flags() {
    for (rating, text) in [
        (QualityRating::Green, "GREEN"),
        (QualityRating::Yellow, "YELLOW"),
        (QualityRating::Red, "RED"),
        (QualityRating::Na, "NA"),
        (QualityRating::Unknown("PLATINUM".into()), "PLATINUM"),
    ] {
        assert_eq!(rating.to_string(), text);
        assert_eq!(rating.as_str(), text);
        assert_eq!(
            serde_json::to_string(&rating).unwrap(),
            format!("\"{text}\"")
        );
        assert_eq!(
            rating.permits_sending(),
            !matches!(rating, QualityRating::Red)
        );
    }

    for (tier, text, limit) in [
        (MessagingLimitTier::Tier50, "TIER_50", Some(50)),
        (MessagingLimitTier::Tier250, "TIER_250", Some(250)),
        (MessagingLimitTier::Tier1K, "TIER_1K", Some(1_000)),
        (MessagingLimitTier::Tier10K, "TIER_10K", Some(10_000)),
        (MessagingLimitTier::Tier100K, "TIER_100K", Some(100_000)),
        (MessagingLimitTier::Unlimited, "TIER_UNLIMITED", None),
        (
            MessagingLimitTier::Unknown("TIER_1M".into()),
            "TIER_1M",
            None,
        ),
    ] {
        assert_eq!(tier.to_string(), text);
        assert_eq!(tier.as_str(), text);
        assert_eq!(tier.unique_contacts_per_24h(), limit);
        assert_eq!(tier.is_unlimited(), text == "TIER_UNLIMITED");
    }
}

#[test]
fn business_profile_updates_cover_every_field_and_rejection() {
    let update = BusinessProfileUpdate::new()
        .about("a")
        .address("b")
        .description("c")
        .email("a@b.test")
        .profile_picture_handle("4:handle")
        .website("https://a.test")
        .vertical("RETAIL");
    let payload = update.to_json().unwrap();
    for key in [
        "about",
        "address",
        "description",
        "email",
        "profile_picture_handle",
        "vertical",
    ] {
        assert!(payload.get(key).is_some(), "{key} missing");
    }
    assert_eq!(payload["websites"].as_array().unwrap().len(), 1);

    // Over-long address / description / e-mail / handle / vertical.
    assert!(BusinessProfileUpdate::new()
        .address("a".repeat(257))
        .validate()
        .is_err());
    assert!(BusinessProfileUpdate::new()
        .description("a".repeat(257))
        .validate()
        .is_err());
    assert!(BusinessProfileUpdate::new()
        .email(format!("{}@b", "a".repeat(200)))
        .validate()
        .is_err());
    assert!(BusinessProfileUpdate::new()
        .profile_picture_handle("a".repeat(513))
        .validate()
        .is_err());
    assert!(BusinessProfileUpdate::new()
        .vertical("a".repeat(65))
        .validate()
        .is_err());
    let long_website = format!("https://a.test/{}", "x".repeat(PROFILE_WEBSITE_MAX_CHARS));
    assert!(BusinessProfileUpdate::new()
        .website(&long_website)
        .validate()
        .is_err());
}

#[test]
fn phone_number_helpers_cover_unrated_and_sandbox_numbers() {
    let unrated: PhoneNumberInfo = serde_json::from_str("{}").unwrap();
    assert!(unrated.permits_broadcast(), "no rating is not a bad rating");
    assert_eq!(unrated.daily_contact_limit(), None);
    assert_eq!(unrated.id, None);

    let unlimited: PhoneNumberInfo =
        serde_json::from_str(r#"{"messaging_limit_tier":"TIER_UNLIMITED"}"#).unwrap();
    assert_eq!(unlimited.daily_contact_limit(), None);
    assert!(unlimited.permits_broadcast());

    let sandbox: PhoneNumberInfo =
        serde_json::from_str(r#"{"account_mode":"SANDBOX","quality_rating":"GREEN"}"#).unwrap();
    assert!(!sandbox.permits_broadcast());

    let unknown_quality: PhoneNumberInfo =
        serde_json::from_str(r#"{"quality_rating":"PLATINUM","unknown_key":1}"#).unwrap();
    assert!(unknown_quality.permits_broadcast());
    assert_eq!(unknown_quality.extra["unknown_key"], 1);
}

#[test]
fn profile_parsing_covers_the_error_arms() {
    for value in [
        serde_json::json!({"data": "nope"}),
        serde_json::json!({"data": [1, 2]}),
        serde_json::json!(7),
    ] {
        assert!(matches!(
            BusinessProfile::parse(&value),
            Err(Error::Payload(_))
        ));
    }
    assert!(PhoneNumberInfo::parse(&serde_json::json!({"data": []})).is_err());
    assert!(PhoneNumberInfo::parse(&serde_json::json!({"data": [{}]})).is_ok());
}

#[test]
fn subscription_helpers_cover_their_arms() {
    let subs = WebhookSubscriptions::parse(&serde_json::json!({"data": [{"id": "1"}]})).unwrap();
    assert!(!subs.is_empty());
    assert_eq!(subs.data[0].name, None);
    assert!(matches!(
        WebhookSubscriptions::parse(&serde_json::json!({"data": [1]})),
        Err(Error::Payload(_))
    ));
    assert!(matches!(
        WebhookSubscriptions::parse(&serde_json::json!({"data": "x"})),
        Err(Error::Payload(_))
    ));

    assert!(SubscriptionState::AlreadySubscribed.is_subscribed());
    assert!(!SubscriptionState::AlreadySubscribed.was_written());
    assert!(SubscriptionConfig::new().validate().is_ok());
    assert_eq!(
        SubscriptionConfig::new().to_json().unwrap(),
        serde_json::json!({})
    );
}

#[test]
fn usage_helpers_cover_every_counter() {
    let usage = BusinessUseCaseUsage::from_header(
        r#"{"1":[{"type":"whatsapp","total_cputime":100},{"type":"management","total_time":100,"estimated_time_to_regain_access":9}]}"#,
    )
    .unwrap();
    assert!(usage.is_throttled("1"));
    assert_eq!(usage.businesses().count(), 1);
    assert!(usage.use_case("1", "MANAGEMENT").unwrap().is_throttled());
    assert!(usage.use_case("1", "missing").is_none());
    assert!(!usage.is_throttled("2"));
    assert!(UseCaseUsage::default().use_case.is_none());
}

#[tokio::test]
async fn client_surface_covers_urls_getters_and_downloads() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "id": "42",
        "url": "https://lookaside.fbsbx.com/x",
        "mime_type": "image/png",
        "file_size": 3,
        "sha256": "ab"
    })));
    transport.push_response(response(200, &[("content-type", "image/png")], b"\x89PNG"));
    transport.push_response(response(200, &[], b"raw"));
    transport.push_response(json_response(&serde_json::json!({ "success": true })));

    let client = client(transport.clone());
    assert_eq!(client.phone_number_id(), "106540352242922");
    assert_eq!(client.api_version(), "v21.0");
    assert!(client.timeout().is_none());
    assert_eq!(client.retry().max_attempts, 3);
    assert!(client.media_url("42").ends_with("/42"));
    assert!(client.upload_url().ends_with("/media"));
    assert!(client.templates_url("1").ends_with("/message_templates"));

    let info = client.media_info("42").await.unwrap();
    assert_eq!(info.file_size, Some(3));
    let bytes = client.download_media("42").await.unwrap();
    assert_eq!(bytes.content_type(), Some("image/png"));
    let raw = client
        .download_from_url("https://lookaside.fbsbx.com/x")
        .await
        .unwrap();
    assert_eq!(raw, b"raw");
    assert_eq!(client.delete_media("42").await.unwrap()["success"], true);

    // Rejections that never reach the network.
    assert!(client.download_from_url("http://x.test").await.is_err());
    assert!(client.media_info("").await.is_err());
    assert!(client.download_media("../x").await.is_err());
    assert!(client.delete_media("a/b").await.is_err());
}

#[test]
fn transport_response_accessors() {
    use meta_whatsapp::transport::{Method, Request, Response};

    let response = Response::new(
        201,
        [("Content-Type".to_string(), "application/json".to_string())],
        br#"{"a":1}"#.to_vec(),
    );
    assert_eq!(response.status(), 201);
    assert!(response.is_success());
    assert_eq!(response.header("CONTENT-TYPE"), Some("application/json"));
    assert_eq!(response.header("missing"), None);
    assert_eq!(response.body(), br#"{"a":1}"#);
    assert_eq!(response.json().unwrap()["a"], 1);
    assert!(response.business_use_case_usage().is_none());

    let empty = Response::new(204, std::iter::empty(), Vec::new());
    assert_eq!(empty.json().unwrap(), serde_json::json!({}));
    assert!(empty.is_success(), "204 is a success status");

    let body = serde_json::json!({"a": 1});
    let request = Request::delete_json("https://x.test/a", "t", &body)
        .with_timeout(Some(std::time::Duration::from_secs(1)));
    assert_eq!(request.method, Method::Delete);
    assert_eq!(request.timeout.unwrap().as_secs(), 1);
    assert!(request.body_json().is_some());
}

#[test]
fn contact_cards_cover_every_builder() {
    let card = ContactCard::new("Ada Lovelace")
        .phone("+16505551234", PhoneType::Cell)
        .email("ada@example.test", EmailType::Work)
        .url("https://example.test", UrlType::Home)
        .org(
            ContactOrg::new("Analytical")
                .department("Engines")
                .title("Countess"),
        )
        .birthday("1815-12-10");
    assert!(card.validate().is_ok());

    // The name/phone/e-mail/org validators are internal; they surface through
    // the card and the contacts message.
    assert!(ContactCard::new("").validate().is_err());
    assert!(ContactCard::new("A")
        .phone("", PhoneType::Cell)
        .validate()
        .is_err());
    assert!(ContactCard::new("A")
        .email("", EmailType::Work)
        .validate()
        .is_err());
    assert!(ContactCard::new("A")
        .org(ContactOrg::new(""))
        .validate()
        .is_err());
    assert!(ContactCard::new("A")
        .birthday("x".repeat(11))
        .validate()
        .is_err());
    assert!(ContactsMessage::new(Vec::new()).validate().is_err());
    assert!(ContactsMessage::new(vec![ContactCard::new("A")])
        .validate()
        .is_ok());

    let name = ContactName::new("Ada Lovelace")
        .first_name("Ada")
        .last_name("Lovelace");
    assert_eq!(name.first_name.as_deref(), Some("Ada"));
    assert_eq!(name.last_name.as_deref(), Some("Lovelace"));
}

#[test]
fn interactive_builders_cover_headers_footers_and_sections() {
    let buttons = ButtonMessage::new("body", vec![ReplyButton::new("id", "title")])
        .header(HeaderText::new("h"))
        .footer("f");
    assert!(buttons.validate().is_ok());
    assert!(ButtonMessage::new("body", Vec::new()).validate().is_err());
    assert!(ReplyButton::new("", "t").validate().is_err());
    // `HeaderText::validate` is internal; it surfaces through the message.
    assert!(
        ButtonMessage::new("body", vec![ReplyButton::new("id", "t")])
            .header(HeaderText::new("x".repeat(61)))
            .validate()
            .is_err()
    );

    let list = ListMessage::new("body", "menu")
        .section(ListSection::new(vec![ListRow::new("id", "title").description("d")]).title("s"))
        .header(HeaderText::new("h"))
        .footer("f");
    assert!(list.validate().is_ok());
    assert!(ListMessage::new("body", "menu").validate().is_err());
    assert!(ListRow::new("", "t").validate().is_err());

    let flow = FlowMessage::new("flow-id", "open", FlowAction::navigate("screen"))
        .body("b")
        .flow_token("token")
        .header(HeaderText::new("h"))
        .footer("f")
        .mode(FlowMode::Published);
    assert!(flow.validate().is_ok());
    assert!(FlowMessage::new("", "open", FlowAction::navigate("s"))
        .validate()
        .is_err());
}

#[test]
fn location_and_text_boundaries() {
    assert!(LocationMessage::new(90.0, 180.0).validate().is_ok());
    assert!(LocationMessage::new(-90.0, -180.0).validate().is_ok());
    assert!(LocationMessage::new(90.1, 0.0).validate().is_err());
    assert!(LocationMessage::new(0.0, 180.1).validate().is_err());
    assert!(LocationMessage::new(f64::NAN, 0.0).validate().is_err());

    assert!(TextMessage::new("x".repeat(4096)).validate().is_ok());
    assert!(TextMessage::new("x".repeat(4097)).validate().is_err());
    assert!(DocumentMessage::new(MediaRef::id("1"))
        .filename("x".repeat(241))
        .validate()
        .is_err());
    assert!(ReactionMessage::new("", "👍").validate().is_err());

    // The re-exported alias is the same type.
    let bytes: ReexportedMediaBytes = MediaBytes::new(Vec::new(), None);
    assert!(bytes.is_empty());
}
