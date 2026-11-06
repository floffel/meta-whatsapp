//! Webhook subscription management for a WhatsApp Business Account and app.
//!
//! Three WABA-level Graph API calls sit behind this module:
//!
//! | Call | Endpoint |
//! | --- | --- |
//! | list | `GET /{waba-id}/subscribed_apps` |
//! | subscribe | `POST /{waba-id}/subscribed_apps` |
//! | unsubscribe | `DELETE /{waba-id}/subscribed_apps` |
//!
//! App-level field selection is a separate edge:
//!
//! | Call | Endpoint |
//! | --- | --- |
//! | list app fields | `GET /{app-id}/subscriptions` |
//! | set app fields | `POST /{app-id}/subscriptions` |
//!
//! The app-level call chooses which fields Meta sends to an app for every
//! WhatsApp Business Account attached to it. Attaching a WABA remains the
//! separate `POST /{waba-id}/subscribed_apps` step.
//!
//! A subscription can silently disappear — the app was re-deployed with a new
//! callback URL, an app secret changed, or an admin removed it in WhatsApp
//! Manager. [`crate::WhatsAppClient::ensure_webhook_subscription`] re-lists and
//! only subscribes when the app is missing, which makes it safe to call from a
//! startup check *and* from an error path.
//!
//! ```rust
//! use meta_whatsapp::subscription::WebhookSubscriptions;
//!
//! let subscriptions = WebhookSubscriptions::parse(&serde_json::json!({
//!     "data": [{ "whatsapp_business_api_data": { "id": "1234", "name": "Bot" } }]
//! }))?;
//! assert!(subscriptions.has_app("1234"));
//! assert!(!subscriptions.has_app("9999"));
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// One application subscribed to a WABA's webhooks.
///
/// ```
/// use meta_whatsapp::subscription::SubscribedApp;
///
/// let app: SubscribedApp = serde_json::from_str(
///     r#"{"id":"1234","name":"Bot","link":"https://example.test"}"#,
/// )?;
/// assert_eq!(app.id.as_deref(), Some("1234"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SubscribedApp {
    /// Application id.
    #[serde(default)]
    pub id: Option<String>,
    /// Application name.
    #[serde(default)]
    pub name: Option<String>,
    /// Link to the application's settings page.
    #[serde(default)]
    pub link: Option<String>,
}

/// The subscriptions of one WABA, as returned by the list call.
///
/// ```
/// use meta_whatsapp::subscription::WebhookSubscriptions;
///
/// let subs = WebhookSubscriptions::parse(&serde_json::json!({"data": []}))?;
/// assert!(subs.is_empty());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WebhookSubscriptions {
    /// Subscribed applications.
    #[serde(default)]
    pub data: Vec<SubscribedApp>,
}

/// One WhatsApp webhook field that can be subscribed at the app level.
///
/// [`SubscriptionField::Unknown`] keeps a field Meta adds later instead of
/// making the whole subscription response unreadable.
///
/// ```
/// use meta_whatsapp::subscription::SubscriptionField;
///
/// assert_eq!(SubscriptionField::parse("messages"), SubscriptionField::Messages);
/// assert_eq!(SubscriptionField::Messages.as_str(), "messages");
/// assert_eq!(
///     SubscriptionField::parse("account_update"),
///     SubscriptionField::Unknown("account_update".into())
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubscriptionField {
    /// Inbound messages and delivery/read statuses.
    Messages,
    /// Template approval, rejection and other status changes.
    MessageTemplateStatusUpdate,
    /// Changes to a phone number's quality rating.
    PhoneNumberQualityUpdate,
    /// A field this crate does not know; the raw wire name is kept.
    Unknown(String),
}

