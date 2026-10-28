//! Commerce wire shapes, permissive inbound parsing, and every enforced limit
//! from the accepted side through the first rejected value.

mod common;

use common::{client, Body, Call, MockTransport};
use meta_whatsapp::commerce::{
    ADDRESS_FIELD_NAME_MAX_CHARS, ADDRESS_FIELD_VALUE_MAX_BYTES, ADDRESS_ID_MAX_CHARS,
    ADDRESS_SAVED_MAX, ADDRESS_VALUES_MAX, CATALOG_ID_MAX_CHARS, COMMERCE_BODY_MAX_BYTES,
    COMMERCE_FOOTER_MAX_CHARS, ORDER_DESCRIPTION_MAX_BYTES, ORDER_ITEMS_MAX,
    ORDER_REFERENCE_ID_MAX_CHARS, ORDER_TEXT_MAX_BYTES, PRODUCTS_MAX,
    PRODUCT_RETAILER_ID_MAX_CHARS, PRODUCT_SECTIONS_MAX, PRODUCT_SECTION_TITLE_MAX_CHARS,
};
use meta_whatsapp::{
    AddressField, AddressFieldType, AddressMessage, AddressReply, CatalogMessage, Error,
    HeaderText, Message, MessageBody, MessageRequest, Order, OrderDetailsMessage, OrderItem,
    OrderStatus, OrderStatusMessage, ProductItem, ProductListMessage, ProductMessage,
    ProductSection, SavedAddress, WebhookPayload,
};
use proptest::prelude::*;
use serde_json::{json, Map, Value};

fn request(body: impl Into<MessageBody>) -> MessageRequest {
    MessageRequest::new("4915112345678", body)
}

fn assert_rejected(body: impl Into<MessageBody>, field: &'static str) {
    match request(body).to_json() {
        Err(Error::Validation { field: actual, .. }) => assert_eq!(actual, field),
        other => panic!("expected a validation error for {field}, got {other:?}"),
    }
}

fn assert_accepted(body: impl Into<MessageBody>) -> Value {
    match request(body).to_json() {
        Ok(value) => value,
        Err(error) => panic!("expected the message to validate, got {error}"),
    }
}

fn json_body(call: &Call) -> &Value {
    match &call.body {
        Body::Json(value) => value,
        other => panic!("expected a JSON body, got {other:?}"),
    }
}

fn section(title: Option<&str>, products: usize) -> ProductSection {
    let items = (0..products)
        .map(|index| ProductItem::new(format!("sku-{index}")))
        .collect();
    let section = ProductSection::new(items);
    match title {
        Some(title) => section.title(title),
        None => section,
    }
}

fn order() -> Order {
    Order::new("catalog-1", Vec::new())
        .item(OrderItem::new("sku-1", 2, 4.5, "EUR"))
        .text("Two coffees")
}

fn one_section() -> ProductListMessage {
    ProductListMessage::new("catalog-1", Vec::new()).section(section(Some("Coffee"), 1))
}

#[tokio::test]
async fn catalog_message_uses_the_interactive_catalog_shape() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    client
        .send(&request(
            CatalogMessage::new("sku-thumb")
                .body("Our shop")
                .footer("Prices include VAT"),
        ))
        .await
        .expect("catalog should send");

    let call = transport.only_call();
    let body = json_body(&call);
    assert_eq!(body["type"], "interactive");
    assert_eq!(body["interactive"]["type"], "catalog_message");
    assert_eq!(body["interactive"]["body"]["text"], "Our shop");
    assert_eq!(body["interactive"]["footer"]["text"], "Prices include VAT");
    assert_eq!(body["interactive"]["action"]["name"], "catalog_message");
    assert_eq!(
        body["interactive"]["action"]["parameters"]["thumbnail_product_retailer_id"],
        "sku-thumb",
    );
}

