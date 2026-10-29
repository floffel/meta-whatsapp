//! `WhatsApp` Flows: the typed inbound `nfm_reply` and the limits the outbound
//! flow message is checked against.
//!
//! A flow message leaves as an interactive `flow` message carrying a
//! `flow_token`; when the customer finishes, Meta posts an `nfm_reply` back
//! whose `response_json` is a *stringified* JSON document. [`FlowReply`] decodes
//! that string once, keeps the wire object in [`FlowReply::raw`], and exposes the
//! keys a flow usually returns — `screen`, `data`, `flow_token` — without a
//! second `serde_json::from_str` at every call site.
//!
//! This module is the round trip of issue #8. The rest of the Flows API lives in
//! its own issues: asset upload (#27), the data-exchange encryption endpoints
//! (#28) and preview/get (#29). Serving a flow's `data_channel_uri` is a server
//! concern; this crate carries the message out and the reply in.
//!
//! ```
//! use meta_whatsapp::FlowReply;
//!
//! let reply = FlowReply::from_value(serde_json::json!({
//!     "name": "flow",
//!     "body": "Sent",
//!     "response_json": "{\"flow_token\":\"order-42\",\"screen\":\"SUMMARY\",\"data\":{\"name\":\"Ada\"}}",
//! }));
//!
//! assert_eq!(reply.flow_token.as_deref(), Some("order-42"));
//! assert_eq!(reply.screen(), Some("SUMMARY"));
//! assert_eq!(reply.data().and_then(|data| data.get("name")), Some(&serde_json::json!("Ada")));
//! // The string Meta sent is still there, untouched.
//! assert!(reply.raw["response_json"].is_string());
//! ```

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::{Error, Result};

/// The `flow_message_version` this crate sends by default: the only value Meta
/// documents (`"3"`).
///
/// ```
/// use meta_whatsapp::flow::FLOW_MESSAGE_VERSION;
/// assert_eq!(FLOW_MESSAGE_VERSION, "3");
/// ```
pub const FLOW_MESSAGE_VERSION: &str = "3";

/// Longest `flow_token` accepted, in UTF-8 bytes.
///
/// Meta documents no maximum — the token is opaque to it and is echoed back in
/// `nfm_reply` — so this is a crate-side bound, not a provider limit: it keeps a
/// request from carrying an unbounded string. See `docs/contracts.md`.
///
/// ```
/// use meta_whatsapp::flow::FLOW_TOKEN_MAX_BYTES;
/// assert_eq!(FLOW_TOKEN_MAX_BYTES, 1024);
/// ```
pub const FLOW_TOKEN_MAX_BYTES: usize = 1024;

/// Longest `flow_cta` accepted, in *characters* (the OpenAPI schema's
/// `maxLength: 30`).
///
/// Characters, not bytes: a CTA of 30 umlauts is 60 bytes and is still accepted.
///
/// ```
/// use meta_whatsapp::flow::FLOW_CTA_MAX_CHARS;
/// assert_eq!(FLOW_CTA_MAX_CHARS, 30);
/// ```
pub const FLOW_CTA_MAX_CHARS: usize = 30;

/// Longest screen id accepted, in characters.
///
/// Meta documents no maximum for a screen id (only that `SUCCESS` is reserved in
/// Flow JSON), so this is a crate-side bound like [`FLOW_TOKEN_MAX_BYTES`].
///
/// ```
/// use meta_whatsapp::flow::FLOW_SCREEN_MAX_CHARS;
/// assert_eq!(FLOW_SCREEN_MAX_CHARS, 64);
/// ```
pub const FLOW_SCREEN_MAX_CHARS: usize = 64;

/// Field path reported when a screen id is refused.
const SCREEN_FIELD: &str = "interactive.action.parameters.flow_action_payload.screen";

/// Field path reported when initial screen data is refused.
const DATA_FIELD: &str = "interactive.action.parameters.flow_action_payload.data";