impl SubscriptionField {
    /// Parses a field name, keeping unknown names instead of failing.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionField;
    ///
    /// assert_eq!(
    ///     SubscriptionField::parse(" message_template_status_update "),
    ///     SubscriptionField::MessageTemplateStatusUpdate
    /// );
    /// assert_eq!(
    ///     SubscriptionField::parse("phone_number_quality_update"),
    ///     SubscriptionField::PhoneNumberQualityUpdate
    /// );
    /// assert_eq!(
    ///     SubscriptionField::parse("future_field"),
    ///     SubscriptionField::Unknown("future_field".into())
    /// );
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "messages" => Self::Messages,
            "message_template_status_update" => Self::MessageTemplateStatusUpdate,
            "phone_number_quality_update" => Self::PhoneNumberQualityUpdate,
            other => Self::Unknown(other.to_string()),
        }
    }

    /// The field name as Meta spells it on the wire.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionField;
    ///
    /// assert_eq!(SubscriptionField::Messages.as_str(), "messages");
    /// assert_eq!(
    ///     SubscriptionField::MessageTemplateStatusUpdate.as_str(),
    ///     "message_template_status_update"
    /// );
    /// assert_eq!(
    ///     SubscriptionField::PhoneNumberQualityUpdate.as_str(),
    ///     "phone_number_quality_update"
    /// );
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Messages => "messages",
            Self::MessageTemplateStatusUpdate => "message_template_status_update",
            Self::PhoneNumberQualityUpdate => "phone_number_quality_update",
            Self::Unknown(raw) => raw,
        }
    }
}

impl Serialize for SubscriptionField {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SubscriptionField {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// One field entry in the app-level subscription read response.
///
/// Meta's documented response wraps each field as `{name, version}`; older
/// examples send bare strings, so both shapes parse.
///
/// ```
/// use meta_whatsapp::subscription::{SubscribedWebhookField, SubscriptionField};
///
/// let field: SubscribedWebhookField = serde_json::from_value(serde_json::json!({
///     "name": "messages", "version": "v23.0"
/// }))?;
/// assert_eq!(field.name, SubscriptionField::Messages);
/// assert_eq!(field.version.as_deref(), Some("v23.0"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct SubscribedWebhookField {
    /// Field selected for the subscription.
    pub name: SubscriptionField,
    /// Graph API version Meta associates with the field, when it sends one.
    pub version: Option<String>,
}

impl SubscribedWebhookField {
    /// A field entry with no version.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{SubscribedWebhookField, SubscriptionField};
    ///
    /// let field = SubscribedWebhookField::new(SubscriptionField::Messages);
    /// assert_eq!(field.name.as_str(), "messages");
    /// assert!(field.version.is_none());
    /// ```
    #[must_use]
    pub fn new(name: SubscriptionField) -> Self {
        Self {
            name,
            version: None,
        }
    }

    /// Adds the Graph API version.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{SubscribedWebhookField, SubscriptionField};
    ///
    /// let field = SubscribedWebhookField::new(SubscriptionField::Messages)
    ///     .version("v23.0");
    /// assert_eq!(field.version.as_deref(), Some("v23.0"));
    /// ```
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
}

impl<'de> Deserialize<'de> for SubscribedWebhookField {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Named {
                name: String,
                #[serde(default)]
                version: Option<String>,
            },
            Plain(String),
        }

        Ok(match Wire::deserialize(deserializer)? {
            Wire::Named { name, version } => Self {
                name: SubscriptionField::parse(&name),
                version,
            },
            Wire::Plain(name) => Self::new(SubscriptionField::parse(&name)),
        })
    }
}

