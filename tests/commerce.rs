//! The commerce surface that is still open on `main`: the catalog entry point
//! (`catalog_message`) and the `Order`/`OrderItem` builders with their
//! validation. The product and address families live in `tests/messages.rs` and
//! `tests/address.rs`; inbound order parsing lives in `tests/webhook.rs`.

mod common;

use common::{accepted, client};
use meta_whatsapp::{CatalogMessage, Error, InteractiveMessage, MessageRequest, Order, OrderItem};
use serde_json::json;

fn validation_field(error: Error) -> String {
    match error {
        Error::Validation { field, .. } => field.to_string(),
        other => panic!("expected a validation error, got {other:?}"),
    }
}

// ---- catalog_message -------------------------------------------------------

#[tokio::test]
async fn catalog_message_body_is_exactly_the_documented_json() {
    let transport = common::MockTransport::new();
    transport.push_response(accepted("wamid.catalog"));
    let client = client(transport.clone());

    let request = MessageRequest::new(
        "4915112345678",
        CatalogMessage::new("2lc20305pt", "Ordering is easy.").footer("Best grocery deals"),
    );
    client.send(&request).await.unwrap();

    let call = transport.only_call();
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/messages"
    );
    let common::Body::Json(body) = &call.body else {
        panic!("expected a JSON body");
    };
    assert_eq!(
        *body,
        json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": "4915112345678",
            "type": "interactive",
            "interactive": {
                "type": "catalog_message",
                "body": { "text": "Ordering is easy." },
                "footer": { "text": "Best grocery deals" },
                "action": {
                    "name": "catalog_message",
                    "parameters": { "thumbnail_product_retailer_id": "2lc20305pt" },
                },
            },
        })
    );
}

#[test]
fn catalog_message_without_a_footer_renders_the_body_and_the_action() {
    let json = MessageRequest::new("1", CatalogMessage::new("sku", "Browse")).to_json();
    let json = json.unwrap_or_else(|error| panic!("a catalog message is valid: {error}"));
    assert_eq!(json["interactive"]["body"]["text"], "Browse");
    assert!(json["interactive"]["footer"].is_null());
    assert_eq!(
        json["interactive"]["action"]["parameters"]["thumbnail_product_retailer_id"],
        "sku"
    );
}

#[test]
fn catalog_message_reports_the_interactive_wire_type_and_kind() {
    let body = meta_whatsapp::MessageBody::from(CatalogMessage::new("sku", "Browse"));
    assert_eq!(body.message_type(), "interactive");
    assert_eq!(
        InteractiveMessage::from(CatalogMessage::new("sku", "Browse")).kind(),
        "catalog_message"
    );
}

#[test]
fn catalog_thumbnail_is_required_and_printable() {
    let empty = CatalogMessage::new("", "Browse").validate().unwrap_err();
    assert_eq!(
        validation_field(empty),
        "interactive.action.parameters.thumbnail_product_retailer_id"
    );

    let control = CatalogMessage::new("sku\u{7}", "Browse")
        .validate()
        .unwrap_err();
    assert_eq!(
        validation_field(control),
        "interactive.action.parameters.thumbnail_product_retailer_id"
    );

    assert!(CatalogMessage::new("sku", "Browse").validate().is_ok());
}

#[test]
fn catalog_body_is_required_and_capped_at_1024_bytes() {
    // The body is not optional in the spec, and an empty one would render as
    // `"text": ""`.
    assert_eq!(
        validation_field(CatalogMessage::new("sku", "").validate().unwrap_err()),
        "interactive.body.text"
    );

    // Exactly at the limit: accepted.
    let at = CatalogMessage::new("sku", "a".repeat(1024));
    assert!(at.validate().is_ok());

    // One byte over: refused, and the field is the body.
    let over = CatalogMessage::new("sku", "a".repeat(1025));
    assert_eq!(
        validation_field(over.validate().unwrap_err()),
        "interactive.body.text"
    );

    // Bytes, not characters: 4-byte characters reach the limit four times as
    // fast, so 256 of them fit and 257 do not.
    let unicode_at = CatalogMessage::new("sku", "🎉".repeat(256));
    assert!(unicode_at.validate().is_ok());
    let unicode_over = CatalogMessage::new("sku", "🎉".repeat(257));
    assert_eq!(
        validation_field(unicode_over.validate().unwrap_err()),
        "interactive.body.text"
    );
}

