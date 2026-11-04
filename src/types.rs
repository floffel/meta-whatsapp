//! Typed view of the webhook payload Meta posts for
//! `whatsapp_business_account`.
//!
//! Parsing is deliberately permissive: unknown fields are kept in
//! [`ChangeValue::extra`] / [`Message::extra`], unknown `type` strings stay
//! strings, missing optional blocks default to `None`. Nothing here rejects a
//! payload because Meta added a field — a *signature* mismatch does the
//! rejecting, in [`crate::webhook`].
//!
//! ```
//! use meta_whatsapp::WebhookPayload;
//!
//! let payload: WebhookPayload = serde_json::from_str(
//!     r#"{"object":"whatsapp_business_account","entry":[{
//!         "id":"1","changes":[{"field":"messages","value":{
//!             "metadata":{"display_phone_number":"1","phone_number_id":"2"},
//!             "messages":[{"id":"wamid.1","from":"4915","timestamp":"1729600000","type":"text",
//!                          "text":{"body":"Hallo"}}]}}]}]}"#,
//! ).unwrap();
//!
//! let (phone_number_id, message) = payload.inbound_messages().next().unwrap();
//! assert_eq!(phone_number_id, "2");
//! assert_eq!(message.text.as_ref().unwrap().body, "Hallo");
//! assert_eq!(message.timestamp_secs(), Some(1_729_600_000));
//!
//! ```

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::message::{require_no_controls, require_non_empty};

/// Top level webhook envelope.
///
/// ```
/// use meta_whatsapp::WebhookPayload;
///
/// let payload: WebhookPayload =
///     serde_json::from_str(r#"{"object":"whatsapp_business_account","entry":[]}"#).unwrap();
/// assert!(payload.entry.is_empty());
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct WebhookPayload {
    /// Always `whatsapp_business_account` for this API.
    #[serde(default)]
    pub object: String,
    /// One entry per business account that received an event.
    #[serde(default)]
    pub entry: Vec<Entry>,
}

/// One business account's batch of changes.
///
/// ```
/// use meta_whatsapp::Entry;
///
/// let entry: Entry = serde_json::from_str(r#"{"id":"1","changes":[]}"#).unwrap();
/// assert_eq!(entry.id, "1");
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Entry {
    /// `WhatsApp` Business Account id.
    #[serde(default)]
    pub id: String,
    /// Events for that account.
    #[serde(default)]
    pub changes: Vec<Change>,
}

/// A single change: a `messages` event, a template status update, a quality
/// update, or anything Meta adds later (the `field` stays a string).
///
/// ```
/// use meta_whatsapp::Change;
///
/// let change: Change = serde_json::from_str(
///     r#"{"field":"message_template_status_update","value":{"event":"APPROVED","message_template_id":7}}"#,
/// ).unwrap();
/// assert_eq!(change.field, "message_template_status_update");
/// assert_eq!(change.value.event(), Some("APPROVED"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Change {
    /// Event name, e.g. `messages`, `message_template_status_update`.
    #[serde(default)]
    pub field: String,
    /// The event body.
    #[serde(default)]
    pub value: ChangeValue,
}

/// Body of a change.
///
/// Two shapes share this type:
///
/// * message/status events (`metadata`, `messages`, `statuses`, `contacts`), and
/// * everything else (template status updates, quality updates, flow events),
///   whose keys land in [`ChangeValue::extra`].
///
/// Non-object values are accepted too (they are stored in `extra["value"]`),
/// so an unexpected shape cannot make the whole webhook fail to parse.
///
/// ```
/// use meta_whatsapp::ChangeValue;
///
/// let scoped: ChangeValue = serde_json::from_str(
///     r#"{"messaging_product":"whatsapp","phone_number":"4915","event":"FLAGGED"}"#,
/// ).unwrap();
/// assert_eq!(scoped.event(), Some("FLAGGED"));
/// assert_eq!(scoped.extra_str("phone_number"), Some("4915"));
/// ```
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ChangeValue {
    /// `whatsapp` for this API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messaging_product: Option<String>,
    /// Which phone number received the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    /// Inbound messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    /// Delivery/read statuses for messages we sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<Status>,
    /// Sender profiles (`profile.name`, `wa_id`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<Contact>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for ChangeValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(Self::from_value(value))
    }
}

impl ChangeValue {
    /// Builds a `ChangeValue` from an already parsed JSON value, without ever
    /// failing: unparsable elements are dropped, unknown keys are kept.
    ///
    /// ```
    /// use meta_whatsapp::ChangeValue;
    ///
    /// let value = ChangeValue::from_value(serde_json::json!("just a string"));
    /// assert!(value.messages.is_empty());
    /// assert_eq!(value.extra["value"], "just a string");
    /// ```
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        let mut map = match value {
            Value::Object(map) => map,
            other => {
                let mut extra = Map::new();
                extra.insert("value".to_string(), other);
                return Self {
                    extra,
                    ..Self::default()
                };
            }
        };

        let messaging_product = take_string(&mut map, "messaging_product");
        let metadata = take_value(&mut map, "metadata");
        let messages = take_vec(&mut map, "messages");
        let statuses = take_vec(&mut map, "statuses");
        let contacts = take_vec(&mut map, "contacts");

