//! Commerce messages: catalog, product, order and address wire shapes, plus
//! every documented limit's accepted and rejected side.

mod common;

use common::{client, Body, Call, MockTransport};
use meta_whatsapp::commerce::{
    ADDRESS_FIELD_NAME_MAX_CHARS, ADDRESS_ID_MAX_CHARS, ADDRESS_SAVED_MAX, ADDRESS_VALUES_MAX,
    CATALOG_ID_MAX_CHARS, ORDER_ITEMS_MAX, ORDER_REFERENCE_ID_MAX_CHARS, PRODUCTS_MAX,
    PRODUCT_RETAILER_ID_MAX_CHARS, PRODUCT_SECTIONS_MAX, PRODUCT_SECTION_TITLE_MAX_CHARS,
};
use meta_whatsapp::{
    AddressField, AddressFieldType, AddressMessage, CatalogMessage, Error, HeaderText,
    InteractiveMessage, MessageBody, MessageRequest, Order, OrderDetailsMessage, OrderItem,
    OrderStatus, OrderStatusMessage, ProductItem, ProductListMessage, ProductMessage,
    ProductSection, SavedAddress, WebhookPayload,
};
use serde_json::Value;

fn json_body(call: &Call) -> &Value {
    match &call.body {
        Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

fn request(body: impl Into<MessageBody>) -> MessageRequest {
    MessageRequest::new("1", body)
}

fn assert_rejected(body: impl Into<MessageBody>, field: &str) {
    match request(body).to_json() {
        Err(Error::Validation { field: actual, .. }) => assert_eq!(actual, field),
        other => panic!("expected a validation error for {field}, got {other:?}"),
    }
}

fn catalog() -> CatalogMessage {
    CatalogMessage::new("sku-thumb")
}

fn product() -> ProductMessage {
    ProductMessage::new("catalog-1", "sku-1")
}

fn one_product_section() -> ProductSection {
    ProductSection::new(vec![ProductItem::new("sku-1")]).title("Coffee")
}

fn item(index: usize) -> OrderItem {
    OrderItem::new(format!("sku-{index}"), 1, 4.0, "EUR")
}

fn order() -> Order {
    Order::new("catalog-1", Vec::new())
        .item(OrderItem::new("sku-1", 2, 4.5, "EUR"))
        .text("Two coffees")
}

#[tokio::test]
async fn catalog_renders_its_thumbnail_product() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    client
        .send(&request(catalog()))
        .await
        .expect("catalog should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["type"], "catalog");
    assert_eq!(
        body["catalog"]["thumbnail_product_retailer_id"],
        "sku-thumb"
    );
}

#[tokio::test]
async fn single_product_renders_body_footer_and_action() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    client
        .send(&request(
            product().body("Single origin").footer("Ships tomorrow"),
        ))
        .await
        .expect("product should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["type"], "interactive");
    assert_eq!(body["interactive"]["type"], "product");
    assert_eq!(body["interactive"]["body"]["text"], "Single origin");
    assert_eq!(body["interactive"]["footer"]["text"], "Ships tomorrow");
    assert_eq!(body["interactive"]["action"]["catalog_id"], "catalog-1");
    assert_eq!(
        body["interactive"]["action"]["product_retailer_id"],
        "sku-1"
    );
}

#[tokio::test]
async fn product_list_renders_header_sections_and_items() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let message = ProductListMessage::new("catalog-1", Vec::new())
        .section(one_product_section())
        .header(HeaderText::new("Menu"))
        .body("Choose a coffee")
        .footer("Prices include VAT");

    client
        .send(&request(message))
        .await
        .expect("product list should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["interactive"]["type"], "product_list");
    assert_eq!(body["interactive"]["header"]["text"], "Menu");
    assert_eq!(body["interactive"]["body"]["text"], "Choose a coffee");
    assert_eq!(body["interactive"]["footer"]["text"], "Prices include VAT");
    assert_eq!(body["interactive"]["action"]["catalog_id"], "catalog-1");
    assert_eq!(
        body["interactive"]["action"]["sections"][0]["title"],
        "Coffee"
    );
    assert_eq!(
        body["interactive"]["action"]["sections"][0]["product_items"][0]["product_retailer_id"],
        "sku-1"
    );
}

#[tokio::test]
async fn order_details_renders_reference_and_line_items() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let message = OrderDetailsMessage::new("order-42", order()).body("Thanks!");

    client
        .send(&request(message))
        .await
        .expect("order details should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["interactive"]["type"], "order_details");
    assert_eq!(body["interactive"]["action"]["name"], "review_order");
    assert_eq!(
        body["interactive"]["action"]["parameters"]["reference_id"],
        "order-42"
    );
    assert_eq!(
        body["interactive"]["action"]["parameters"]["order"]["catalog_id"],
        "catalog-1"
    );
    assert_eq!(
        body["interactive"]["action"]["parameters"]["order"]["text"],
        "Two coffees"
    );
    assert_eq!(
        body["interactive"]["action"]["parameters"]["order"]["product_items"][0]["quantity"],
        2
    );
}

#[tokio::test]
async fn order_status_renders_known_and_unknown_statuses() {
    for status in [
        OrderStatus::Shipped,
        OrderStatus::Unknown("weather_delay".to_string()),
    ] {
        let transport = MockTransport::new();
        let client = client(transport.clone());
        let message =
            OrderStatusMessage::new("order-42", status.clone()).description("Arrives tomorrow");

        client
            .send(&request(message))
            .await
            .expect("order status should send");

        let call = transport.only_call();
        let body = json_body(&call);
        assert_eq!(body["interactive"]["type"], "order_status");
        assert_eq!(body["interactive"]["action"]["name"], "order_status");
        assert_eq!(
            body["interactive"]["action"]["parameters"]["order"]["status"],
            status.as_str()
        );
        assert_eq!(
            body["interactive"]["action"]["parameters"]["order"]["description"],
            "Arrives tomorrow"
        );
    }
}

#[tokio::test]
async fn address_renders_country_values_and_saved_addresses() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let message = AddressMessage::new("DE")
        .body("Where should we deliver?")
        .field(AddressFieldType::Name, "name", "Ada")
        .field(AddressFieldType::Address, "city", "Berlin")
        .saved_address(
            SavedAddress::new("home")
                .field("city", "Berlin")
                .field("zip", "10115"),
        );

    client
        .send(&request(message))
        .await
        .expect("address should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["interactive"]["type"], "address_message");
    assert_eq!(body["interactive"]["action"]["name"], "address_message");
    let parameters = &body["interactive"]["action"]["parameters"];
    assert_eq!(parameters["country"], "DE");
    assert_eq!(parameters["values"][0]["type"], "name");
    assert_eq!(parameters["values"][1]["name"], "city");
    assert_eq!(parameters["saved_addresses"][0]["id"], "home");
    assert_eq!(parameters["saved_addresses"][0]["value"]["zip"], "10115");
}

#[test]
fn interactive_enum_reports_each_commerce_kind() {
    let cases = [
        (InteractiveMessage::Product(product()), "product"),
        (
            InteractiveMessage::ProductList(ProductListMessage::new(
                "catalog-1",
                vec![one_product_section()],
            )),
            "product_list",
        ),
        (
            InteractiveMessage::OrderDetails(OrderDetailsMessage::new("order-42", order())),
            "order_details",
        ),
        (
            InteractiveMessage::OrderStatus(OrderStatusMessage::new(
                "order-42",
                OrderStatus::Processing,
            )),
            "order_status",
        ),
        (
            InteractiveMessage::Address(AddressMessage::new("DE")),
            "address_message",
        ),
    ];
    for (message, kind) in cases {
        assert_eq!(message.kind(), kind);
        assert!(message.validate().is_ok());
    }
}

#[test]
fn inbound_order_and_address_parse_with_unknown_fields() {
    let json = r#"{
      "object": "whatsapp_business_account",
      "entry": [{"id": "1", "changes": [{"field": "messages", "value": {
        "metadata": {"display_phone_number": "1", "phone_number_id": "2"},
        "messages": [
          {"id": "wamid.order", "from": "3", "timestamp": "10", "type": "order",
           "order": {"catalog_id": "catalog-1", "text": "Two coffees",
                     "product_items": [{"product_retailer_id": "sku-1", "quantity": "2",
                                        "item_price": "4.50", "currency": "EUR",
                                        "future": true}],
                     "future_order": "kept"}},
          {"id": "wamid.address", "from": "3", "timestamp": "11", "type": "interactive",
           "interactive": {"type": "address_message", "address_message": {
             "country": "DE",
             "values": [{"type": "address", "name": "city", "value": "Berlin",
                         "future": "kept"}],
             "future_address": "kept"}}},
          {"id": "wamid.address2", "from": "3", "timestamp": "12", "type": "address",
           "address": {"country": "DE", "values": [{"type": "name", "name": "name", "value": "Ada"}]}}
        ]
      }}]}]
    }"#;
    let payload: WebhookPayload = serde_json::from_str(json).unwrap();
    let messages: Vec<_> = payload
        .inbound_messages()
        .map(|(_, message)| message)
        .collect();
    assert_eq!(messages.len(), 3);

    let order = messages[0].order().expect("order block");
    assert_eq!(order.catalog_id, "catalog-1");
    assert_eq!(order.text.as_deref(), Some("Two coffees"));
    assert_eq!(order.product_items[0].quantity, 2);
    assert!((order.product_items[0].item_price - 4.5).abs() < f64::EPSILON);
    assert_eq!(order.product_items[0].extra["future"], true);
    assert_eq!(order.extra["future_order"], "kept");

    let address = messages[1].address().expect("interactive address");
    assert_eq!(address.country.as_deref(), Some("DE"));
    assert_eq!(address.field("city"), Some("Berlin"));
    assert_eq!(address.values[0].extra["future"], "kept");
    assert_eq!(address.extra["future_address"], "kept");
    assert_eq!(
        messages[2].address().and_then(|reply| reply.field("name")),
        Some("Ada")
    );

    let rendered = serde_json::to_string(&payload).unwrap();
    let reparsed: WebhookPayload = serde_json::from_str(&rendered).unwrap();
    assert_eq!(payload, reparsed);
}

#[test]
fn enum_wire_spellings_round_trip() {
    for status in [
        OrderStatus::Pending,
        OrderStatus::Processing,
        OrderStatus::PartiallyShipped,
        OrderStatus::Shipped,
        OrderStatus::Completed,
        OrderStatus::Canceled,
        OrderStatus::Cancelled,
        OrderStatus::Unknown("new".into()),
    ] {
        assert_eq!(OrderStatus::from_wire(status.as_str()), status);
    }
    for field_type in [
        AddressFieldType::Name,
        AddressFieldType::Address,
        AddressFieldType::Phone,
        AddressFieldType::Email,
        AddressFieldType::Unknown("nickname".into()),
    ] {
        assert_eq!(AddressFieldType::from_wire(field_type.as_str()), field_type);
    }
}

// ---------------------------------------------------------------------------
// Boundary matrix: just valid, just invalid.
// ---------------------------------------------------------------------------

#[test]
fn product_and_catalog_ids_stop_at_100_characters() {
    let id = "a".repeat(PRODUCT_RETAILER_ID_MAX_CHARS);
    assert!(request(CatalogMessage::new(&id)).to_json().is_ok());
    assert!(request(ProductMessage::new(&id, &id)).to_json().is_ok());
    assert_rejected(
        CatalogMessage::new("a".repeat(PRODUCT_RETAILER_ID_MAX_CHARS + 1)),
        "catalog.thumbnail_product_retailer_id",
    );
    assert_rejected(
        ProductMessage::new("a".repeat(CATALOG_ID_MAX_CHARS + 1), "sku-1"),
        "interactive.action.catalog_id",
    );
}

#[test]
fn interactive_body_stops_at_1024_bytes() {
    assert!(request(product().body("a".repeat(1024))).to_json().is_ok());
    assert_rejected(product().body("a".repeat(1025)), "interactive.body.text");
    let order = Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 1.0, "EUR")])
        .text("t".repeat(1025));
    assert_rejected(
        OrderDetailsMessage::new("order-42", order),
        "interactive.action.parameters.order.text",
    );
}