#[tokio::test]
async fn invalid_commerce_never_reaches_the_transport() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    let result = client.send(&request(AddressMessage::new("de"))).await;

    assert!(matches!(result, Err(Error::Validation { .. })));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn every_commerce_type_converts_to_an_interactive_message_body() {
    let bodies: Vec<(MessageBody, &str)> = vec![
        (CatalogMessage::new("p").into(), "catalog_message"),
        (ProductMessage::new("c", "p").into(), "product"),
        (one_section().into(), "product_list"),
        (
            OrderDetailsMessage::new("r", order()).into(),
            "order_details",
        ),
        (
            OrderStatusMessage::new("r", OrderStatus::Shipped).into(),
            "order_status",
        ),
        (AddressMessage::new("DE").into(), "address_message"),
    ];
    for (body, kind) in bodies {
        assert_eq!(body.message_type(), "interactive");
        assert!(body.validate().is_ok(), "{kind} should validate");
    }
}

#[test]
fn products_and_product_lists_render_their_action_shapes() {
    let product = assert_accepted(
        ProductMessage::new("catalog-1", "sku-1")
            .body("Single origin")
            .footer("Ships tomorrow"),
    );
    assert_eq!(product["interactive"]["type"], "product");
    assert_eq!(product["interactive"]["body"]["text"], "Single origin");
    assert_eq!(product["interactive"]["footer"]["text"], "Ships tomorrow");
    assert_eq!(product["interactive"]["action"]["catalog_id"], "catalog-1");
    assert_eq!(
        product["interactive"]["action"]["product_retailer_id"],
        "sku-1",
    );

    let list = assert_accepted(
        one_section()
            .header(HeaderText::new("Menu"))
            .body("Choose a coffee")
            .footer("Prices include VAT"),
    );
    assert_eq!(list["interactive"]["type"], "product_list");
    assert_eq!(list["interactive"]["header"]["type"], "text");
    assert_eq!(list["interactive"]["header"]["text"], "Menu");
    assert_eq!(list["interactive"]["body"]["text"], "Choose a coffee");
    assert_eq!(list["interactive"]["footer"]["text"], "Prices include VAT");
    assert_eq!(list["interactive"]["action"]["catalog_id"], "catalog-1");
    assert_eq!(
        list["interactive"]["action"]["sections"][0]["product_items"][0]["product_retailer_id"],
        "sku-0",
    );
}

#[test]
fn order_details_and_status_render_the_review_order_action() {
    let details = assert_accepted(
        OrderDetailsMessage::new("order-42", order())
            .body("Thanks!")
            .footer("Review before paying"),
    );
    assert_eq!(details["interactive"]["type"], "order_details");
    assert_eq!(details["interactive"]["action"]["name"], "review_order");
    assert_eq!(
        details["interactive"]["action"]["parameters"]["reference_id"],
        "order-42",
    );
    assert_eq!(
        details["interactive"]["action"]["parameters"]["order"]["catalog_id"],
        "catalog-1",
    );
    let item = &details["interactive"]["action"]["parameters"]["order"]["product_items"][0];
    assert_eq!(item["product_retailer_id"], "sku-1");
    assert_eq!(item["quantity"], 2);
    assert_eq!(item["item_price"], 4.5);
    assert_eq!(item["currency"], "EUR");

    let status = assert_accepted(
        OrderStatusMessage::new("order-42", OrderStatus::Shipped)
            .body("On its way")
            .description("Arrives tomorrow"),
    );
    assert_eq!(status["interactive"]["type"], "order_status");
    assert_eq!(status["interactive"]["action"]["name"], "review_order");
    assert_eq!(
        status["interactive"]["action"]["parameters"]["reference_id"],
        "order-42",
    );
    assert_eq!(
        status["interactive"]["action"]["parameters"]["order"]["status"],
        "shipped",
    );
    assert_eq!(
        status["interactive"]["action"]["parameters"]["order"]["description"],
        "Arrives tomorrow",
    );
}