        Self {
            messaging_product,
            metadata,
            messages,
            statuses,
            contacts,
            extra: map,
        }
    }

    /// `extra["event"]`, the discriminator used by non-message events such as
    /// `message_template_status_update`.
    ///
    /// ```
    /// use meta_whatsapp::ChangeValue;
    ///
    /// let value = ChangeValue::from_value(serde_json::json!({"event": "APPROVED"}));
    /// assert_eq!(value.event(), Some("APPROVED"));
    /// ```
    #[must_use]
    pub fn event(&self) -> Option<&str> {
        self.extra_str("event")
    }

    /// A string-valued unknown key.
    ///
    /// ```
    /// use meta_whatsapp::ChangeValue;
    ///
    /// let value = ChangeValue::from_value(serde_json::json!({"message_template_name": "hello"}));
    /// assert_eq!(value.extra_str("message_template_name"), Some("hello"));
    /// assert_eq!(value.extra_str("missing"), None);
    /// ```
    #[must_use]
    pub fn extra_str(&self, key: &str) -> Option<&str> {
        self.extra.get(key).and_then(Value::as_str)
    }

    /// Whether this change carries no message, status or contact payload (the
    /// keys this crate models). Unknown keys in [`ChangeValue::extra`] do not
    /// count, so account-level events are "empty" here.
    ///
    /// ```
    /// use meta_whatsapp::ChangeValue;
    ///
    /// assert!(ChangeValue::default().is_empty());
    /// let event = ChangeValue::from_value(serde_json::json!({"event": "APPROVED"}));
    /// assert!(event.is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty() && self.statuses.is_empty() && self.contacts.is_empty()
    }
}

/// Phone number metadata of the receiving business number.
///
/// ```
/// use meta_whatsapp::types::Metadata;
///
/// let metadata: Metadata = serde_json::from_str(
///     r#"{"display_phone_number":"+49 151","phone_number_id":"106540352242922"}"#,
/// ).unwrap();
/// assert_eq!(metadata.phone_number_id, "106540352242922");
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Metadata {
    /// Display phone number.
    #[serde(default)]
    pub display_phone_number: String,
    /// Id used in the Graph API path.
    #[serde(default)]
    pub phone_number_id: String,
}

/// Sender profile attached to a batch of messages.
///
/// ```
/// use meta_whatsapp::types::Contact;
///
/// let contact: Contact = serde_json::from_str(
///     r#"{"profile":{"name":"Florian"},"wa_id":"4915112345678"}"#,
/// ).unwrap();
/// assert_eq!(contact.profile.name.as_deref(), Some("Florian"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Contact {
    /// `wa_id`, the sender's `WhatsApp` id.
    #[serde(default)]
    pub wa_id: String,
    /// Display profile.
    #[serde(default)]
    pub profile: Profile,
}

/// Display name of a sender.
///
/// ```
/// use meta_whatsapp::types::Profile;
///
/// let profile: Profile = serde_json::from_str(r#"{"name":"Ada"}"#).unwrap();
/// assert_eq!(profile.name.as_deref(), Some("Ada"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Profile {
    /// `WhatsApp` display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Business profile id, when present.
    #[serde(default)]
    pub whatsapp_id: Option<String>,
}

/// An inbound message, whatever its type.
///
/// The `type` string is authoritative; the matching optional block
/// (`text`, `image`, `interactive`, ...) carries the payload. Blocks this
/// crate does not model stay in [`Message::extra`].
///
/// ```
/// use meta_whatsapp::Message;
///
/// let message: Message = serde_json::from_str(
///     r#"{"id":"wamid.1","from":"4915","timestamp":"1729600000","type":"image",
///         "image":{"id":"987","mime_type":"image/jpeg","caption":"Katze"}}"#,
/// ).unwrap();
/// assert_eq!(message.media().unwrap().caption.as_deref(), Some("Katze"));
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Message {
    /// Provider message id (use it as an idempotency key).
    #[serde(default)]
    pub id: String,
    /// Sender's `WhatsApp` id (phone number, digits only).
    #[serde(default)]
    pub from: String,
    /// Unix timestamp as string, as sent by Meta.
    #[serde(default)]
    pub timestamp: String,
    /// `text`, `image`, `audio`, `document`, `button`, `interactive`, `order`, ….
    #[serde(rename = "type", default)]
    pub message_type: String,
    /// Present when `type == "text"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    /// Present when `type == "image"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<MediaObject>,
    /// Present when `type == "video"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<MediaObject>,
    /// Present when `type == "audio"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<MediaObject>,
    /// Present when `type == "document"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MediaObject>,
    /// Present when `type == "sticker"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticker: Option<MediaObject>,
    /// Present when `type == "location"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    /// Shared contact cards (`type == "contacts"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub contacts: Vec<ContactCard>,
    /// Present when `type == "reaction"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaction: Option<Reaction>,
    /// Present when `type == "interactive"` (button/list/flow reply).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interactive: Option<InteractiveReply>,
    /// Present when `type == "order"`: the catalog order a customer placed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Order>,
    /// Present when `type == "button"` (template quick-reply tap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<ButtonReply>,
    /// Quoted message / forwarding metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Context>,
    /// Present when `type == "errors"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<StatusError>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Message {
    /// The media block matching this message's type, if any.
    ///
    /// ```
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"1","from":"2","type":"sticker","sticker":{"id":"55","animated":true}}"#,
    /// ).unwrap();
    /// assert_eq!(message.media().unwrap().id, "55");
    /// ```
    #[must_use]
    pub fn media(&self) -> Option<&MediaObject> {
        match self.message_type.as_str() {
            "image" => self.image.as_ref(),
            "video" => self.video.as_ref(),
            "audio" => self.audio.as_ref(),
            "document" => self.document.as_ref(),
            "sticker" => self.sticker.as_ref(),
            _ => None,
        }
    }

    /// The text body, if this is a text message.
    ///
    /// ```
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message =
    ///     serde_json::from_str(r#"{"id":"1","from":"2","type":"text","text":{"body":"hi"}}"#).unwrap();
    /// assert_eq!(message.text_body(), Some("hi"));
    /// ```
    #[must_use]
    pub fn text_body(&self) -> Option<&str> {
        self.text.as_ref().map(|text| text.body.as_str())
    }

    /// The flow reply, when this message is the `nfm_reply` of a finished flow.
    ///
    /// ```rust
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"1","from":"2","type":"interactive","interactive":{"type":"nfm_reply",
    ///         "nfm_reply":{"name":"flow","body":"Sent",
    ///                      "response_json":"{\"screen\":\"SUMMARY\",\"data\":{\"ok\":true}}"}}}"#,
    /// ).unwrap();
    /// let reply = message.flow_reply().unwrap();
    /// assert_eq!(reply.name.as_deref(), Some("flow"));
    /// assert_eq!(reply.response().unwrap().field("screen"), Some("SUMMARY"));
    /// ```
    #[must_use]
    pub fn flow_reply(&self) -> Option<&FlowReply> {
        self.interactive
            .as_ref()
            .and_then(|interactive| interactive.nfm_reply.as_ref())
    }

    /// The order a customer placed from a catalog, when this message is one.
    ///
    /// ```
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"wamid.1","from":"4915","type":"order","order":{
    ///         "catalog_id":"194836987003835","text":"Love these!",
    ///         "product_items":[{"product_retailer_id":"di9ozbzfi4","quantity":2,
    ///                           "item_price":7.99,"currency":"USD"}]}}"#,
    /// ).unwrap();
    /// let order = message.order().unwrap();
    /// assert_eq!(order.text.as_deref(), Some("Love these!"));
    /// assert_eq!(order.product_items[0].line_total(), Some(15.98));
    /// ```
    #[must_use]
    pub fn order(&self) -> Option<&Order> {
        self.order.as_ref()
    }

    /// `timestamp` parsed as unix seconds.
    ///
    /// ```rust
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"1","from":"2","type":"text","timestamp":"1729600000"}"#,
    /// ).unwrap();
    /// assert_eq!(message.timestamp_secs(), Some(1_729_600_000));
    /// ```
    #[must_use]
    pub fn timestamp_secs(&self) -> Option<u64> {
        self.timestamp.trim().parse().ok()
    }
}