/// One object subscription in the app-level subscription read response.
///
/// ```
/// use meta_whatsapp::subscription::{AppWebhookSubscription, SubscriptionField};
///
/// let subscription: AppWebhookSubscription = serde_json::from_value(serde_json::json!({
///     "object": "whatsapp_business_account",
///     "callback_url": "https://example.test/hooks/whatsapp",
///     "active": true,
///     "fields": ["messages", {"name": "message_template_status_update", "version": "v23.0"}]
/// }))?;
/// assert!(subscription.has_field(&SubscriptionField::Messages));
/// assert!(subscription.is_whatsapp_business_account());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct AppWebhookSubscription {
    /// Object type, e.g. `whatsapp_business_account`.
    pub object: String,
    /// Callback URL receiving notifications for this object.
    pub callback_url: Option<String>,
    /// Whether Meta considers this subscription active.
    pub active: Option<bool>,
    /// Selected fields.
    pub fields: Vec<SubscribedWebhookField>,
}

impl AppWebhookSubscription {
    /// Whether this is the WhatsApp Business Account subscription.
    ///
    /// ```
    /// use meta_whatsapp::subscription::AppWebhookSubscription;
    ///
    /// let subscription: AppWebhookSubscription = serde_json::from_value(serde_json::json!({
    ///     "object": "whatsapp_business_account"
    /// }))?;
    /// assert!(subscription.is_whatsapp_business_account());
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn is_whatsapp_business_account(&self) -> bool {
        self.object == "whatsapp_business_account"
    }

    /// Whether `field` is in the selected field list.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{
    ///     AppWebhookSubscription, SubscribedWebhookField, SubscriptionField,
    /// };
    ///
    /// let subscription: AppWebhookSubscription = serde_json::from_value(serde_json::json!({
    ///     "object": "whatsapp_business_account",
    ///     "fields": [{"name": "messages"}]
    /// }))?;
    /// assert!(subscription.has_field(&SubscriptionField::Messages));
    /// assert!(!subscription.has_field(&SubscriptionField::PhoneNumberQualityUpdate));
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn has_field(&self, field: &SubscriptionField) -> bool {
        self.fields.iter().any(|entry| entry.name == *field)
    }
}

/// The app-level subscriptions returned by `GET /{app-id}/subscriptions`.
///
/// ```
/// use meta_whatsapp::subscription::AppWebhookSubscriptions;
///
/// let subscriptions = AppWebhookSubscriptions::parse(&serde_json::json!({
///     "data": [{
///         "object": "whatsapp_business_account",
///         "callback_url": "https://example.test/hooks/whatsapp",
///         "active": true,
///         "fields": [{"name": "messages", "version": "v23.0"}]
///     }]
/// }))?;
/// assert_eq!(
///     subscriptions.whatsapp_business_account().unwrap().callback_url.as_deref(),
///     Some("https://example.test/hooks/whatsapp")
/// );
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct AppWebhookSubscriptions {
    /// Configured object subscriptions.
    pub data: Vec<AppWebhookSubscription>,
}

impl AppWebhookSubscriptions {
    /// Parses the Graph response, retaining serde's error as the source.
    ///
    /// ```
    /// use meta_whatsapp::subscription::AppWebhookSubscriptions;
    ///
    /// assert!(AppWebhookSubscriptions::parse(&serde_json::json!({"data": []})).is_ok());
    /// let error = AppWebhookSubscriptions::parse(&serde_json::json!({"data": "not an array"}))
    ///     .unwrap_err();
    /// assert!(std::error::Error::source(&error).is_some(), "serde error is retained");
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the body does not have the documented object and
    /// `data[]` shape.
    pub fn parse(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(|source| Error::Json { source })
    }

    /// The WhatsApp Business Account subscription, when configured.
    ///
    /// ```
    /// use meta_whatsapp::subscription::AppWebhookSubscriptions;
    ///
    /// let subscriptions = AppWebhookSubscriptions::parse(&serde_json::json!({"data": [
    ///     {"object": "page"},
    ///     {"object": "whatsapp_business_account"}
    /// ]}))?;
    /// assert_eq!(
    ///     subscriptions.whatsapp_business_account().unwrap().object,
    ///     "whatsapp_business_account"
    /// );
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn whatsapp_business_account(&self) -> Option<&AppWebhookSubscription> {
        self.data
            .iter()
            .find(|entry| entry.is_whatsapp_business_account())
    }
}