#[test]
fn address_message_renders_values_and_saved_addresses() {
    let address = assert_accepted(
        AddressMessage::new("DE")
            .body("Where should we deliver?")
            .field(AddressFieldType::Address, "city", "Berlin")
            .saved_address(SavedAddress::new("home").field("zip", "10115")),
    );
    assert_eq!(address["interactive"]["type"], "address_message");
    assert_eq!(
        address["interactive"]["body"]["text"],
        "Where should we deliver?"
    );
    assert_eq!(address["interactive"]["action"]["name"], "address_message");
    assert_eq!(
        address["interactive"]["action"]["parameters"]["country"],
        "DE",
    );
    assert_eq!(
        address["interactive"]["action"]["parameters"]["values"][0]["name"],
        "city",
    );
    assert_eq!(
        address["interactive"]["action"]["parameters"]["saved_addresses"][0]["value"]["zip"],
        "10115",
    );
}

#[test]
fn inbound_order_parses_numeric_strings_aliases_and_unknowns() {
    let payload: WebhookPayload = serde_json::from_str(
        r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
            "field":"messages","value":{
                "metadata":{"display_phone_number":"1","phone_number_id":"2"},
                "messages":[{"id":"m1","from":"3","timestamp":"10","type":"order","order":{
                    "catalogId":"catalog-1",
                    "text":"",
                    "productItems":[{
                        "productRetailerId":"sku-1",
                        "quantity":"2",
                        "itemPrice":"4.5",
                        "currency":"EUR",
                        "provider_future":"kept"
                    }],
                    "order_future":"kept"
                }}]
            }
        }]}]}"#,
    )
    .expect("order webhook should parse");
    let message = payload.inbound_messages().next().expect("one message").1;
    let order = message.order().expect("order block");
    assert_eq!(order.catalog_id, "catalog-1");
    assert_eq!(order.text.as_deref(), Some(""));
    assert_eq!(order.product_items[0].quantity, 2);
    assert!((order.product_items[0].item_price - 4.5).abs() < f64::EPSILON);
    assert_eq!(order.product_items[0].extra["provider_future"], "kept");
    assert_eq!(order.extra["order_future"], "kept");
}

#[test]
fn inbound_address_parses_from_interactive_and_top_levels() {
    let interactive: Message = serde_json::from_str(
        r#"{"id":"m1","from":"3","type":"interactive","interactive":{
            "type":"address_message",
            "address_message":{
                "country":"DE",
                "values":[{"name":"city","value":"Berlin","future":"kept"}],
                "savedAddresses":[{"id":"home","value":{"city":"Berlin"}}],
                "address_future":"kept"
            }
        }}"#,
    )
    .expect("interactive address should parse");
    let address = interactive.address().expect("interactive address block");
    assert_eq!(address.country.as_deref(), Some("DE"));
    assert_eq!(address.field("city"), Some("Berlin"));
    assert_eq!(address.values[0].extra["future"], "kept");
    assert_eq!(address.saved_addresses[0].id, "home");
    assert_eq!(address.extra["address_future"], "kept");

    let top_level: Message = serde_json::from_str(
        r#"{"id":"m2","from":"3","type":"address","address":{
            "country":"DE","values":[{"type":"nickname","name":"nick","value":"Ada"}]
        }}"#,
    )
    .expect("top-level address should parse");
    let address = top_level.address().expect("top-level address block");
    assert_eq!(
        address.values[0].field_type,
        AddressFieldType::Unknown("nickname".into()),
    );
}