/// The order a customer placed from a catalog (`type: "order"`).
///
/// Shape from Meta's *Order messages* webhook reference: `catalog_id`,
/// optional `text`, and `product_items`. The block is parse-side only — a
/// business cannot send an order; the outbound order messages (`order_details`,
/// `order_status`) are interactive and remain an open issue.
///
/// Decoding is total, like the rest of the payload: an unreadable member is
/// kept verbatim in [`Order::extra`] instead of failing the message, so one
/// malformed field never removes an order from the batch.
///
/// ```
/// use meta_whatsapp::types::Order;
///
/// let order: Order = serde_json::from_str(
///     r#"{"catalog_id":"194836987003835","text":"Two please",
///         "product_items":[{"product_retailer_id":"sku-1","quantity":2,
///                           "item_price":4.5,"currency":"EUR"}]}"#,
/// ).unwrap();
/// assert_eq!(order.catalog_id, "194836987003835");
/// assert_eq!(order.product_items[0].product_retailer_id, "sku-1");
/// ```
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Order {
    /// Catalog the ordered products belong to.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub catalog_id: String,
    /// Free text the customer attached to the order.
    ///
    /// An empty string is a normal value, not a defect: Meta's own example
    /// payload sends `"text": ""`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Ordered products; empty when absent or not an array.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub product_items: Vec<OrderItem>,
    /// Members this crate does not model or could not read, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for Order {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let mut map = match value {
            Value::Object(map) => map,
            other => {
                let mut extra = Map::new();
                extra.insert("value".to_string(), other);
                return Ok(Self {
                    extra,
                    ..Self::default()
                });
            }
        };
        let catalog_id = take_string_alias(&mut map, &["catalog_id", "catalogId"]);
        let text = match map.remove("text") {
            Some(Value::String(text)) => Some(text),
            Some(other) => {
                map.insert("text".to_string(), other);
                None
            }
            None => None,
        };
        let product_items = take_items(&mut map, &["product_items", "productItems"]);
        Ok(Self {
            catalog_id,
            text,
            product_items,
            extra: map,
        })
    }
}

impl Order {
    /// An order for `product_items` from `catalog_id`.
    ///
    /// ```rust
    /// use meta_whatsapp::{Order, OrderItem};
    ///
    /// let order = Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 4.0, "EUR")]);
    /// assert_eq!(order.catalog_id, "catalog-1");
    /// assert!(order.validate().is_ok());
    /// ```
    #[must_use]
    pub fn new(catalog_id: impl Into<String>, product_items: Vec<OrderItem>) -> Self {
        Self {
            catalog_id: catalog_id.into(),
            text: None,
            product_items,
            extra: Map::new(),
        }
    }

    /// Sets the free text the customer attached to the order.
    ///
    /// ```rust
    /// use meta_whatsapp::{Order, OrderItem};
    ///
    /// let order = Order::new("c", vec![OrderItem::new("p", 1, 1.0, "USD")]).text("Two please");
    /// assert_eq!(order.text.as_deref(), Some("Two please"));
    /// ```
    #[must_use]
    pub fn text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }

    /// Appends a line item.
    ///
    /// ```rust
    /// use meta_whatsapp::{Order, OrderItem};
    ///
    /// let order = Order::new("c", Vec::new()).item(OrderItem::new("p", 1, 1.0, "USD"));
    /// assert_eq!(order.product_items.len(), 1);
    /// ```
    #[must_use]
    pub fn item(mut self, item: OrderItem) -> Self {
        self.product_items.push(item);
        self
    }

    /// Checks the catalog id, the item list and every line item.
    ///
    /// This is the check an application runs on an order it parsed out of a
    /// webhook before it echoes the order back, invoices it or stores it: the
    /// parser accepts anything, `validate` says whether the order is usable.
    ///
    /// Meta publishes no upper bound for `product_items` and none for `text`
    /// (its own example sends `text: ""`), so neither is capped here — a
    /// ceiling of this crate's own invention would refuse orders the provider
    /// happily delivers.
    ///
    /// ```rust
    /// use meta_whatsapp::{Order, OrderItem};
    ///
    /// let order = Order::new("catalog-1", vec![OrderItem::new("sku-1", 1, 4.0, "EUR")]);
    /// assert!(order.validate().is_ok());
    /// assert!(Order::new("catalog-1", Vec::new()).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or unprintable catalog id, an empty
    /// item list, or an invalid line item.
    pub fn validate(&self) -> crate::error::Result<()> {
        require_non_empty("order.catalog_id", &self.catalog_id)?;
        require_no_controls("order.catalog_id", &self.catalog_id)?;
        if self.product_items.is_empty() {
            return Err(Error::validation(
                "order.product_items",
                "must contain at least one item",
            ));
        }
        for item in &self.product_items {
            item.validate()?;
        }
        Ok(())
    }
}

