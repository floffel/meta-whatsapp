//! Typed view of the webhook payload Meta posts for `whatsapp_business_account`.

use serde::{Deserialize, Serialize};

/// Top level webhook envelope.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WebhookPayload {
    /// Always `whatsapp_business_account` for this API.
    pub object: String,
    /// One entry per business account that received an event.
    #[serde(default)]
    pub entry: Vec<Entry>,
}

/// One business account's batch of changes.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Entry {
    /// `WhatsApp` Business Account id.
    pub id: String,
    /// Events for that account.
    #[serde(default)]
    pub changes: Vec<Change>,
}

/// A single change, either an inbound message or a delivery status.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Change {
    /// `messages` for both message and status events.
    pub field: String,
    /// The event body.
    pub value: ChangeValue,
}

/// Body of a `messages` change.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ChangeValue {
    /// `whatsapp` for this API.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messaging_product: Option<String>,
    /// Which phone number received the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
    /// Inbound messages.
    #[serde(default)]
    pub messages: Vec<Message>,
    /// Delivery/read statuses for messages we sent.
    #[serde(default)]
    pub statuses: Vec<Status>,
}

/// Phone number metadata of the receiving business number.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Metadata {
    /// Display phone number.
    pub display_phone_number: String,
    /// Id used in the Graph API path.
    pub phone_number_id: String,
}

/// An inbound message.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Message {
    /// Provider message id (use it as idempotency key).
    pub id: String,
    /// Sender's `WhatsApp` id (phone number, digits only).
    pub from: String,
    /// Unix timestamp as string, as sent by Meta.
    #[serde(default)]
    pub timestamp: String,
    /// `text`, `image`, `audio`, `document`, `button`, `interactive`, ….
    #[serde(rename = "type")]
    pub message_type: String,
    /// Present when `type == "text"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
}

/// Text body of a text message.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Text {
    /// The message text.
    pub body: String,
}

/// Delivery status for a message we sent.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Status {
    /// Message id this status belongs to.
    pub id: String,
    /// `sent`, `delivered`, `read` or `failed`.
    pub status: String,
    /// Unix timestamp as string.
    #[serde(default)]
    pub timestamp: String,
    /// Recipient `WhatsApp` id.
    #[serde(default)]
    pub recipient_id: String,
}

impl WebhookPayload {
    /// Iterates all inbound messages together with the `phone_number_id` they
    /// arrived on — the value a multi-tenant service routes by.
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
    pub fn statuses(&self) -> impl Iterator<Item = &Status> {
        self.entry
            .iter()
            .flat_map(|e| e.changes.iter())
            .flat_map(|c| c.value.statuses.iter())
    }
}