/// Checks the shape of a screen id before it reaches the wire.
///
/// A screen id is the page identifier inside the flow, so only ASCII letters,
/// digits, `_` and `-` are accepted, up to [`FLOW_SCREEN_MAX_CHARS`] characters.
/// Whether a *well formed* id actually exists in the flow is Meta's call: this
/// check stops the shapes that can never be a screen, not the ones that are
/// merely unknown to us.
///
/// ```
/// use meta_whatsapp::flow::validate_screen_name;
///
/// assert!(validate_screen_name("WELCOME_SCREEN").is_ok());
/// assert!(validate_screen_name("screen-2").is_ok());
/// assert!(validate_screen_name("WELCOME SCREEN").is_err());
/// assert!(validate_screen_name("").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `interactive.action.parameters.flow_action_payload.screen`.
pub fn validate_screen_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(Error::validation(SCREEN_FIELD, "must not be empty"));
    }
    let count = name.chars().count();
    if count > FLOW_SCREEN_MAX_CHARS {
        return Err(Error::validation(
            SCREEN_FIELD,
            format!("must be at most {FLOW_SCREEN_MAX_CHARS} characters, got {count}"),
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(Error::validation(
            SCREEN_FIELD,
            "must contain only ASCII letters, digits, '_' and '-'",
        ));
    }
    Ok(())
}

/// Checks the initial screen data of a `navigate` action.
///
/// The schema asks for a non-empty object; `data` on the wire is that object.
///
/// ```
/// use meta_whatsapp::flow::validate_screen_data;
///
/// assert!(validate_screen_data(&serde_json::json!({"name": "Ada"})).is_ok());
/// assert!(validate_screen_data(&serde_json::json!({})).is_err());
/// assert!(validate_screen_data(&serde_json::json!("Ada")).is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `interactive.action.parameters.flow_action_payload.data`.
pub fn validate_screen_data(data: &Value) -> Result<()> {
    match data {
        Value::Object(map) if map.is_empty() => Err(Error::validation(
            DATA_FIELD,
            "must be a non-empty JSON object",
        )),
        Value::Object(_) => Ok(()),
        _ => Err(Error::validation(DATA_FIELD, "must be a JSON object")),
    }
}

/// Typed view of the `interactive.nfm_reply` object a finished flow sends back.
///
/// Meta sends `response_json` as a string; here it is already parsed. Two shapes
/// parse to the same type, because Meta's own examples disagree: `response_json`
/// as the documented string, and as an object (accepted rather than rejected).
/// A string that is not JSON is kept verbatim in [`FlowReply::response_json`]
/// instead of being dropped, and the whole wire object always stays available in
/// [`FlowReply::raw`].
///
/// ```
/// use meta_whatsapp::FlowReply;
///
/// let reply: FlowReply = serde_json::from_str(
///     r#"{"name":"flow","body":"Sent","response_json":"{\"screen\":\"SUMMARY\",\"data\":{\"ok\":true}}"}"#,
/// )?;
/// assert_eq!(reply.flow_name.as_deref(), Some("flow"));
/// assert_eq!(reply.screen(), Some("SUMMARY"));
/// assert_eq!(reply.data().and_then(|data| data.get("ok")), Some(&serde_json::json!(true)));
/// assert!(reply.flow_token.is_none(), "this flow echoed no token");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowReply {
    /// The `flow_token` the flow echoed back, if any.
    ///
    /// Read from the reply object when it carries one, otherwise from the
    /// response document — that is where Meta puts it, and an endpoint-powered
    /// flow may also return it at the top level.
    pub flow_token: Option<String>,
    /// `nfm_reply.name` — `flow` in every documented example.
    pub flow_name: Option<String>,
    /// The decoded response document, or [`Value::Null`] when Meta sent none.
    ///
    /// When `response_json` was a string that is not JSON, this holds that
    /// string unchanged: a malformed response is worth inspecting, not worth
    /// failing the whole webhook over.
    pub response_json: Value,
    /// The `nfm_reply` object exactly as it arrived, for keys this type does not
    /// name (`body`, or anything Meta adds later).
    pub raw: Value,
}