/// One line item of an inbound order (`order.product_items[]`).
///
/// `quantity` and `item_price` are `None` when the payload omits the member or
/// sends something this crate cannot read as a number, and the raw member is
/// then kept in [`OrderItem::extra`] — it never becomes a silent zero, because
/// a wrong price is a wrong invoice line.
///
/// ```
/// use meta_whatsapp::types::OrderItem;
///
/// let item: OrderItem = serde_json::from_str(
///     r#"{"productRetailerId":"sku-1","quantity":"2","itemPrice":"4.5","currency":"EUR"}"#,
/// ).unwrap();
/// assert_eq!(item.product_retailer_id, "sku-1");
/// assert_eq!(item.quantity, Some(2));
/// assert_eq!(item.item_price, Some(4.5));
/// assert_eq!(item.line_total(), Some(9.0));
/// ```
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct OrderItem {
    /// Catalog product id (`product_retailer_id`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub product_retailer_id: String,
    /// Units ordered, as an unsigned 32-bit integer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<u32>,
    /// Price per unit in `currency`, finite and non-negative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_price: Option<f64>,
    /// Currency code as sent (the catalog's, normally ISO 4217).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub currency: String,
    /// Members this crate does not model or could not read, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl OrderItem {
    /// A line item with every field Meta's order payload carries.
    ///
    /// The builder is the mirror image of the parser: it sets `quantity` and
    /// `item_price` to `Some`, so an item built by hand passes
    /// [`OrderItem::validate`] whenever the values themselves are valid.
    ///
    /// ```rust
    /// use meta_whatsapp::OrderItem;
    ///
    /// let item = OrderItem::new("sku-1", 2, 4.5, "EUR");
    /// assert_eq!(item.quantity, Some(2));
    /// assert_eq!(item.item_price, Some(4.5));
    /// assert!(item.validate().is_ok());
    /// ```
    #[must_use]
    pub fn new(
        product_retailer_id: impl Into<String>,
        quantity: u32,
        item_price: f64,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            product_retailer_id: product_retailer_id.into(),
            quantity: Some(quantity),
            item_price: Some(item_price),
            currency: currency.into(),
            extra: Map::new(),
        }
    }

    /// Checks the product id, the unit count, the price and the currency.
    ///
    /// Use it on the parse side too: the parser is deliberately total, so a
    /// quantity or price it could not read stays `None` here instead of
    /// failing the webhook — an order that is then rejected by `validate` is
    /// visible, while a silently dropped message would not be.
    ///
    /// ```rust
    /// use meta_whatsapp::OrderItem;
    ///
    /// assert!(OrderItem::new("sku-1", 1, 0.0, "EUR").validate().is_ok());
    /// assert!(OrderItem::new("sku-1", 0, 1.0, "EUR").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field: an empty or
    /// unprintable id, a quantity of zero, a negative or non-finite price, a
    /// currency that is not three uppercase ASCII letters, or an order member
    /// the parser could not read at all (`quantity`/`item_price` are `None`).
    pub fn validate(&self) -> crate::error::Result<()> {
        require_non_empty(
            "order.product_items.product_retailer_id",
            &self.product_retailer_id,
        )?;
        // No length rule: neither the spec nor the reference caps a retailer id.
        require_no_controls(
            "order.product_items.product_retailer_id",
            &self.product_retailer_id,
        )?;
        match self.quantity {
            Some(0) => {
                return Err(Error::validation(
                    "order.product_items.quantity",
                    "must be at least 1",
                ))
            }
            Some(_) => {}
            None => {
                return Err(Error::validation(
                    "order.product_items.quantity",
                    "is missing or not an unsigned integer",
                ))
            }
        }
        match self.item_price {
            Some(price) if price.is_finite() && price >= 0.0 => {}
            Some(_) => {
                return Err(Error::validation(
                    "order.product_items.item_price",
                    "must be finite and at least 0",
                ))
            }
            None => {
                return Err(Error::validation(
                    "order.product_items.item_price",
                    "is missing or not a readable price",
                ))
            }
        }
        if self.currency.len() != 3 || !self.currency.bytes().all(|byte| byte.is_ascii_uppercase())
        {
            return Err(Error::validation(
                "order.product_items.currency",
                "must be a three-letter uppercase ISO 4217 code",
            ));
        }
        Ok(())
    }

    /// `quantity * item_price`, or `None` when either half is unknown.
    ///
    /// ```
    /// use meta_whatsapp::types::OrderItem;
    ///
    /// let item: OrderItem = serde_json::from_str(
    ///     r#"{"product_retailer_id":"sku-1","quantity":3,"item_price":2.5,"currency":"EUR"}"#,
    /// ).unwrap();
    /// assert_eq!(item.line_total(), Some(7.5));
    ///
    /// // A quantity without a readable price has no total to report.
    /// let partial: OrderItem =
    ///     serde_json::from_str(r#"{"product_retailer_id":"sku-1","quantity":3}"#).unwrap();
    /// assert_eq!(partial.line_total(), None);
    /// ```
    #[must_use]
    pub fn line_total(&self) -> Option<f64> {
        let quantity = self.quantity?;
        let price = self.item_price?;
        Some(f64::from(quantity) * price)
    }
}

impl<'de> Deserialize<'de> for OrderItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut map = Map::<String, Value>::deserialize(deserializer)?;
        Ok(Self {
            product_retailer_id: take_string_alias(
                &mut map,
                &["product_retailer_id", "productRetailerId"],
            ),
            quantity: take_quantity(&mut map),
            item_price: take_price(&mut map),
            currency: take_string_alias(&mut map, &["currency"]),
            extra: map,
        })
    }
}

/// Removes the first present key and returns its string; a member of another
/// type stays in `extra` instead of being dropped.
fn take_string_alias(map: &mut Map<String, Value>, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| take_string(map, key))
        .unwrap_or_default()
}