/// Body of `POST /{app-id}/subscriptions`.
///
/// The callback URL and verify token are secrets in the same sense as every
/// other webhook configuration value: `Debug` redacts the token and shows only
/// the URL's character count.
///
/// ```
/// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
///
/// let request = AppWebhookSubscriptionRequest::new(
///     "https://example.test/hooks/whatsapp",
///     "s3cret",
///     [SubscriptionField::Messages, SubscriptionField::MessageTemplateStatusUpdate],
/// );
/// let body = request.to_json()?;
/// assert_eq!(body["object"], "whatsapp_business_account");
/// assert_eq!(body["fields"], "messages,message_template_status_update");
/// assert!(!format!("{request:?}").contains("s3cret"));
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AppWebhookSubscriptionRequest {
    callback_url: String,
    verify_token: String,
    fields: Vec<SubscriptionField>,
}

impl AppWebhookSubscriptionRequest {
    /// Creates a request, preserving field order and removing duplicates.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
    ///
    /// let request = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp",
    ///     "s3cret",
    ///     [SubscriptionField::Messages, SubscriptionField::Messages],
    /// );
    /// assert_eq!(request.fields(), [SubscriptionField::Messages]);
    /// ```
    #[must_use]
    pub fn new(
        callback_url: impl Into<String>,
        verify_token: impl Into<String>,
        fields: impl IntoIterator<Item = SubscriptionField>,
    ) -> Self {
        let mut unique = Vec::new();
        for field in fields {
            if !unique.contains(&field) {
                unique.push(field);
            }
        }
        Self {
            callback_url: callback_url.into(),
            verify_token: verify_token.into(),
            fields: unique,
        }
    }

    /// The callback URL Meta will call.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
    ///
    /// let request = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp", "token", [SubscriptionField::Messages],
    /// );
    /// assert_eq!(request.callback_url(), "https://example.test/hooks/whatsapp");
    /// ```
    #[must_use]
    pub fn callback_url(&self) -> &str {
        &self.callback_url
    }

    /// The selected fields in wire order.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
    ///
    /// let request = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp", "token",
    ///     [SubscriptionField::PhoneNumberQualityUpdate],
    /// );
    /// assert_eq!(request.fields(), [SubscriptionField::PhoneNumberQualityUpdate]);
    /// ```
    #[must_use]
    pub fn fields(&self) -> &[SubscriptionField] {
        &self.fields
    }

    /// Validates the callback URL, token and field list before any request is
    /// built.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
    ///
    /// let valid = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp", "token", [SubscriptionField::Messages],
    /// );
    /// assert!(valid.validate().is_ok());
    /// let invalid = AppWebhookSubscriptionRequest::new(
    ///     "http://example.test/hooks/whatsapp", "token", [SubscriptionField::Messages],
    /// );
    /// assert!(invalid.validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `callback_url`, `verify_token` or `fields`.
    pub fn validate(&self) -> Result<()> {
        if !self.callback_url.starts_with("https://") {
            return Err(Error::validation(
                "callback_url",
                "must be an absolute https:// URL",
            ));
        }
        if self.callback_url.chars().any(char::is_whitespace) {
            return Err(Error::validation(
                "callback_url",
                "must not contain whitespace",
            ));
        }
        let url_chars = self.callback_url.chars().count();
        if url_chars > 1024 {
            return Err(Error::validation(
                "callback_url",
                format!("must be at most 1024 characters, got {url_chars}"),
            ));
        }
        let host = self.callback_url.trim_start_matches("https://");
        if host.is_empty() || host.starts_with('/') {
            return Err(Error::validation("callback_url", "must name a host"));
        }
        if self.verify_token.is_empty() {
            return Err(Error::validation(
                "verify_token",
                "must not be empty - Meta echoes it back on every handshake",
            ));
        }
        let token_chars = self.verify_token.chars().count();
        if token_chars > 256 {
            return Err(Error::validation(
                "verify_token",
                format!("must be at most 256 characters, got {token_chars}"),
            ));
        }
        if self.verify_token.chars().any(char::is_control) {
            return Err(Error::validation(
                "verify_token",
                "must not contain control characters",
            ));
        }
        if self.fields.is_empty() {
            return Err(Error::validation(
                "fields",
                "must contain at least one subscription field",
            ));
        }
        for field in &self.fields {
            let raw = field.as_str();
            if raw.is_empty() {
                return Err(Error::validation("fields", "field names must not be empty"));
            }
            if raw.chars().any(char::is_control) {
                return Err(Error::validation(
                    "fields",
                    "field names must not contain control characters",
                ));
            }
            if raw
                .chars()
                .any(|character| character.is_whitespace() || character == ',')
            {
                return Err(Error::validation(
                    "fields",
                    "field names must not contain whitespace or commas",
                ));
            }
        }
        Ok(())
    }

