//! Webhook subscription management at the app and WhatsApp Business Account
//! levels.
//!
//! The WABA-level three Graph API calls sit behind this module:
//!
//! | Call | Endpoint |
//! | --- | --- |
//! | list | `GET /{waba-id}/subscribed_apps` |
//! | subscribe | `POST /{waba-id}/subscribed_apps` |
//! | unsubscribe | `DELETE /{waba-id}/subscribed_apps` |
//!
//! The app-level endpoint is a separate subscription. It is the one that picks
//! the webhook fields (`messages`, `message_template_status_update`, …) and
//! carries the callback URL and verification token:
//!
//! | Call | Endpoint |
//! | --- | --- |
//! | list | `GET /{app-id}/subscriptions` |
//! | subscribe | `POST /{app-id}/subscriptions` |
//! | unsubscribe | `DELETE /{app-id}/subscriptions?object=whatsapp_business_account` |
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

/// Graph object subscribed to WhatsApp Business Account webhooks.
///
/// ```
/// assert_eq!(
///     meta_whatsapp::subscription::WHATSAPP_WEBHOOK_OBJECT,
///     "whatsapp_business_account"
/// );
/// ```
pub const WHATSAPP_WEBHOOK_OBJECT: &str = "whatsapp_business_account";

/// Maximum number of digits in an app id used as a URL segment.
///
/// ```
/// assert_eq!(meta_whatsapp::subscription::APP_ID_MAX_DIGITS, 32);
/// ```
pub const APP_ID_MAX_DIGITS: usize = 32;

/// Maximum characters in a webhook callback URL.
///
/// ```
/// assert_eq!(
///     meta_whatsapp::subscription::WEBHOOK_CALLBACK_URL_MAX_CHARS,
///     1024
/// );
/// ```
pub const WEBHOOK_CALLBACK_URL_MAX_CHARS: usize = 1024;

/// Maximum characters in a webhook verification token.
///
/// ```
/// assert_eq!(
///     meta_whatsapp::subscription::WEBHOOK_VERIFY_TOKEN_MAX_CHARS,
///     256
/// );
/// ```
pub const WEBHOOK_VERIFY_TOKEN_MAX_CHARS: usize = 256;

/// Minimum number of fields an app-level subscription must request.
///
/// A subscription with no fields receives nothing, so an empty list is
/// rejected before the request.
///
/// ```
/// assert_eq!(meta_whatsapp::subscription::WEBHOOK_FIELDS_MIN, 1);
/// ```
pub const WEBHOOK_FIELDS_MIN: usize = 1;

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