/// Removes `quantity` and parses it; an unreadable member goes back to `extra`.
fn take_quantity(map: &mut Map<String, Value>) -> Option<u32> {
    let value = map.remove("quantity")?;
    let quantity = parse_quantity(&value);
    if quantity.is_none() {
        map.insert("quantity".to_string(), value);
    }
    quantity
}

/// Removes `item_price` (or `itemPrice`) and parses it; an unreadable member
/// goes back to `extra` under the key it came in on.
fn take_price(map: &mut Map<String, Value>) -> Option<f64> {
    let key = ["item_price", "itemPrice"]
        .into_iter()
        .find(|key| map.contains_key(*key))?;
    let value = map.remove(key)?;
    let price = parse_price(&value);
    if price.is_none() {
        map.insert(key.to_string(), value);
    }
    price
}

/// Removes the first present key holding an array and parses every element into
/// an [`OrderItem`]. Elements that are not objects are dropped, as elsewhere in
/// this parser; a member that is not an array at all stays in `extra`.
fn take_items(map: &mut Map<String, Value>, keys: &[&str]) -> Vec<OrderItem> {
    for key in keys {
        match map.remove(*key) {
            Some(Value::Array(items)) => {
                return items
                    .into_iter()
                    .filter_map(|item| serde_json::from_value(item).ok())
                    .collect()
            }
            Some(other) => {
                map.insert((*key).to_string(), other);
            }
            None => {}
        }
    }
    Vec::new()
}

/// A quantity is an unsigned integer, sent as a JSON number or as a decimal
/// string. `2.0`, `-1`, `""` and `"2x"` are not quantities.
fn parse_quantity(value: &Value) -> Option<u32> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|raw| u32::try_from(raw).ok()),
        Value::String(text) => parse_digits(text),
        _ => None,
    }
}

fn parse_digits(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// A price is a finite, non-negative amount, sent as a JSON number or as a
/// plain decimal string. `NaN`, `inf`, `-1`, `1e3`, `" 4.5"` and `"4.5x"` are
/// refused: none of them is a price this crate can carry with confidence.
fn parse_price(value: &Value) -> Option<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => parse_decimal(text)?,
        _ => return None,
    };
    (parsed.is_finite() && parsed >= 0.0).then_some(parsed)
}

fn parse_decimal(text: &str) -> Option<f64> {
    let mut dots = 0;
    let mut digits = 0;
    for byte in text.bytes() {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' if dots == 0 => dots += 1,
            _ => return None,
        }
    }
    if digits == 0 {
        return None;
    }
    text.parse().ok()
}

/// Text body of a text message.
///
/// ```
/// use meta_whatsapp::types::Text;
/// let text: Text = serde_json::from_str(r#"{"body":"Hallo"}"#).unwrap();
/// assert_eq!(text.body, "Hallo");
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Text {
    /// The message text.
    #[serde(default)]
    pub body: String,
}

/// Media block of an inbound image/video/audio/document/sticker message.
///
/// ```
/// use meta_whatsapp::types::MediaObject;
///
/// let media: MediaObject = serde_json::from_str(
///     r#"{"id":"1","mime_type":"application/pdf","filename":"a.pdf","caption":"Rechnung"}"#,
/// ).unwrap();
/// assert_eq!(media.filename.as_deref(), Some("a.pdf"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct MediaObject {
    /// Media id for `GET /{media_id}`.
    #[serde(default)]
    pub id: String,
    /// MIME type as declared by the sender.
    #[serde(default, alias = "mimeType")]
    pub mime_type: Option<String>,
    /// SHA-256 of the file, hex encoded.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Caption, when the sender added one.
    #[serde(default)]
    pub caption: Option<String>,
    /// Filename, for documents.
    #[serde(default)]
    pub filename: Option<String>,
    /// Whether an audio message is a voice note.
    #[serde(default)]
    pub voice: bool,
    /// Whether a sticker is animated.
    #[serde(default)]
    pub animated: bool,
}

/// Inbound location pin.
///
/// ```
/// use meta_whatsapp::types::Location;
///
/// let location: Location =
///     serde_json::from_str(r#"{"latitude":48.1,"longitude":11.5,"name":"Büro"}"#).unwrap();
/// assert_eq!(location.name.as_deref(), Some("Büro"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Location {
    /// Degrees north.
    #[serde(default)]
    pub latitude: f64,
    /// Degrees east.
    #[serde(default)]
    pub longitude: f64,
    /// Place name.
    #[serde(default)]
    pub name: Option<String>,
    /// Street address.
    #[serde(default)]
    pub address: Option<String>,
    /// Map URL.
    #[serde(default)]
    pub url: Option<String>,
}

/// Contact card shared by a user.
///
/// ```
/// use meta_whatsapp::types::ContactCard;
///
/// let card: ContactCard =
///     serde_json::from_str(r#"{"name":{"formatted_name":"Ada","first_name":"Ada"}}"#).unwrap();
/// assert_eq!(card.name.formatted_name.as_deref(), Some("Ada"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactCard {
    /// Name block.
    #[serde(default)]
    pub name: ContactCardName,
    /// Keys this crate does not model (phones, emails, org, ...).
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Name block of a shared contact card.
///
/// ```
/// use meta_whatsapp::types::ContactCardName;
///
/// let name: ContactCardName = serde_json::from_str(r#"{"formatted_name":"Ada Lovelace"}"#).unwrap();
/// assert_eq!(name.formatted_name.as_deref(), Some("Ada Lovelace"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContactCardName {
    /// Full name.
    #[serde(default)]
    pub formatted_name: Option<String>,
    /// Given name.
    #[serde(default)]
    pub first_name: Option<String>,
    /// Family name.
    #[serde(default)]
    pub last_name: Option<String>,
}

/// Reaction event: somebody reacted to one of our messages.
///
/// ```
/// use meta_whatsapp::types::Reaction;
///
/// let reaction: Reaction = serde_json::from_str(r#"{"message_id":"wamid.1","emoji":"👍"}"#).unwrap();
/// assert_eq!(reaction.emoji.as_deref(), Some("👍"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Reaction {
    /// Message that was reacted to.
    #[serde(default)]
    pub message_id: String,
    /// Emoji, `None` when the reaction was removed.
    #[serde(default)]
    pub emoji: Option<String>,
}