    /// Validates and renders the exact Graph request body.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{AppWebhookSubscriptionRequest, SubscriptionField};
    ///
    /// let request = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp", "s3cret",
    ///     [SubscriptionField::Messages, SubscriptionField::PhoneNumberQualityUpdate],
    /// );
    /// assert_eq!(request.to_json().unwrap(), serde_json::json!({
    ///     "object": "whatsapp_business_account",
    ///     "callback_url": "https://example.test/hooks/whatsapp",
    ///     "verify_token": "s3cret",
    ///     "fields": "messages,phone_number_quality_update"
    /// }));
    /// ```
    ///
    /// # Errors
    ///
    /// Whatever [`AppWebhookSubscriptionRequest::validate`] rejects.
    pub fn to_json(&self) -> Result<Value> {
        self.validate()?;
        let fields = self
            .fields
            .iter()
            .map(SubscriptionField::as_str)
            .collect::<Vec<_>>()
            .join(",");
        Ok(serde_json::json!({
            "object": "whatsapp_business_account",
            "callback_url": self.callback_url,
            "verify_token": self.verify_token,
            "fields": fields,
        }))
    }

    /// Whether `subscription` already has this object, callback URL and field
    /// set. The verify token is not returned by `GET`, so it cannot be
    /// compared.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{
    ///     AppWebhookSubscriptionRequest, AppWebhookSubscriptions, SubscriptionField,
    /// };
    ///
    /// let request = AppWebhookSubscriptionRequest::new(
    ///     "https://example.test/hooks/whatsapp", "s3cret", [SubscriptionField::Messages],
    /// );
    /// let current = AppWebhookSubscriptions::parse(&serde_json::json!({"data": [{
    ///     "object": "whatsapp_business_account",
    ///     "callback_url": "https://example.test/hooks/whatsapp",
    ///     "active": true,
    ///     "fields": [{"name": "messages"}]
    /// }]}))?;
    /// assert!(request.is_applied_to(&current));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_applied_to(&self, subscriptions: &AppWebhookSubscriptions) -> bool {
        let Some(current) = subscriptions.whatsapp_business_account() else {
            return false;
        };
        if current.callback_url.as_deref() != Some(self.callback_url.as_str())
            || current.active == Some(false)
        {
            return false;
        }
        self.fields.iter().all(|field| current.has_field(field))
            && current.fields.len() == self.fields.len()
    }
}

impl std::fmt::Debug for AppWebhookSubscriptionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppWebhookSubscriptionRequest")
            .field("callback_url_chars", &self.callback_url.chars().count())
            .field("verify_token", &"[redacted]")
            .field("fields", &self.fields)
            .finish()
    }
}

