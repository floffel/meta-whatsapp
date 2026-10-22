//! Graph API client for sending messages.

use serde_json::json;

use crate::error::Result;
use crate::transport::Transport;

/// Default Graph API version used when none is given.
pub const DEFAULT_API_VERSION: &str = "v21.0";

/// Client bound to one business phone number.
#[derive(Debug)]
pub struct WhatsAppClient<T: Transport> {
    phone_number_id: String,
    token: String,
    api_version: String,
    base_url: String,
    transport: T,
}

impl<T: Transport> WhatsAppClient<T> {
    /// Creates a client for `phone_number_id` using `token`.
    pub fn new(phone_number_id: impl Into<String>, token: impl Into<String>, transport: T) -> Self {
        Self {
            phone_number_id: phone_number_id.into(),
            token: token.into(),
            api_version: DEFAULT_API_VERSION.to_string(),
            base_url: "https://graph.facebook.com".to_string(),
            transport,
        }
    }

    /// Overrides the API version (for example `v20.0`).
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Overrides the base URL — useful for tests or a proxy.
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// The messages endpoint for the bound phone number.
    #[must_use]
    pub fn messages_url(&self) -> String {
        format!(
            "{}/{}/{}/messages",
            self.base_url.trim_end_matches('/'),
            self.api_version,
            self.phone_number_id
        )
    }

    /// Sends a plain text message.
    ///
    /// # Errors
    ///
    /// Propagates transport and Graph API errors.
    pub async fn send_text(&self, to: &str, body: &str) -> Result<serde_json::Value> {
        let payload = json!({
            "messaging_product": "whatsapp",
            "recipient_type": "individual",
            "to": to,
            "type": "text",
            "text": { "preview_url": false, "body": body },
        });
        self.transport
            .post_json(&self.messages_url(), &self.token, &payload)
            .await
    }

    /// Marks an inbound message as read (blue ticks).
    ///
    /// # Errors
    ///
    /// Propagates transport and Graph API errors.
    pub async fn mark_read(&self, message_id: &str) -> Result<serde_json::Value> {
        let payload = json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id,
        });
        self.transport
            .post_json(&self.messages_url(), &self.token, &payload)
            .await
    }
}