/// The `nfm_reply` block Meta sends when a customer completes a Flow.
///
/// The wire shape is small and slightly surprising: the flow's answers are not
/// inline, they arrive in `response_json` as a **JSON document encoded in a
/// string**, and `flow_token` lives inside that document — there is no
/// top-level token field. `name` is the payload type (Meta sends `"flow"`),
/// `body` is the text shown in the chat.
///
/// Source: `https://developers.facebook.com/docs/whatsapp/flows/reference/flowswebhooks/`
/// documents `interactive.nfm_reply.name`, `.body` and `.response_json`, all
/// three as strings.
///
/// ```
/// use meta_whatsapp::types::FlowReply;
///
/// let reply: FlowReply = serde_json::from_str(
///     r#"{"name":"flow","body":"Sent",
///         "response_json":"{\"flow_token\":\"abc\",\"city\":\"Berlin\"}"}"#,
/// )?;
/// assert_eq!(reply.name.as_deref(), Some("flow"));
/// assert_eq!(reply.body.as_deref(), Some("Sent"));
/// let response = reply.response()?;
/// assert_eq!(response.flow_token(), Some("abc"));
/// assert_eq!(response.field("city"), Some("Berlin"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FlowReply {
    /// The flow's answers, JSON-encoded in a string exactly as Meta sends them.
    #[serde(default, alias = "responseJson")]
    pub response_json: Option<String>,
    /// Payload type; Meta sends `"flow"`.
    #[serde(default)]
    pub name: Option<String>,
    /// Text Meta shows in the chat (`"Sent"` for a completed flow).
    #[serde(default)]
    pub body: Option<String>,
    /// Keys this crate does not model, kept verbatim.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl FlowReply {
    /// Decodes `response_json` into a [`FlowResponse`].
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    ///
    /// let reply = FlowReply {
    ///     response_json: Some(r#"{"flow_token":"t"}"#.to_string()),
    ///     ..FlowReply::default()
    /// };
    /// assert_eq!(reply.response()?.flow_token(), Some("t"));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`crate::Error::Payload`] when `response_json` is absent or is not valid JSON.
    /// The raw string stays on the struct, so a caller can log what arrived.
    pub fn response(&self) -> crate::error::Result<FlowResponse> {
        let raw = self.response_json.as_deref().ok_or_else(|| {
            crate::error::Error::Payload("nfm_reply.response_json is missing".to_string())
        })?;
        let value: Value = serde_json::from_str(raw).map_err(|error| {
            crate::error::Error::Payload(format!(
                "nfm_reply.response_json is not valid JSON: {error}"
            ))
        })?;
        Ok(FlowResponse { value })
    }
}

/// The decoded `nfm_reply.response_json` document.
///
/// Keys are the ones the flow itself defines (screen and component ids, plus
/// `flow_token`), so nothing here is typed field by field: [`FlowResponse::get`]
/// returns the raw value, [`FlowResponse::field`] the string-valued ones.
///
/// ```
/// use meta_whatsapp::types::FlowReply;
///
/// let reply: FlowReply = serde_json::from_str(
///     r#"{"response_json":"{\"flow_token\":\"t\",\"screen_0_Name\":\"Ada\",\"items\":[1,2]}"}"#,
/// )?;
/// let response = reply.response()?;
/// assert_eq!(response.field("screen_0_Name"), Some("Ada"));
/// assert_eq!(response.get("items"), Some(&serde_json::json!([1, 2])));
/// assert_eq!(response.get("missing"), None);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowResponse {
    value: Value,
}

impl FlowResponse {
    /// The decoded document, whatever JSON type it turned out to be.
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    ///
    /// let reply = FlowReply {
    ///     response_json: Some(r#"{"a":1}"#.to_string()),
    ///     ..FlowReply::default()
    /// };
    /// assert_eq!(reply.response()?.as_value(), &serde_json::json!({"a": 1}));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn as_value(&self) -> &Value {
        &self.value
    }

    /// The `flow_token` the flow echoed back, when it sent one.
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    ///
    /// let reply = FlowReply {
    ///     response_json: Some(r#"{"flow_token":"tok"}"#.to_string()),
    ///     ..FlowReply::default()
    /// };
    /// assert_eq!(reply.response()?.flow_token(), Some("tok"));
    ///
    /// let without = FlowReply {
    ///     response_json: Some("{}".to_string()),
    ///     ..FlowReply::default()
    /// };
    /// assert_eq!(without.response()?.flow_token(), None);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn flow_token(&self) -> Option<&str> {
        self.field("flow_token")
    }

    /// Raw value of a key the flow defined, `None` when absent or when the
    /// document is not an object.
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    ///
    /// let reply = FlowReply {
    ///     response_json: Some(r#"{"qty":3}"#.to_string()),
    ///     ..FlowReply::default()
    /// };
    /// assert_eq!(reply.response()?.get("qty"), Some(&serde_json::json!(3)));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.value.as_object()?.get(name)
    }

    /// String value of a key the flow defined; non-string values and missing
    /// keys both answer `None`, use [`FlowResponse::get`] to see the raw value.
    ///
    /// ```
    /// use meta_whatsapp::types::FlowReply;
    ///
    /// let reply = FlowReply {
    ///     response_json: Some(r#"{"city":"Berlin","n":1}"#.to_string()),
    ///     ..FlowReply::default()
    /// };
    /// let response = reply.response()?;
    /// assert_eq!(response.field("city"), Some("Berlin"));
    /// assert_eq!(response.field("n"), None);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&str> {
        self.get(name)?.as_str()
    }
}