#[test]
fn headers_and_footers_stop_at_60_characters() {
    let kit = || vec![one_product_section()];
    assert!(request(
        ProductListMessage::new("catalog-1", kit())
            .header(HeaderText::new("h".repeat(60)))
            .footer("f".repeat(60)),
    )
    .to_json()
    .is_ok());
    assert_rejected(
        ProductListMessage::new("catalog-1", kit()).header(HeaderText::new("h".repeat(61))),
        "interactive.header.text",
    );
    assert_rejected(
        ProductListMessage::new("catalog-1", kit()).footer("f".repeat(61)),
        "interactive.footer.text",
    );
}

#[test]
fn product_sections_and_products_stop_at_ten_and_thirty() {
    assert_rejected(
        ProductListMessage::new("catalog-1", Vec::new()),
        "interactive.action.sections",
    );
    assert_rejected(
        ProductListMessage::new("catalog-1", vec![ProductSection::new(Vec::new())]),
        "interactive.action.sections.product_items",
    );

    let ten_sections = (0..PRODUCT_SECTIONS_MAX)
        .map(|index| ProductSection::new(vec![ProductItem::new(format!("sku-{index}"))]))
        .collect::<Vec<_>>();
    assert!(request(ProductListMessage::new("catalog-1", ten_sections))
        .to_json()
        .is_ok());
    let eleven_sections = (0..=PRODUCT_SECTIONS_MAX)
        .map(|index| ProductSection::new(vec![ProductItem::new(format!("sku-{index}"))]))
        .collect::<Vec<_>>();
    assert_rejected(
        ProductListMessage::new("catalog-1", eleven_sections),
        "interactive.action.sections",
    );

    let thirty = vec![ProductSection::new(
        (0..PRODUCTS_MAX)
            .map(|index| ProductItem::new(format!("sku-{index}")))
            .collect(),
    )];
    assert!(request(ProductListMessage::new("catalog-1", thirty))
        .to_json()
        .is_ok());
    let thirty_one = vec![ProductSection::new(
        (0..=PRODUCTS_MAX)
            .map(|index| ProductItem::new(format!("sku-{index}")))
            .collect(),
    )];
    assert_rejected(
        ProductListMessage::new("catalog-1", thirty_one),
        "interactive.action.sections.product_items",
    );
}