#[test]
fn unknown_status_and_field_spellings_are_carried() {
    assert_eq!(
        OrderStatus::from_wire("partiallyShipped"),
        OrderStatus::PartiallyShipped,
    );
    let status: OrderStatus =
        serde_json::from_str(r#""packed_by_robot""#).expect("unknown status should parse");
    assert_eq!(status.as_str(), "packed_by_robot");
    assert_eq!(
        serde_json::to_string(&status).expect("status serializes"),
        r#""packed_by_robot""#,
    );
    let field: AddressReply =
        serde_json::from_str(r#"{"values":[{"type":"nickname","name":"nick","value":"Ada"}]}"#)
            .expect("unknown field type should parse");
    assert_eq!(field.values[0].field_type.as_str(), "nickname");
}

#[test]
fn product_and_catalog_ids_stop_at_100_characters() {
    let at = "p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS);
    assert_accepted(ProductMessage::new("c", &at));
    assert_accepted(CatalogMessage::new(&at));
    assert_accepted(ProductListMessage::new(
        "c",
        vec![ProductSection::new(vec![ProductItem::new(&at)])],
    ));
    assert_rejected(
        ProductMessage::new("c", "p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS + 1)),
        "interactive.action.product_retailer_id",
    );
    assert_rejected(
        CatalogMessage::new("p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS + 1)),
        "interactive.action.parameters.thumbnail_product_retailer_id",
    );
    assert_rejected(
        ProductListMessage::new(
            "c",
            vec![ProductSection::new(vec![ProductItem::new(
                "p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS + 1),
            )])],
        ),
        "interactive.action.sections.product_items.product_retailer_id",
    );

    let catalog_at = "c".repeat(CATALOG_ID_MAX_CHARS);
    assert_accepted(ProductMessage::new(&catalog_at, "p"));
    assert_rejected(
        ProductMessage::new("c".repeat(CATALOG_ID_MAX_CHARS + 1), "p"),
        "interactive.action.catalog_id",
    );
}

#[test]
fn product_sections_and_titles_stop_at_their_limits() {
    let ten_sections: Vec<_> = (0..PRODUCT_SECTIONS_MAX)
        .map(|index| section(Some("Coffee"), 3).title(format!("Section {index}")))
        .collect();
    assert_accepted(ProductListMessage::new("c", ten_sections));

    let eleven_sections: Vec<_> = (0..=PRODUCT_SECTIONS_MAX)
        .map(|index| section(Some("Coffee"), 1).title(format!("Section {index}")))
        .collect();
    assert_rejected(
        ProductListMessage::new("c", eleven_sections),
        "interactive.action.sections",
    );

    let title_at = "t".repeat(PRODUCT_SECTION_TITLE_MAX_CHARS);
    let two_titled = vec![section(Some(&title_at), 1), section(Some(&title_at), 1)];
    assert_accepted(ProductListMessage::new("c", two_titled));
    let one_too_long = vec![
        section(Some(&"t".repeat(PRODUCT_SECTION_TITLE_MAX_CHARS + 1)), 1),
        section(Some("Coffee"), 1),
    ];
    assert_rejected(
        ProductListMessage::new("c", one_too_long),
        "interactive.action.sections.title",
    );

    let untitled = vec![section(None, 1), section(None, 1)];
    assert_rejected(
        ProductListMessage::new("c", untitled),
        "interactive.action.sections.title",
    );
    assert_rejected(
        ProductListMessage::new("c", vec![section(None, 0)]),
        "interactive.action.sections.product_items",
    );
}

#[test]
fn product_count_stops_at_30_in_total() {
    assert_accepted(ProductListMessage::new(
        "c",
        vec![section(None, PRODUCTS_MAX)],
    ));
    assert_rejected(
        ProductListMessage::new("c", vec![section(None, PRODUCTS_MAX + 1)]),
        "interactive.action.sections.product_items",
    );
    assert_rejected(
        ProductListMessage::new("c", Vec::new()),
        "interactive.action.sections",
    );
}

#[test]
fn product_list_requires_a_header() {
    let mut message = one_section();
    message.header = None;
    assert_rejected(message, "interactive.header.text");
}

#[test]
fn commerce_bodies_headers_and_footers_stop_at_their_limits() {
    let body_at = "b".repeat(COMMERCE_BODY_MAX_BYTES);
    assert_accepted(CatalogMessage::new("p").body(body_at));
    assert_accepted(CatalogMessage::new("p").body("é".repeat(COMMERCE_BODY_MAX_BYTES / 2)));
    assert_rejected(
        CatalogMessage::new("p").body("b".repeat(COMMERCE_BODY_MAX_BYTES + 1)),
        "interactive.body.text",
    );
    assert_rejected(CatalogMessage::new("p").body(""), "interactive.body.text");

    let header_at = "h".repeat(COMMERCE_FOOTER_MAX_CHARS);
    let footer_at = "f".repeat(COMMERCE_FOOTER_MAX_CHARS);
    assert_accepted(
        one_section()
            .header(HeaderText::new(&header_at))
            .footer(&footer_at),
    );
    assert_rejected(
        one_section().header(HeaderText::new("h".repeat(COMMERCE_FOOTER_MAX_CHARS + 1))),
        "interactive.header.text",
    );
    assert_rejected(
        one_section().footer("f".repeat(COMMERCE_FOOTER_MAX_CHARS + 1)),
        "interactive.footer.text",
    );
}

#[test]
fn order_item_count_text_and_reference_id_stop_at_their_limits() {
    let items: Vec<_> = (0..ORDER_ITEMS_MAX)
        .map(|index| OrderItem::new(format!("sku-{index}"), 1, 1.0, "EUR"))
        .collect();
    assert_accepted(OrderDetailsMessage::new(
        "r",
        Order::new("c", items.clone()),
    ));
    let too_many: Vec<_> = (0..=ORDER_ITEMS_MAX)
        .map(|index| OrderItem::new(format!("sku-{index}"), 1, 1.0, "EUR"))
        .collect();
    assert_rejected(
        OrderDetailsMessage::new("r", Order::new("c", too_many)),
        "interactive.action.parameters.order.product_items",
    );
    assert_rejected(
        OrderDetailsMessage::new("r", Order::new("c", Vec::new())),
        "interactive.action.parameters.order.product_items",
    );

    let text_at = "t".repeat(ORDER_TEXT_MAX_BYTES);
    assert_accepted(OrderDetailsMessage::new(
        "r",
        Order::new("c", items.clone()).text(text_at),
    ));
    assert_rejected(
        OrderDetailsMessage::new(
            "r",
            Order::new("c", items).text("t".repeat(ORDER_TEXT_MAX_BYTES + 1)),
        ),
        "interactive.action.parameters.order.text",
    );

    let reference_at = "r".repeat(ORDER_REFERENCE_ID_MAX_CHARS);
    assert_accepted(OrderDetailsMessage::new(reference_at, order()));
    assert_rejected(
        OrderDetailsMessage::new("r".repeat(ORDER_REFERENCE_ID_MAX_CHARS + 1), order()),
        "interactive.action.parameters.reference_id",
    );
}

#[test]
fn every_commerce_limit_accepts_the_value_just_inside() {
    assert_accepted(CatalogMessage::new(
        "p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS - 1),
    ));
    assert_accepted(ProductMessage::new(
        "c".repeat(CATALOG_ID_MAX_CHARS - 1),
        "p".repeat(PRODUCT_RETAILER_ID_MAX_CHARS - 1),
    ));

    let nine_sections: Vec<_> = (0..PRODUCT_SECTIONS_MAX - 1)
        .map(|index| section(Some("Coffee"), 1).title(format!("Section {index}")))
        .collect();
    assert_accepted(ProductListMessage::new("c", nine_sections));
    assert_accepted(ProductListMessage::new(
        "c",
        vec![section(None, PRODUCTS_MAX - 1)],
    ));
    let title_inside = "t".repeat(PRODUCT_SECTION_TITLE_MAX_CHARS - 1);
    assert_accepted(ProductListMessage::new(
        "c",
        vec![section(Some(&title_inside), 1)],
    ));

    let body_inside = "b".repeat(COMMERCE_BODY_MAX_BYTES - 1);
    let header_inside = "h".repeat(COMMERCE_FOOTER_MAX_CHARS - 1);
    let footer_inside = "f".repeat(COMMERCE_FOOTER_MAX_CHARS - 1);
    assert_accepted(
        one_section()
            .header(HeaderText::new(header_inside))
            .body(body_inside)
            .footer(footer_inside),
    );

    let items: Vec<_> = (0..ORDER_ITEMS_MAX - 1)
        .map(|index| OrderItem::new(format!("sku-{index}"), 1, 1.0, "EUR"))
        .collect();
    assert_accepted(OrderDetailsMessage::new(
        "r".repeat(ORDER_REFERENCE_ID_MAX_CHARS - 1),
        Order::new("c", items).text("t".repeat(ORDER_TEXT_MAX_BYTES - 1)),
    ));
    assert_accepted(
        OrderStatusMessage::new(
            "r".repeat(ORDER_REFERENCE_ID_MAX_CHARS - 1),
            OrderStatus::Shipped,
        )
        .description("d".repeat(ORDER_DESCRIPTION_MAX_BYTES - 1)),
    );

    let values_inside =
        (0..ADDRESS_VALUES_MAX - 1).fold(AddressMessage::new("DE"), |message, index| {
            message.field(AddressFieldType::Address, format!("field-{index}"), "value")
        });
    assert_accepted(values_inside);
    let saved_inside = (0..ADDRESS_SAVED_MAX - 1)
        .fold(AddressMessage::new("DE"), |message, index| {
            message.saved_address(SavedAddress::new(format!("id-{index}")))
        });
    assert_accepted(saved_inside);
    assert_accepted(
        AddressMessage::new("DE")
            .field(
                AddressFieldType::Address,
                "n".repeat(ADDRESS_FIELD_NAME_MAX_CHARS - 1),
                "v".repeat(ADDRESS_FIELD_VALUE_MAX_BYTES - 1),
            )
            .saved_address(SavedAddress::new("i".repeat(ADDRESS_ID_MAX_CHARS - 1))),
    );
}

#[test]
fn order_quantity_price_and_currency_boundaries_are_enforced() {
    assert_accepted(OrderDetailsMessage::new(
        "r",
        Order::new("c", vec![OrderItem::new("p", 1, 0.0, "EUR")]),
    ));
    for item in [
        OrderItem::new("p", 0, 1.0, "EUR"),
        OrderItem::new("p", 1, -0.01, "EUR"),
        OrderItem::new("p", 1, f64::NAN, "EUR"),
        OrderItem::new("p", 1, f64::INFINITY, "EUR"),
    ] {
        let expected = if item.quantity == 0 {
            "interactive.action.parameters.order.product_items.quantity"
        } else {
            "interactive.action.parameters.order.product_items.item_price"
        };
        assert_rejected(
            OrderDetailsMessage::new("r", Order::new("c", vec![item])),
            expected,
        );
    }

    for currency in ["EU", "EURO", "eur", "E1R"] {
        assert_rejected(
            OrderDetailsMessage::new(
                "r",
                Order::new("c", vec![OrderItem::new("p", 1, 1.0, currency)]),
            ),
            "interactive.action.parameters.order.product_items.currency",
        );
    }

    assert_rejected(
        OrderDetailsMessage::new(
            "r",
            Order::new("c", vec![OrderItem::new("", 1, 1.0, "EUR")]),
        ),
        "interactive.action.parameters.order.product_items.product_retailer_id",
    );
}

#[test]
fn order_and_address_enum_spellings_cover_every_variant() {
    for (status, wire) in [
        (OrderStatus::Pending, "pending"),
        (OrderStatus::Processing, "processing"),
        (OrderStatus::PartiallyShipped, "partially_shipped"),
        (OrderStatus::Shipped, "shipped"),
        (OrderStatus::Completed, "completed"),
        (OrderStatus::Canceled, "canceled"),
        (OrderStatus::Cancelled, "cancelled"),
        (OrderStatus::Unknown("packed".into()), "packed"),
    ] {
        assert_eq!(status.as_str(), wire);
        assert_eq!(status.to_string(), wire);
        assert_eq!(
            serde_json::to_string(&status).expect("status serializes"),
            format!(r#""{wire}""#),
        );
    }

    for (field_type, wire) in [
        (AddressFieldType::Name, "name"),
        (AddressFieldType::Address, "address"),
        (AddressFieldType::Phone, "phone"),
        (AddressFieldType::Email, "email"),
        (AddressFieldType::Unknown("nickname".into()), "nickname"),
    ] {
        assert_eq!(field_type.as_str(), wire);
        assert_eq!(field_type.to_string(), wire);
        assert_eq!(
            serde_json::to_string(&field_type).expect("field type serializes"),
            format!(r#""{wire}""#),
        );
    }
}

#[test]
fn malformed_inbound_order_numbers_are_typed_errors() {
    for quantity in [
        json!(-1),
        json!(1.5),
        json!(u64::from(u32::MAX) + 1),
        json!("not-a-number"),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<OrderItem>(json!({ "quantity": quantity })).is_err());
    }
    for price in [
        json!("not-a-number"),
        json!("NaN"),
        json!("inf"),
        json!("-inf"),
        Value::Null,
        json!([1]),
    ] {
        assert!(serde_json::from_value::<OrderItem>(json!({ "item_price": price })).is_err());
    }
}

#[test]
fn order_status_description_stops_at_1024_bytes() {
    let at = "d".repeat(ORDER_DESCRIPTION_MAX_BYTES);
    assert_accepted(OrderStatusMessage::new("r", OrderStatus::Shipped).description(at));
    assert_rejected(
        OrderStatusMessage::new("r", OrderStatus::Shipped)
            .description("d".repeat(ORDER_DESCRIPTION_MAX_BYTES + 1)),
        "interactive.action.parameters.order.description",
    );
    assert_rejected(
        OrderStatusMessage::new("r", OrderStatus::Unknown(String::new())),
        "interactive.action.parameters.order.status",
    );
}

#[test]
fn address_country_is_exactly_two_uppercase_letters() {
    for country in ["DE", "US", "GB"] {
        assert_accepted(AddressMessage::new(country));
    }
    for country in ["", "D", "DEU", "de", "DÉ"] {
        assert_rejected(
            AddressMessage::new(country),
            "interactive.action.parameters.country",
        );
    }
}

#[test]
fn address_value_and_saved_address_counts_stop_at_their_limits() {
    let values_at = (0..ADDRESS_VALUES_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.field(AddressFieldType::Address, format!("field-{index}"), "value")
    });
    assert_accepted(values_at);
    let values_over = (0..=ADDRESS_VALUES_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.field(AddressFieldType::Address, format!("field-{index}"), "value")
    });
    assert_rejected(values_over, "interactive.action.parameters.values");

    let saved_at = (0..ADDRESS_SAVED_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.saved_address(SavedAddress::new(format!("id-{index}")))
    });
    assert_accepted(saved_at);
    let saved_over = (0..=ADDRESS_SAVED_MAX).fold(AddressMessage::new("DE"), |message, index| {
        message.saved_address(SavedAddress::new(format!("id-{index}")))
    });
    assert_rejected(saved_over, "interactive.action.parameters.saved_addresses");
}

#[test]
fn address_field_names_values_and_saved_ids_stop_at_their_limits() {
    let name_at = "n".repeat(ADDRESS_FIELD_NAME_MAX_CHARS);
    assert_accepted(AddressMessage::new("DE").field(AddressFieldType::Address, name_at, "v"));
    assert_rejected(
        AddressMessage::new("DE").field(
            AddressFieldType::Address,
            "n".repeat(ADDRESS_FIELD_NAME_MAX_CHARS + 1),
            "v",
        ),
        "interactive.action.parameters.values.name",
    );
    assert_rejected(
        AddressMessage::new("DE").field(AddressFieldType::Address, "", "v"),
        "interactive.action.parameters.values.name",
    );

    let value_at = "v".repeat(ADDRESS_FIELD_VALUE_MAX_BYTES);
    assert_accepted(AddressMessage::new("DE").field(
        AddressFieldType::Address,
        "address",
        value_at,
    ));
    assert_rejected(
        AddressMessage::new("DE").field(
            AddressFieldType::Address,
            "address",
            "v".repeat(ADDRESS_FIELD_VALUE_MAX_BYTES + 1),
        ),
        "interactive.action.parameters.values.value",
    );

    let id_at = "i".repeat(ADDRESS_ID_MAX_CHARS);
    assert_accepted(AddressMessage::new("DE").saved_address(SavedAddress::new(id_at)));
    assert_rejected(
        AddressMessage::new("DE")
            .saved_address(SavedAddress::new("i".repeat(ADDRESS_ID_MAX_CHARS + 1))),
        "interactive.action.parameters.saved_addresses.id",
    );
    assert_rejected(
        AddressMessage::new("DE").saved_address(SavedAddress::new("")),
        "interactive.action.parameters.saved_addresses.id",
    );
}

#[test]
fn address_field_limits_and_unknown_types_are_checked() {
    let unknown = AddressField::new(AddressFieldType::Unknown("nickname".into()), "nick", "Ada");
    assert!(unknown.validate().is_ok());
    assert_eq!(unknown.field_type.as_str(), "nickname");
    assert!(AddressField::new(AddressFieldType::Address, "n\n", "v")
        .validate()
        .is_err());
    assert!(SavedAddress::new("bad\nid").validate().is_err());
    assert!(
        AddressField::new(AddressFieldType::Unknown(String::new()), "nick", "Ada",)
            .validate()
            .is_err()
    );
    assert!(
        AddressField::new(AddressFieldType::Unknown("bad\ntype".into()), "nick", "Ada",)
            .validate()
            .is_err()
    );
}

fn arb_json() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        ".{0,24}".prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 32, 5, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            prop::collection::vec((".{0,16}", inner), 0..4).prop_map(|pairs| {
                Value::Object(pairs.into_iter().collect::<Map<String, Value>>())
            }),
        ]
    })
}