/// Reply to an interactive message.
///
/// A flow reply (`type == "nfm_reply"`) is decoded into [`FlowReply`] here, so
/// the stringified `response_json` does not have to be parsed again downstream.
///
/// ```rust
/// use meta_whatsapp::types::InteractiveReply;
///
/// let reply: InteractiveReply = serde_json::from_str(
///     r#"{"type":"button_reply","button_reply":{"id":"yes","title":"Ja"}}"#,
/// ).unwrap();
/// assert_eq!(reply.button_reply.unwrap().id, "yes");
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InteractiveReply {
    /// `button_reply`, `list_reply` or `nfm_reply`.
    #[serde(rename = "type", default)]
    pub reply_type: String,
    /// Quick-reply selection.
    #[serde(default)]
    pub button_reply: Option<ReplyRef>,
    /// List selection.
    #[serde(default)]
    pub list_reply: Option<ReplyRef>,
    /// Flow completion reply (`type == "nfm_reply"`).
    #[serde(default)]
    pub nfm_reply: Option<FlowReply>,
}

/// The id/title pair echoed for a button or list selection.
///
/// ```
/// use meta_whatsapp::types::ReplyRef;
///
/// let reference: ReplyRef = serde_json::from_str(r#"{"id":"a","title":"A","description":"d"}"#).unwrap();
/// assert_eq!(reference.description.as_deref(), Some("d"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReplyRef {
    /// Id you set when sending the interactive message.
    #[serde(default)]
    pub id: String,
    /// Title shown to the user.
    #[serde(default)]
    pub title: String,
    /// Row description, list replies only.
    #[serde(default)]
    pub description: Option<String>,
}

/// Tap on a template quick-reply button (`type == "button"`).
///
/// ```
/// use meta_whatsapp::types::ButtonReply;
///
/// let reply: ButtonReply = serde_json::from_str(r#"{"text":"Ja","payload":"yes"}"#).unwrap();
/// assert_eq!(reply.payload.as_deref(), Some("yes"));
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ButtonReply {
    /// Button label.
    #[serde(default)]
    pub text: String,
    /// Button payload.
    #[serde(default)]
    pub payload: Option<String>,
}

/// Quoted-message and forwarding metadata.
///
/// ```
/// use meta_whatsapp::types::Context;
///
/// let context: Context =
///     serde_json::from_str(r#"{"from":"4915","id":"wamid.1","forwarded":true}"#).unwrap();
/// assert!(context.forwarded);
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Context {
    /// Original sender.
    #[serde(default)]
    pub from: Option<String>,
    /// Quoted message id.
    #[serde(default)]
    pub id: String,
    /// Forwarded flag.
    #[serde(default)]
    pub forwarded: bool,
    /// Forwarded many times.
    #[serde(default)]
    pub frequently_forwarded: bool,
    /// Product the user asked about (catalog messages).
    #[serde(default)]
    pub referred_product: Option<Value>,
}

/// Delivery status for a message we sent.
///
/// ```
/// use meta_whatsapp::Status;
///
/// let status: Status = serde_json::from_str(
///     r#"{"id":"wamid.1","status":"delivered","timestamp":"1729600005","recipient_id":"4915"}"#,
/// ).unwrap();
/// assert_eq!(status.status, "delivered");
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Status {
    /// Message id this status belongs to.
    #[serde(default)]
    pub id: String,
    /// `sent`, `delivered`, `read` or `failed`.
    #[serde(default)]
    pub status: String,
    /// Unix timestamp as string.
    #[serde(default)]
    pub timestamp: String,
    /// Recipient `WhatsApp` id.
    #[serde(default)]
    pub recipient_id: String,
    /// Echo of the `biz_opaque_callback_data` sent with the message.
    #[serde(default)]
    pub biz_opaque_callback_data: Option<String>,
    /// Conversation/pricing block, passed through.
    #[serde(default)]
    pub conversation: Option<Value>,
    /// Pricing block, passed through.
    #[serde(default)]
    pub pricing: Option<Value>,
    /// Failure details for `status == "failed"`.
    #[serde(default)]
    pub errors: Vec<StatusError>,
}

impl Status {
    /// `timestamp` parsed as unix seconds.
    ///
    /// ```
    /// use meta_whatsapp::Status;
    ///
    /// let status: Status = serde_json::from_str(r#"{"id":"1","status":"sent","timestamp":"7"}"#).unwrap();
    /// assert_eq!(status.timestamp_secs(), Some(7));
    /// ```
    #[must_use]
    pub fn timestamp_secs(&self) -> Option<u64> {
        self.timestamp.trim().parse().ok()
    }
}

/// Error attached to a failed status or an `errors` message.
///
/// ```
/// use meta_whatsapp::types::StatusError;
///
/// let error: StatusError = serde_json::from_str(
///     r#"{"code":131026,"title":"Message undeliverable","message":"Recipient not on WhatsApp"}"#,
/// ).unwrap();
/// assert_eq!(error.code, 131_026);
///
/// ```
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct StatusError {
    /// Meta error code.
    #[serde(default)]
    pub code: i64,
    /// Short title.
    #[serde(default)]
    pub title: String,
    /// Longer explanation.
    #[serde(default)]
    pub message: Option<String>,
    /// Structured detail, passed through.
    #[serde(default)]
    pub error_data: Option<Value>,
}

impl WebhookPayload {
    /// The changes that carry a `value.metadata`, paired with that change's
    /// `phone_number_id` (empty when the metadata block omits it).
    ///
    /// Both routing iterators read the id from the same place, so the filter
    /// lives here once: a change without `metadata` has no id to attribute an
    /// event to, and inventing one would misroute it to another tenant.
    fn attributable_changes(&self) -> impl Iterator<Item = (&str, &ChangeValue)> {
        self.entry
            .iter()
            .flat_map(|entry| entry.changes.iter())
            .filter_map(|change| {
                let phone_number_id = change.value.metadata.as_ref()?.phone_number_id.as_str();
                Some((phone_number_id, &change.value))
            })
    }