#[test]
fn product_section_titles_stop_at_24_characters() {
    let title = "t".repeat(PRODUCT_SECTION_TITLE_MAX_CHARS);
    assert!(request(ProductListMessage::new(
        "catalog-1",
        vec![one_product_section().title(title)]
    ))
    .to_json()
    .is_ok());
    assert_rejected(
        ProductListMessage::new(
            "catalog-1",
            vec![one_product_section().title("t".repeat(PRODUCT_SECTION_TITLE_MAX_CHARS + 1))],
        ),
        "interactive.action.sections.title",
    );
}

#[test]
fn order_items_stop_at_thirty() {
    let items = (0..ORDER_ITEMS_MAX).map(item).collect();
    assert!(request(OrderDetailsMessage::new(
        "order-42",
        Order::new("catalog-1", items)
    ))
    .to_json()
    .is_ok());
    let items = (0..=ORDER_ITEMS_MAX).map(item).collect();
    assert_rejected(
        OrderDetailsMessage::new("order-42", Order::new("catalog-1", items)),
        "interactive.action.parameters.order.product_items",
    );
}

#[test]
fn quantity_currency_and_price_boundaries_are_exact() {
    assert!(request(OrderDetailsMessage::new(
        "order-42",
        Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 0.0, "EUR")]),
    ))
    .to_json()
    .is_ok());
    assert_rejected(
        OrderDetailsMessage::new(
            "order-42",
            Order::new("catalog-1", vec![OrderItem::new("sku-1", 0, 1.0, "EUR")]),
        ),
        "interactive.action.parameters.order.product_items.quantity",
    );
    assert_rejected(
        OrderDetailsMessage::new(
            "order-42",
            Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, -0.01, "EUR")]),
        ),
        "interactive.action.parameters.order.product_items.item_price",
    );
    assert_rejected(
        OrderDetailsMessage::new(
            "order-42",
            Order::new(
                "catalog-1",
                vec![OrderItem::new("sku-1", 1, f64::NAN, "EUR")],
            ),
        ),
        "interactive.action.parameters.order.product_items.item_price",
    );
    assert_rejected(
        OrderDetailsMessage::new(
            "order-42",
            Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 1.0, "EU")]),
        ),
        "interactive.action.parameters.order.product_items.currency",
    );
}