impl FlowReply {
    /// Builds a reply from an already parsed JSON value, without ever failing.
    ///
    /// ```
    /// use meta_whatsapp::FlowReply;
    ///
    /// let reply = FlowReply::from_value(serde_json::json!({
    ///     "flow_token": "t-1",
    ///     "response_json": {"screen": "DONE"},
    /// }));
    /// assert_eq!(reply.flow_token.as_deref(), Some("t-1"));
    /// assert_eq!(reply.screen(), Some("DONE"));
    /// ```
    #[must_use]
    pub fn from_value(value: Value) -> Self {
        let mut reply = Self {
            raw: value,
            ..Self::default()
        };
        let Value::Object(map) = &reply.raw else {
            return reply;
        };

        reply.flow_name = map
            .get("name")
            .or_else(|| map.get("flow_name"))
            .and_then(Value::as_str)
            .map(str::to_string);
        reply.flow_token = map
            .get("flow_token")
            .and_then(Value::as_str)
            .map(str::to_string);
        reply.response_json = match map.get("response_json") {
            // The documented shape: a JSON document inside a string.
            Some(Value::String(text)) => {
                serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
            }
            // Tolerated shape: the document itself.
            Some(other) => other.clone(),
            None => Value::Null,
        };
        // The response document is where Meta puts the token; prefer it when the
        // reply object carries none of its own.
        if reply.flow_token.is_none() {
            reply.flow_token = reply
                .response_json
                .get("flow_token")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        reply
    }

    /// A key of the decoded response document, as raw JSON.
    ///
    /// ```
    /// use meta_whatsapp::FlowReply;
    ///
    /// let reply = FlowReply::from_value(serde_json::json!({
    ///     "response_json": "{\"total\":42}",
    /// }));
    /// assert_eq!(reply.get("total"), Some(&serde_json::json!(42)));
    /// assert_eq!(reply.get("missing"), None);
    /// ```
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.response_json.get(key)
    }

    /// A string-valued key of the decoded response document.
    ///
    /// ```
    /// use meta_whatsapp::FlowReply;
    ///
    /// let reply = FlowReply::from_value(serde_json::json!({
    ///     "response_json": "{\"coupon\":\"SUMMER\",\"count\":3}",
    /// }));
    /// assert_eq!(reply.string("coupon"), Some("SUMMER"));
    /// assert_eq!(reply.string("count"), None);
    /// ```
    #[must_use]
    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// The screen the flow finished on (`response_json.screen`).
    ///
    /// ```
    /// use meta_whatsapp::FlowReply;
    ///
    /// let reply = FlowReply::from_value(serde_json::json!({
    ///     "response_json": "{\"screen\":\"SUCCESS\"}",
    /// }));
    /// assert_eq!(reply.screen(), Some("SUCCESS"));
    /// ```
    #[must_use]
    pub fn screen(&self) -> Option<&str> {
        self.string("screen")
    }

    /// The collected form values (`response_json.data`), when the flow returned
    /// an object.
    ///
    /// ```
    /// use meta_whatsapp::FlowReply;
    ///
    /// let reply = FlowReply::from_value(serde_json::json!({
    ///     "response_json": "{\"screen\":\"SUMMARY\",\"data\":{\"size\":\"L\"}}",
    /// }));
    /// assert_eq!(reply.data().and_then(|data| data.get("size")), Some(&serde_json::json!("L")));
    /// ```
    #[must_use]
    pub fn data(&self) -> Option<&Map<String, Value>> {
        self.get("data").and_then(Value::as_object)
    }
}

impl<'de> Deserialize<'de> for FlowReply {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::from_value(Value::deserialize(deserializer)?))
    }
}

impl Serialize for FlowReply {
    /// Writes the wire object back verbatim.
    ///
    /// A parsed webhook therefore re-serialises to the shape Meta sent, with
    /// `response_json` still a string. Mutating the decoded view does not change
    /// the wire object — [`FlowReply::raw`] stays the source of truth.
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.raw.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_object_reply_keeps_its_raw_value() {
        let reply = FlowReply::from_value(Value::String("Sent".into()));
        assert_eq!(reply.response_json, Value::Null);
        assert_eq!(reply.raw, Value::String("Sent".into()));
        assert!(reply.flow_token.is_none());
        assert!(reply.data().is_none());
    }

    #[test]
    fn an_unparsable_response_string_is_kept_verbatim() {
        let reply = FlowReply::from_value(serde_json::json!({"response_json": "not json"}));
        assert_eq!(reply.response_json, Value::String("not json".into()));
        assert_eq!(reply.screen(), None);
    }

    #[test]
    fn deserialising_a_serialised_reply_is_stable() {
        let raw = serde_json::json!({
            "name": "flow",
            "body": "Sent",
            "response_json": "{\"screen\":\"DONE\",\"flow_token\":\"t-1\"}",
        });
        let reply: FlowReply = serde_json::from_value(raw.clone()).expect("a reply");
        assert_eq!(reply.flow_token.as_deref(), Some("t-1"));
        assert_eq!(serde_json::to_value(&reply).expect("serialisable"), raw);
    }
}