    /// Iterates all inbound messages together with the `phone_number_id` they
    /// arrived on — the value a multi-tenant service routes by.
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    ///
    /// let payload: WebhookPayload =
    ///     serde_json::from_str(r#"{"object":"whatsapp_business_account","entry":[]}"#).unwrap();
    /// assert_eq!(payload.inbound_messages().count(), 0);
    /// ```
    pub fn inbound_messages(&self) -> impl Iterator<Item = (&str, &Message)> {
        self.attributable_changes()
            .flat_map(|(phone_number_id, value)| {
                value
                    .messages
                    .iter()
                    .map(move |message| (phone_number_id, message))
            })
    }

    /// Iterates all delivery statuses.
    ///
    /// The `phone_number_id` is dropped here; use
    /// [`WebhookPayload::statuses_with_phone_number_id`] when the status has to
    /// be routed to the number it belongs to.
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    ///
    /// let payload: WebhookPayload =
    ///     serde_json::from_str(r#"{"object":"whatsapp_business_account","entry":[]}"#).unwrap();
    /// assert_eq!(payload.statuses().count(), 0);
    /// ```
    pub fn statuses(&self) -> impl Iterator<Item = &Status> {
        self.entry
            .iter()
            .flat_map(|e| e.changes.iter())
            .flat_map(|c| c.value.statuses.iter())
    }

    /// Iterates all delivery statuses together with the `phone_number_id` they
    /// arrived on — the sibling [`WebhookPayload::statuses`] lacks.
    ///
    /// A `sent`/`delivered`/`read`/`failed` event carries no number of its own;
    /// the id lives in the same `value.metadata` that
    /// [`WebhookPayload::inbound_messages`] already reads, so multi-tenant
    /// routing of statuses is possible without walking `entry[].changes[]` by
    /// hand.
    ///
    /// Filtering is identical to [`WebhookPayload::inbound_messages`]: a change
    /// whose `value.metadata` is absent is skipped (no id, nothing to route by),
    /// and `metadata` that is present but has no `phone_number_id` yields `""`
    /// rather than being dropped, so both iterators agree on the same change.
    /// [`WebhookPayload::statuses`] still sees everything, attributable or not.
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    ///
    /// let payload: WebhookPayload = serde_json::from_str(
    ///     r#"{"object":"whatsapp_business_account","entry":[
    ///         {"id":"1","changes":[{"field":"messages","value":{
    ///             "metadata":{"display_phone_number":"15550000001","phone_number_id":"111"},
    ///             "statuses":[{"id":"wamid.1","status":"delivered","timestamp":"10"}]}}]},
    ///         {"id":"2","changes":[{"field":"messages","value":{
    ///             "statuses":[{"id":"wamid.2","status":"read","timestamp":"11"}]}}]}]}"#,
    /// ).unwrap();
    ///
    /// let routed: Vec<_> = payload
    ///     .statuses_with_phone_number_id()
    ///     .map(|(phone_number_id, status)| (phone_number_id, status.id.as_str()))
    ///     .collect();
    /// // The change without metadata is skipped: no id to route by.
    /// assert_eq!(routed, vec![("111", "wamid.1")]);
    /// // The unpaired view is unchanged and still sees both.
    /// assert_eq!(payload.statuses().count(), 2);
    /// ```
    pub fn statuses_with_phone_number_id(&self) -> impl Iterator<Item = (&str, &Status)> {
        self.attributable_changes()
            .flat_map(|(phone_number_id, value)| {
                value
                    .statuses
                    .iter()
                    .map(move |status| (phone_number_id, status))
            })
    }

    /// Iterates every change, including non-message events.
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    ///
    /// let payload: WebhookPayload = serde_json::from_str(
    ///     r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[
    ///         {"field":"message_template_status_update","value":{"event":"APPROVED"}}]}]}"#,
    /// ).unwrap();
    /// assert_eq!(payload.changes().count(), 1);
    /// ```
    pub fn changes(&self) -> impl Iterator<Item = &Change> {
        self.entry.iter().flat_map(|entry| entry.changes.iter())
    }

    /// Every numeric-looking timestamp in the payload (messages and statuses).
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    ///
    /// let payload: WebhookPayload = serde_json::from_str(
    ///     r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
    ///         "field":"messages","value":{
    ///             "metadata":{"display_phone_number":"1","phone_number_id":"2"},
    ///             "messages":[{"id":"m","from":"3","type":"text","timestamp":"100"}]}}]}]}"#,
    /// ).unwrap();
    /// assert_eq!(payload.timestamps().collect::<Vec<_>>(), vec!["100"]);
    /// ```
    pub fn timestamps(&self) -> impl Iterator<Item = &str> {
        self.entry
            .iter()
            .flat_map(|entry| entry.changes.iter())
            .flat_map(|change| {
                change
                    .value
                    .messages
                    .iter()
                    .map(|message| message.timestamp.as_str())
                    .chain(
                        change
                            .value
                            .statuses
                            .iter()
                            .map(|status| status.timestamp.as_str()),
                    )
            })
    }

    /// The newest parsable timestamp in the payload, if any.
    ///
    /// ```
    /// use meta_whatsapp::WebhookPayload;
    /// assert_eq!(WebhookPayload::default().newest_timestamp(), None);
    /// ```
    #[must_use]
    pub fn newest_timestamp(&self) -> Option<u64> {
        self.timestamps()
            .filter_map(|raw| raw.trim().parse::<u64>().ok())
            .max()
    }
}

fn take_string(map: &mut Map<String, Value>, key: &str) -> Option<String> {
    match map.remove(key)? {
        Value::String(value) => Some(value),
        other => {
            map.insert(key.to_string(), other);
            None
        }
    }
}

fn take_value<T: serde::de::DeserializeOwned>(
    map: &mut Map<String, Value>,
    key: &str,
) -> Option<T> {
    let value = map.remove(key)?;
    serde_json::from_value(value).ok()
}

/// Parses an array element-wise so one malformed entry does not discard the
/// rest of the batch.
fn take_vec<T: serde::de::DeserializeOwned>(map: &mut Map<String, Value>, key: &str) -> Vec<T> {
    match map.remove(key) {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        Some(other) => {
            map.insert(key.to_string(), other);
            Vec::new()
        }
        None => Vec::new(),
    }
}