/// The outcome of
/// [`crate::WhatsAppClient::ensure_app_webhook_subscription`].
///
/// ```
/// use meta_whatsapp::subscription::AppSubscriptionState;
///
/// assert!(AppSubscriptionState::AlreadySubscribed.was_written() == false);
/// assert!(AppSubscriptionState::Subscribed.was_written());
/// assert!(AppSubscriptionState::Updated.was_written());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AppSubscriptionState {
    /// The callback and field set were already in place; no write was made.
    AlreadySubscribed,
    /// No WhatsApp Business Account subscription existed and one was created.
    Subscribed,
    /// A subscription existed but its callback URL or field set differed.
    Updated,
}

impl AppSubscriptionState {
    /// Whether this call wrote to the API.
    ///
    /// ```
    /// use meta_whatsapp::subscription::AppSubscriptionState;
    ///
    /// assert!(!AppSubscriptionState::AlreadySubscribed.was_written());
    /// assert!(AppSubscriptionState::Subscribed.was_written());
    /// assert!(AppSubscriptionState::Updated.was_written());
    /// ```
    #[must_use]
    pub fn was_written(self) -> bool {
        !matches!(self, Self::AlreadySubscribed)
    }
}

impl WebhookSubscriptions {
    /// Reads the list out of the response body.
    ///
    /// Each entry may be the app object itself or wrap it in
    /// `whatsapp_business_api_data`, as the published examples show both.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the body is not an object or `data` is not an
    /// array.
    ///
    /// ```
    /// use meta_whatsapp::subscription::WebhookSubscriptions;
    ///
    /// assert!(WebhookSubscriptions::parse(&serde_json::json!({"data": []})).is_ok());
    /// assert!(WebhookSubscriptions::parse(&serde_json::json!({"data": {}})).is_err());
    /// assert!(WebhookSubscriptions::parse(&serde_json::json!("x")).is_err());
    /// ```
    pub fn parse(value: &Value) -> Result<Self> {
        let Value::Object(map) = value else {
            return Err(Error::Payload(
                "expected a JSON object for the subscription list".to_string(),
            ));
        };
        let entries = match map.get("data") {
            None => Vec::new(),
            Some(Value::Array(entries)) => entries.clone(),
            Some(_) => {
                return Err(Error::Payload("`data` was not an array".to_string()));
            }
        };
        let mut apps = Vec::with_capacity(entries.len());
        for entry in entries {
            let inner = entry
                .get("whatsapp_business_api_data")
                .filter(|inner| inner.is_object())
                .unwrap_or(&entry);
            apps.push(serde_json::from_value(inner.clone()).map_err(|error| {
                Error::Payload(format!("unexpected subscribed app shape: {error}"))
            })?);
        }
        Ok(Self { data: apps })
    }

    /// Whether `app_id` is subscribed.
    ///
    /// ```
    /// use meta_whatsapp::subscription::{SubscribedApp, WebhookSubscriptions};
    ///
    /// let mut app = SubscribedApp::default();
    /// app.id = Some("7".into());
    /// let mut subs = WebhookSubscriptions::default();
    /// subs.data = vec![app];
    /// assert!(subs.has_app("7"));
    /// assert!(!subs.has_app("8"));
    /// ```
    #[must_use]
    pub fn has_app(&self, app_id: &str) -> bool {
        self.data
            .iter()
            .any(|app| app.id.as_deref() == Some(app_id))
    }

    /// Whether the WABA has no subscribed application at all.
    ///
    /// ```
    /// use meta_whatsapp::subscription::WebhookSubscriptions;
    /// assert!(WebhookSubscriptions::default().is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// Optional overrides for a subscription: a callback URL that differs from the
/// app default, and the token Meta echoes back in the GET handshake.
///
/// ```
/// use meta_whatsapp::subscription::SubscriptionConfig;
///
/// let config = SubscriptionConfig::new()
///     .callback_url("https://example.test/hooks/whatsapp")
///     .verify_token("s3cret");
/// let body = config.to_json()?;
/// assert_eq!(body["override_callback_uri"], "https://example.test/hooks/whatsapp");
/// assert_eq!(body["verify_token"], "s3cret");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubscriptionConfig {
    /// HTTPS callback that overrides the app-level webhook URL.
    pub callback_url: Option<String>,
    /// Token the webhook handshake must echo back.
    pub verify_token: Option<String>,
}