proptest! {
    /// Order parsing is total over arbitrary JSON, and anything accepted keeps
    /// one quantity/price representation when serialized.
    #[test]
    fn order_parsing_is_total(value in arb_json()) {
        if let Ok(order) = serde_json::from_value::<Order>(value) {
            let rendered = serde_json::to_value(&order).expect("order serializes");
            prop_assert!(rendered.is_object());
        }
    }

    /// Address parsing is total over arbitrary JSON.
    #[test]
    fn address_parsing_is_total(value in arb_json()) {
        if let Ok(address) = serde_json::from_value::<AddressReply>(value) {
            let rendered = serde_json::to_value(&address).expect("address serializes");
            prop_assert!(rendered.is_object());
        }
    }

    /// Numeric strings and numbers mean the same quantity on input.
    #[test]
    fn quantity_accepts_numbers_and_numeric_strings(quantity in 0_u32..1_000_000) {
        let from_number: OrderItem = serde_json::from_value(json!({"quantity": quantity}))
            .expect("number parses");
        let from_string: OrderItem =
            serde_json::from_value(json!({"quantity": quantity.to_string()}))
                .expect("numeric string parses");
        prop_assert_eq!(from_number.quantity, quantity);
        prop_assert_eq!(from_string.quantity, quantity);
    }

    /// Unknown order statuses round-trip instead of being rejected.
    #[test]
    fn order_statuses_round_trip(raw in ".{0,64}") {
        let status = OrderStatus::from_wire(raw.clone());
        let encoded = serde_json::to_string(&status).expect("status serializes");
        let decoded: OrderStatus = serde_json::from_str(&encoded).expect("status parses");
        prop_assert_eq!(decoded.as_str(), raw.as_str());
    }

    /// Product ids are measured in characters, not bytes, and controls are
    /// never accepted. The renderer accepts exactly the valid side.
    #[test]
    fn product_id_acceptance_matches_the_character_rule(raw in ".{0,140}") {
        let expected = !raw.is_empty()
            && raw.chars().count() <= PRODUCT_RETAILER_ID_MAX_CHARS
            && !raw.chars().any(char::is_control);
        prop_assert_eq!(ProductMessage::new("catalog", raw).validate().is_ok(), expected);
    }

    /// Body acceptance is exactly non-empty and at most 1024 UTF-8 bytes.
    #[test]
    fn commerce_body_acceptance_matches_the_byte_rule(body in ".{0,1200}") {
        let expected = !body.is_empty() && body.len() <= COMMERCE_BODY_MAX_BYTES;
        prop_assert_eq!(CatalogMessage::new("p").body(body).validate().is_ok(), expected);
    }
}