#[test]
fn catalog_footer_boundary_is_60_characters() {
    let at = CatalogMessage::new("sku", "Browse").footer("f".repeat(60));
    assert!(at.validate().is_ok());

    let over = CatalogMessage::new("sku", "Browse").footer("f".repeat(61));
    assert_eq!(
        validation_field(over.validate().unwrap_err()),
        "interactive.footer.text"
    );

    // Characters, not bytes: 60 four-byte emoji fit, 61 do not.
    assert!(CatalogMessage::new("sku", "Browse")
        .footer("🎉".repeat(60))
        .validate()
        .is_ok());
    assert!(CatalogMessage::new("sku", "Browse")
        .footer("🎉".repeat(61))
        .validate()
        .is_err());
}

// ---- order builders --------------------------------------------------------

fn item() -> OrderItem {
    OrderItem::new("sku-1", 2, 4.5, "EUR")
}

#[test]
fn a_built_order_validates_and_keeps_its_fields() {
    let order = Order::new("catalog-1", vec![item()]).text("Two please");
    assert_eq!(order.catalog_id, "catalog-1");
    assert_eq!(order.text.as_deref(), Some("Two please"));
    assert_eq!(order.product_items[0].quantity, Some(2));
    assert_eq!(order.product_items[0].line_total(), Some(9.0));
    assert!(order.validate().is_ok());
}

#[test]
fn an_order_needs_at_least_one_item() {
    let empty = Order::new("catalog-1", Vec::new());
    assert_eq!(
        validation_field(empty.validate().unwrap_err()),
        "order.product_items"
    );
}

#[test]
fn the_item_count_is_not_capped_by_this_crate() {
    // Meta publishes no upper bound for `product_items`; the only count rule
    // is that an order has at least one line.
    let many = Order::new("catalog-1", vec![item(); 100]);
    assert!(many.validate().is_ok());
}

#[test]
fn the_documented_empty_order_text_is_accepted() {
    // Meta's own example payload sends `"text": ""`, so an empty string must
    // not be treated as a defect, and `text` has no published ceiling.
    let empty = Order::new("catalog-1", vec![item()]).text("");
    assert!(empty.validate().is_ok());

    let long = Order::new("catalog-1", vec![item()]).text("t".repeat(4000));
    assert!(long.validate().is_ok());
}

#[test]
fn a_line_item_needs_a_unit_count_of_at_least_one() {
    let zero = OrderItem {
        quantity: Some(0),
        ..item()
    };
    assert_eq!(
        validation_field(zero.validate().unwrap_err()),
        "order.product_items.quantity"
    );

    let missing = OrderItem {
        quantity: None,
        ..item()
    };
    assert_eq!(
        validation_field(missing.validate().unwrap_err()),
        "order.product_items.quantity"
    );

    // The boundary pair: 1 is the smallest valid count, 0 is not.
    assert!(OrderItem {
        quantity: Some(1),
        ..item()
    }
    .validate()
    .is_ok());
}

#[test]
fn a_price_must_be_finite_and_not_negative() {
    for price in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -0.01, -1.0] {
        let bad = OrderItem {
            item_price: Some(price),
            ..item()
        };
        assert_eq!(
            validation_field(bad.validate().unwrap_err()),
            "order.product_items.item_price",
            "price {price} must be refused"
        );
    }

    let missing = OrderItem {
        item_price: None,
        ..item()
    };
    assert_eq!(
        validation_field(missing.validate().unwrap_err()),
        "order.product_items.item_price"
    );

    // The boundary pair: zero is a price (a free line), the smallest negative
    // finite value is not.
    assert!(OrderItem {
        item_price: Some(0.0),
        ..item()
    }
    .validate()
    .is_ok());
}

#[test]
fn a_currency_is_three_uppercase_letters() {
    for currency in ["", "EU", "EURO", "eur", "EuR", "€UR"] {
        let bad = OrderItem {
            currency: currency.to_string(),
            ..item()
        };
        assert_eq!(
            validation_field(bad.validate().unwrap_err()),
            "order.product_items.currency",
            "currency {currency:?} must be refused"
        );
    }
    assert!(item().validate().is_ok());
}