#[test]
fn reference_id_stops_at_35_characters() {
    let at = "r".repeat(ORDER_REFERENCE_ID_MAX_CHARS);
    assert!(request(OrderDetailsMessage::new(&at, order()))
        .to_json()
        .is_ok());
    assert_rejected(
        OrderDetailsMessage::new("r".repeat(ORDER_REFERENCE_ID_MAX_CHARS + 1), order()),
        "interactive.action.parameters.reference_id",
    );
}

#[test]
fn order_status_description_stops_at_1024_bytes() {
    assert!(request(
        OrderStatusMessage::new("order-42", OrderStatus::Shipped).description("d".repeat(1024))
    )
    .to_json()
    .is_ok());
    assert_rejected(
        OrderStatusMessage::new("order-42", OrderStatus::Shipped).description("d".repeat(1025)),
        "interactive.action.parameters.order.description",
    );
    assert_rejected(
        OrderStatusMessage::new("order-42", OrderStatus::Unknown(String::new())),
        "interactive.action.parameters.order.status",
    );
}

#[test]
fn address_country_is_exactly_two_uppercase_letters() {
    assert!(request(AddressMessage::new("DE")).to_json().is_ok());
    for bad in ["D", "DEU", "de"] {
        assert_rejected(
            AddressMessage::new(bad),
            "interactive.action.parameters.country",
        );
    }
}

