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

pub use crate::commerce::{
    AddressField, AddressFieldType, AddressReply, Order, OrderItem, SavedAddress,
};

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
    /// `text`, `image`, `audio`, `document`, `button`, `interactive`, ….
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
    /// Present when `type == "order"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Order>,
    /// Present when `type == "address"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<AddressReply>,
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

    /// The order block, if this is an inbound order.
    ///
    /// ```
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"1","from":"2","type":"order","order":{"catalog_id":"c","product_items":[
    ///         {"product_retailer_id":"p","quantity":1,"item_price":4.0,"currency":"EUR"}]}}"#,
    /// )?;
    /// assert_eq!(message.order().map(|order| order.product_items.len()), Some(1));
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn order(&self) -> Option<&Order> {
        self.order.as_ref()
    }

    /// The address block, from either an interactive reply or an `address`
    /// message.
    ///
    /// ```
    /// use meta_whatsapp::Message;
    ///
    /// let message: Message = serde_json::from_str(
    ///     r#"{"id":"1","from":"2","type":"interactive","interactive":{"type":"address_message","address_message":{"country":"DE","values":[{"type":"address","name":"city","value":"Berlin"}]}}}"#,
    /// )?;
    /// assert_eq!(message.address().and_then(|address| address.field("city")), Some("Berlin"));
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn address(&self) -> Option<&AddressReply> {
        self.interactive
            .as_ref()
            .and_then(|interactive| interactive.address_message.as_ref())
            .or(self.address.as_ref())
    }

    /// `timestamp` parsed as unix seconds.
    ///
    /// ```
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

/// Reply to an interactive message: button, list, flow or address.
///
/// ```
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
    /// `button_reply`, `list_reply`, `nfm_reply` or `address_message`.
    #[serde(rename = "type", default)]
    pub reply_type: String,
    /// Quick-reply selection.
    #[serde(default)]
    pub button_reply: Option<ReplyRef>,
    /// List selection.
    #[serde(default)]
    pub list_reply: Option<ReplyRef>,
    /// Flow response payload.
    #[serde(default)]
    pub nfm_reply: Option<Value>,
    /// Address response (`type == "address_message"`).
    #[serde(default)]
    pub address_message: Option<AddressReply>,
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
        self.entry
            .iter()
            .flat_map(|e| e.changes.iter())
            .filter_map(|c| {
                let phone_number_id = c.value.metadata.as_ref()?.phone_number_id.as_str();
                Some(c.value.messages.iter().map(move |m| (phone_number_id, m)))
            })
            .flatten()
    }

    /// Iterates all delivery statuses.
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