impl SubscriptionConfig {
    /// An empty configuration (subscribe with the app defaults).
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionConfig;
    /// assert_eq!(SubscriptionConfig::new().to_json().unwrap(), serde_json::json!({}));
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the callback URL override.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionConfig;
    /// assert!(SubscriptionConfig::new().callback_url("https://a.test/h").callback_url.is_some());
    /// ```
    #[must_use]
    pub fn callback_url(mut self, url: impl Into<String>) -> Self {
        self.callback_url = Some(url.into());
        self
    }

    /// Sets the verification token.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionConfig;
    /// assert_eq!(SubscriptionConfig::new().verify_token("t").verify_token.unwrap(), "t");
    /// ```
    #[must_use]
    pub fn verify_token(mut self, token: impl Into<String>) -> Self {
        self.verify_token = Some(token.into());
        self
    }

    /// Checks the callback URL is `https://` and the token is not empty.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `override_callback_uri` or `verify_token`.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionConfig;
    ///
    /// assert!(SubscriptionConfig::new().callback_url("https://a.test/h").validate().is_ok());
    /// assert!(SubscriptionConfig::new().callback_url("http://a.test/h").validate().is_err());
    /// assert!(SubscriptionConfig::new().verify_token("").validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        if let Some(url) = &self.callback_url {
            if !url.starts_with("https://") {
                return Err(Error::validation(
                    "override_callback_uri",
                    "must be an https:// URL",
                ));
            }
            let chars = url.chars().count();
            if chars > 1024 {
                return Err(Error::validation(
                    "override_callback_uri",
                    format!("must be at most 1024 characters, got {chars}"),
                ));
            }
        }
        if let Some(token) = &self.verify_token {
            if token.is_empty() {
                return Err(Error::validation(
                    "verify_token",
                    "must not be empty - Meta echoes it back on every handshake",
                ));
            }
            let chars = token.chars().count();
            if chars > 256 {
                return Err(Error::validation(
                    "verify_token",
                    format!("must be at most 256 characters, got {chars}"),
                ));
            }
        }
        Ok(())
    }

    /// Validates and renders the subscription body.
    ///
    /// # Errors
    ///
    /// Whatever [`SubscriptionConfig::validate`] rejects.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionConfig;
    ///
    /// assert_eq!(
    ///     SubscriptionConfig::new().verify_token("t").to_json().unwrap()["verify_token"],
    ///     "t"
    /// );
    /// ```
    pub fn to_json(&self) -> Result<Value> {
        self.validate()?;
        let mut map = Map::new();
        if let Some(url) = &self.callback_url {
            map.insert("override_callback_uri".into(), Value::String(url.clone()));
        }
        if let Some(token) = &self.verify_token {
            map.insert("verify_token".into(), Value::String(token.clone()));
        }
        Ok(Value::Object(map))
    }
}

/// The outcome of [`crate::WhatsAppClient::ensure_webhook_subscription`].
///
/// ```
/// use meta_whatsapp::subscription::SubscriptionState;
///
/// assert!(SubscriptionState::AlreadySubscribed.is_subscribed());
/// assert!(SubscriptionState::Subscribed.is_subscribed());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SubscriptionState {
    /// The app was already subscribed; no write was made.
    AlreadySubscribed,
    /// The app was missing and has just been subscribed.
    Subscribed,
}

impl SubscriptionState {
    /// `true` for both states — the app is subscribed afterwards.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionState;
    /// assert!(SubscriptionState::Subscribed.is_subscribed());
    /// ```
    #[must_use]
    pub fn is_subscribed(self) -> bool {
        matches!(self, Self::AlreadySubscribed | Self::Subscribed)
    }