#[test]
fn a_product_id_is_required_and_printable() {
    let empty = OrderItem {
        product_retailer_id: String::new(),
        ..item()
    };
    assert_eq!(
        validation_field(empty.validate().unwrap_err()),
        "order.product_items.product_retailer_id"
    );

    let control = OrderItem {
        product_retailer_id: "sku\n1".to_string(),
        ..item()
    };
    assert_eq!(
        validation_field(control.validate().unwrap_err()),
        "order.product_items.product_retailer_id"
    );

    // No length rule: a retailer id longer than any documented ceiling is fine.
    assert!(OrderItem {
        product_retailer_id: "s".repeat(400),
        ..item()
    }
    .validate()
    .is_ok());
}

#[test]
fn the_catalog_id_is_required_and_printable() {
    let empty = Order::new("", vec![item()]);
    assert_eq!(
        validation_field(empty.validate().unwrap_err()),
        "order.catalog_id"
    );

    let control = Order::new("catalog\u{1}", vec![item()]);
    assert_eq!(
        validation_field(control.validate().unwrap_err()),
        "order.catalog_id"
    );
}

// ---- the review finding: prices and quantities on the way in ----------------

/// Pulls the typed order out of a minimal `type: "order"` envelope.
fn parsed_item(item: &serde_json::Value) -> OrderItem {
    let payload = json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": "1",
            "changes": [{
                "field": "messages",
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": { "display_phone_number": "1", "phone_number_id": "2" },
                    "messages": [{
                        "from": "3",
                        "id": "wamid.1",
                        "timestamp": "1",
                        "type": "order",
                        "order": { "catalog_id": "c", "product_items": [item] },
                    }],
                },
            }],
        }],
    });
    let payload: meta_whatsapp::WebhookPayload =
        serde_json::from_value(payload).unwrap_or_else(|error| panic!("envelope: {error}"));
    let item = payload
        .inbound_messages()
        .next()
        .and_then(|(_, message)| message.order().cloned())
        .and_then(|order| order.product_items.into_iter().next());
    item.unwrap_or_else(|| panic!("the order block must survive parsing"))
}

#[test]
fn an_inbound_price_never_becomes_nan_infinity_or_a_negative() {
    // Every one of these parses as an `f64` in Rust, and none of them is money.
    for raw in [
        "NaN", "inf", "-inf", "infinity", "-1", "-0.01", "1e3", " 4.5", "4.5x", "",
    ] {
        let item = parsed_item(&json!({
            "product_retailer_id": "sku-1",
            "quantity": 1,
            "item_price": raw,
            "currency": "EUR",
        }));
        assert_eq!(
            item.item_price, None,
            "item_price {raw:?} must not be read as a price"
        );
        // The unreadable member is kept, not dropped.
        assert_eq!(item.extra.get("item_price"), Some(&json!(raw)));
        assert!(item.validate().is_err(), "an unreadable price is not valid");
    }

    // The boundary pair: "0" and "0.01" are prices, "-1" above is not.
    assert_eq!(
        parsed_item(&json!({ "quantity": 1, "item_price": "0", "currency": "EUR" })).item_price,
        Some(0.0)
    );
    assert_eq!(
        parsed_item(&json!({ "quantity": 1, "item_price": "0.01", "currency": "EUR" })).item_price,
        Some(0.01)
    );
}

#[test]
fn an_inbound_quantity_is_an_unsigned_integer_only() {
    for raw in [
        json!(0),
        json!("0"),
        json!(2.5),
        json!(-1),
        json!("2x"),
        json!(""),
    ] {
        let item = parsed_item(&json!({
            "product_retailer_id": "sku-1",
            "quantity": raw,
            "item_price": 1.0,
            "currency": "EUR",
        }));
        // Zero is an integer, so it parses — `validate` is what refuses it.
        if raw == json!(0) || raw == json!("0") {
            assert_eq!(item.quantity, Some(0));
            assert_eq!(
                validation_field(item.validate().unwrap_err()),
                "order.product_items.quantity"
            );
        } else {
            assert_eq!(item.quantity, None, "quantity {raw:?} is not a count");
        }
    }

    // The boundary pair: 1 is a count, 0 is not, and a huge count that fits
    // `u32` still parses.
    assert_eq!(
        parsed_item(&json!({ "quantity": 1, "item_price": 1.0, "currency": "EUR" })).quantity,
        Some(1)
    );
    assert_eq!(
        parsed_item(&json!({ "quantity": u32::MAX, "item_price": 1.0, "currency": "EUR" }))
            .quantity,
        Some(u32::MAX)
    );
}