#[test]
fn address_values_and_saved_addresses_stop_at_their_limits() {
    let at_limit = (0..ADDRESS_VALUES_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.field(AddressFieldType::Address, format!("field-{index}"), "value")
    });
    assert!(request(at_limit).to_json().is_ok());
    let over = (0..=ADDRESS_VALUES_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.field(AddressFieldType::Address, format!("field-{index}"), "value")
    });
    assert_rejected(over, "interactive.action.parameters.values");

    let at_limit = (0..ADDRESS_SAVED_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.saved_address(SavedAddress::new(format!("id-{index}")))
    });
    assert!(request(at_limit).to_json().is_ok());
    let over = (0..=ADDRESS_SAVED_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.saved_address(SavedAddress::new(format!("id-{index}")))
    });
    assert_rejected(over, "interactive.action.parameters.saved_addresses");
}

#[test]
fn address_field_validation_preserves_unknown_types() {
    let field = AddressField::new(AddressFieldType::Unknown("nickname".into()), "nick", "Ada");
    let message = AddressMessage::new("DE").value(field.clone());
    let json = message.validate().and_then(|()| request(message).to_json());
    assert!(json.is_ok());
    assert_eq!(field.field_type.as_str(), "nickname");

    assert_rejected(
        AddressMessage::new("DE").value(AddressField::new(AddressFieldType::Address, "", "x")),
        "interactive.action.parameters.values.name",
    );
    assert_rejected(
        AddressMessage::new("DE").value(AddressField::new(
            AddressFieldType::Address,
            "n".repeat(ADDRESS_FIELD_NAME_MAX_CHARS + 1),
            "x",
        )),
        "interactive.action.parameters.values.name",
    );
    assert_rejected(
        AddressMessage::new("DE").value(AddressField::new(
            AddressFieldType::Address,
            "address",
            "x".repeat(1025),
        )),
        "interactive.action.parameters.values.value",
    );
}

#[test]
fn address_field_names_and_saved_ids_stop_at_their_limits() {
    assert!(request(AddressMessage::new("DE").field(
        AddressFieldType::Address,
        "address",
        "x".repeat(1024),
    ))
    .to_json()
    .is_ok());

    let name = "n".repeat(ADDRESS_FIELD_NAME_MAX_CHARS);
    assert!(
        request(AddressMessage::new("DE").field(AddressFieldType::Address, name, "value",))
            .to_json()
            .is_ok()
    );

    let id = "i".repeat(ADDRESS_ID_MAX_CHARS);
    assert!(
        request(AddressMessage::new("DE").saved_address(SavedAddress::new(id)))
            .to_json()
            .is_ok()
    );
    assert_rejected(
        AddressMessage::new("DE")
            .saved_address(SavedAddress::new("i".repeat(ADDRESS_ID_MAX_CHARS + 1))),
        "interactive.action.parameters.saved_addresses.id",
    );
}

#[test]
fn message_body_type_is_catalog_for_catalog_messages() {
    let body: MessageBody = catalog().into();
    assert_eq!(body.message_type(), "catalog");
    assert!(matches!(body, MessageBody::Catalog(_)));
}