    /// Whether this call had to write to the API.
    ///
    /// ```
    /// use meta_whatsapp::subscription::SubscriptionState;
    ///
    /// assert!(SubscriptionState::Subscribed.was_written());
    /// assert!(!SubscriptionState::AlreadySubscribed.was_written());
    /// ```
    #[must_use]
    pub fn was_written(self) -> bool {
        matches!(self, Self::Subscribed)
    }
}

/// Validates a WhatsApp Business Account id before it becomes a URL segment.
///
/// ```
/// use meta_whatsapp::subscription::validate_waba_id;
///
/// assert!(validate_waba_id("102290129340398").is_ok());
/// assert!(validate_waba_id("").is_err());
/// assert!(validate_waba_id("../me").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `waba_id` when it is empty, longer than 32
/// characters, or not digits only.
pub fn validate_waba_id(waba_id: &str) -> Result<()> {
    if waba_id.is_empty() {
        return Err(Error::validation("waba_id", "must not be empty"));
    }
    if waba_id.len() > 32 || !waba_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::validation(
            "waba_id",
            "must be digits only (at most 32)",
        ));
    }
    Ok(())
}

/// Validates an app id before it becomes a URL segment.
///
/// App-level subscription calls accept at most 32 ASCII digits. This mirrors
/// [`validate_waba_id`] but keeps the error's field name distinct.
///
/// ```
/// use meta_whatsapp::subscription::validate_app_id;
///
/// assert!(validate_app_id("1234567890").is_ok());
/// assert!(validate_app_id("").is_err());
/// assert!(validate_app_id("../me").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `app_id` when it is empty, longer than 32
/// characters, or not ASCII digits only.
pub fn validate_app_id(app_id: &str) -> Result<()> {
    if app_id.is_empty() {
        return Err(Error::validation("app_id", "must not be empty"));
    }
    if app_id.len() > 32 || !app_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::validation(
            "app_id",
            "must be digits only (at most 32)",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_documented_shapes_parse() {
        let wrapped = serde_json::json!({
            "data": [{"whatsapp_business_api_data": {"id": "1", "name": "Bot", "link": "https://x.test"}}]
        });
        let flat = serde_json::json!({"data": [{"id": "2", "name": "Bot"}]});
        assert!(WebhookSubscriptions::parse(&wrapped).unwrap().has_app("1"));
        assert!(WebhookSubscriptions::parse(&flat).unwrap().has_app("2"));
    }

    #[test]
    fn a_missing_data_key_is_an_empty_list() {
        let subs = WebhookSubscriptions::parse(&serde_json::json!({"success": true})).unwrap();
        assert!(subs.is_empty());
    }

    #[test]
    fn app_ids_are_compared_exactly() {
        let subs =
            WebhookSubscriptions::parse(&serde_json::json!({"data": [{"id": "123"}]})).unwrap();
        assert!(subs.has_app("123"));
        assert!(!subs.has_app("0123"));
        assert!(!subs.has_app(""));
    }

    #[test]
    fn empty_data_entries_are_tolerated() {
        let subs = WebhookSubscriptions::parse(&serde_json::json!({"data": [{}]})).unwrap();
        assert_eq!(subs.data.len(), 1);
        assert_eq!(subs.data[0].id, None);
        assert!(!subs.has_app("1"));
    }

    #[test]
    fn config_limits_are_enforced() {
        let long = format!("https://a.test/{}", "x".repeat(2048));
        assert!(SubscriptionConfig::new()
            .callback_url(long)
            .validate()
            .is_err());
        assert!(SubscriptionConfig::new()
            .verify_token("x".repeat(257))
            .validate()
            .is_err());
        assert!(SubscriptionConfig::new()
            .verify_token("x".repeat(256))
            .validate()
            .is_ok());
    }
}
