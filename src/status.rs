//! Delivery status of a message you sent — the Cloud API *Read a Message*
//! endpoint.
//!
//! `GET /{phone-number-id}?id={wamid}&messaging_product=whatsapp` answers the
//! current state of one outbound message (`sent`, `delivered`, `read`,
//! `failed`), which is the pull-side complement of the `statuses` webhook push
//! ([`crate::types::Status`]): useful when a webhook was lost, and for services
//! that do not expose a callback at all.
//!
//! Decoding is permissive (§7): unknown fields are ignored, missing optionals
//! default, and an unknown status string is carried through as
//! [`DeliveryStatus::Unknown`] instead of failing the response — a status Meta
//! adds later cannot break a deployment.
//!
//! ```
//! use meta_whatsapp::status::{DeliveryStatus, MessageStatus};
//!
//! let status = MessageStatus::parse(&serde_json::json!({
//!     "messaging_product": "whatsapp",
//!     "messages": [{
//!         "id": "wamid.HBgLMTU3MjkwMDg0NTU5NR",
//!         "status": "read",
//!         "timestamp": "1626323163",
//!         "recipient_id": "4915112345678",
//!     }],
//! }))
//! .unwrap();
//!
//! assert_eq!(status.status, DeliveryStatus::Read);
//! assert_eq!(status.timestamp_secs(), Some(1_626_323_163));
//! assert!(status.errors.is_empty());
//! ```

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::StatusError;

/// Upper bound for a message id (wamid) accepted by
/// [`crate::WhatsAppClient::message_status`].
///
/// Real wamids are a `wamid.` prefix plus a base64 payload and land around
/// 100–200 bytes; 256 is comfortably above that and still bounds what a
/// caller can put into the query string.
///
/// ```
/// use meta_whatsapp::status::{validate_wamid, MAX_WAMID_LEN};
///
/// assert_eq!(MAX_WAMID_LEN, 256);
/// assert!(validate_wamid(&"a".repeat(MAX_WAMID_LEN)).is_ok());
/// assert!(validate_wamid(&"a".repeat(MAX_WAMID_LEN + 1)).is_err());
/// ```
pub const MAX_WAMID_LEN: usize = 256;

/// Checks the shape of a wamid *before* it is interpolated into a query value.
///
/// Accepted: at least one and at most [`MAX_WAMID_LEN`] bytes of
/// `A-Z a-z 0-9 - _ . : + / =` — the alphabet real wamids use. Everything else
/// is rejected, which is how an empty id, whitespace, NUL, control characters
/// and the query metacharacters (`?`, `&`, `#`, `%`, `@`, ` `) never reach the
/// URL.
///
/// A `.` is only legal as the `wamid.` separator: never leading, trailing or
/// doubled. That keeps path shapes such as `../etc/passwd` out — the id travels
/// as a query value, but a validator that mirrors the shape of a real id is
/// harder to fool than one that only checks a character set.
///
/// ```
/// use meta_whatsapp::status::validate_wamid;
///
/// assert!(validate_wamid("wamid.HBgLMTU3MjkwMDg0NTU5NRMVAgEYGBQyQTc=").is_ok());
/// assert!(validate_wamid("").is_err(), "nothing to ask about");
/// assert!(validate_wamid("wamid. 1").is_err(), "whitespace");
/// assert!(validate_wamid("wamid.\0").is_err(), "NUL");
/// assert!(validate_wamid("wamid.1?x=2&id=3").is_err(), "query smuggling");
/// assert!(validate_wamid("../etc/passwd").is_err(), "path shape");
/// assert!(validate_wamid("a..b").is_err(), "a doubled separator");
/// ```
///
/// # Errors
///
/// [`Error::Validation`] when the id is empty, longer than
/// [`MAX_WAMID_LEN`], or contains anything but `A-Z a-z 0-9 - _ . : + / =` —
/// which covers whitespace, NUL, control bytes and the query metacharacters.
/// A `.` that leads, trails or repeats the id is refused the same way.
pub fn validate_wamid(wamid: &str) -> Result<()> {
    if wamid.is_empty() {
        return Err(Error::validation("wamid", "must not be empty"));
    }
    if wamid.len() > MAX_WAMID_LEN {
        return Err(Error::validation(
            "wamid",
            format!("must be at most {MAX_WAMID_LEN} bytes, got {}", wamid.len()),
        ));
    }
    if !wamid.bytes().all(is_wamid_byte) {
        return Err(Error::validation(
            "wamid",
            "only ASCII alphanumerics and '-', '_', '.', ':', '+', '/' and '=' are allowed",
        ));
    }
    if !is_wamid_shape(wamid) {
        return Err(Error::validation(
            "wamid",
            "'.' must separate the id, not lead it, trail it or repeat",
        ));
    }
    Ok(())
}

/// The alphabet of a wamid: base64 payload plus the `wamid.` separator. Note
/// what is *not* here: whitespace, control bytes and every character that
/// could end the query value or start a new parameter.
fn is_wamid_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'+' | b'/' | b'=')
}

/// The shape of a wamid. Base64 carries no dot, so a genuine id holds at most
/// the one `wamid.` separator; anything else is a path or a relative reference.
fn is_wamid_shape(wamid: &str) -> bool {
    !wamid.starts_with('.') && !wamid.ends_with('.') && !wamid.contains("..")
}

/// Delivery state of a sent message.
///
/// ```
/// use meta_whatsapp::status::DeliveryStatus;
///
/// assert_eq!(DeliveryStatus::parse("read"), DeliveryStatus::Read);
/// assert_eq!(DeliveryStatus::parse("Delivered"), DeliveryStatus::Delivered);
/// assert_eq!(
///     DeliveryStatus::parse("played"),
///     DeliveryStatus::Unknown("played".into())
/// );
/// assert_eq!(DeliveryStatus::Failed.as_str(), "failed");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeliveryStatus {
    /// WhatsApp accepted the message and handed it to the network.
    Sent,
    /// It reached the recipient's device.
    Delivered,
    /// The recipient opened it (the blue ticks).
    Read,
    /// It was not delivered; the reason is in [`MessageStatus::errors`].
    Failed,
    /// A status this crate does not know; the raw string is kept.
    Unknown(String),
}

impl Default for DeliveryStatus {
    /// The status of a response that carried none: [`DeliveryStatus::Unknown`]
    /// holding an empty string.
    ///
    /// ```
    /// use meta_whatsapp::status::DeliveryStatus;
    ///
    /// assert_eq!(DeliveryStatus::default(), DeliveryStatus::Unknown(String::new()));
    /// ```
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl DeliveryStatus {
    /// Parses a status string, case-insensitively and without failing on values
    /// this crate does not know.
    ///
    /// ```
    /// use meta_whatsapp::status::DeliveryStatus;
    ///
    /// assert_eq!(DeliveryStatus::parse(" sent "), DeliveryStatus::Sent);
    /// assert_eq!(DeliveryStatus::parse("READ"), DeliveryStatus::Read);
    /// assert_eq!(
    ///     DeliveryStatus::parse("deleted"),
    ///     DeliveryStatus::Unknown("deleted".into())
    /// );
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "sent" => Self::Sent,
            "delivered" => Self::Delivered,
            "read" => Self::Read,
            "failed" => Self::Failed,
            _ => Self::Unknown(raw.trim().to_string()),
        }
    }

    /// The wire spelling: `sent`, `delivered`, `read`, `failed`, or the raw
    /// string for an unknown status.
    ///
    /// ```
    /// use meta_whatsapp::status::DeliveryStatus;
    ///
    /// assert_eq!(DeliveryStatus::Sent.as_str(), "sent");
    /// assert_eq!(DeliveryStatus::Unknown("played".into()).as_str(), "played");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
            Self::Unknown(raw) => raw,
        }
    }
}

impl Serialize for DeliveryStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DeliveryStatus {
    /// Accepts any JSON value: a string is parsed, `null` becomes the default
    /// and anything else is kept as the raw JSON text — decoding a status never
    /// fails on a value Meta invented.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        Ok(match Value::deserialize(deserializer)? {
            Value::String(text) => Self::parse(&text),
            Value::Null => Self::default(),
            other => Self::Unknown(other.to_string()),
        })
    }
}

/// One entry of the *Read a Message* response: what happened to a message we
/// sent.
///
/// Unknown fields are ignored, missing ones default, so a response Meta later
/// extends still decodes.
///
/// ```
/// use meta_whatsapp::status::{DeliveryStatus, MessageStatus};
///
/// let status: MessageStatus = serde_json::from_str(
///     r#"{"id":"wamid.HBgLMTU=","status":"failed","timestamp":"1729600000",
///         "recipient_id":"4915112345678","errors":[{"code":131026,"title":"Message undeliverable"}],
///         "some_new_field":{"a":1}}"#,
/// )
/// .unwrap();
///
/// assert_eq!(status.status, DeliveryStatus::Failed);
/// assert_eq!(status.errors[0].code, 131_026);
/// assert_eq!(status.errors[0].title, "Message undeliverable");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MessageStatus {
    /// The message id this entry describes (the wamid you asked about).
    #[serde(default)]
    pub id: String,
    /// `sent` / `delivered` / `read` / `failed`, or the raw string when Meta
    /// sends something else.
    #[serde(default)]
    pub status: DeliveryStatus,
    /// Unix timestamp as a string.
    #[serde(default)]
    pub timestamp: String,
    /// Recipient WhatsApp id.
    #[serde(default)]
    pub recipient_id: String,
    /// Failure details; empty unless the message failed.
    #[serde(default)]
    pub errors: Vec<StatusError>,
}

impl MessageStatus {
    /// Reads every entry of a *Read a Message* body.
    ///
    /// Accepts the documented envelope (`{"messages": […]}`), a bare message
    /// object (what a proxy in front of the Graph API tends to hand back) and a
    /// JSON array of entries. Anything that cannot carry a status — an
    /// envelope with neither `messages` nor `id`/`status` — is
    /// [`Error::Payload`].
    ///
    /// ```
    /// use meta_whatsapp::status::MessageStatus;
    ///
    /// let entries = MessageStatus::parse_all(&serde_json::json!({
    ///     "messages": [{"id": "a", "status": "sent"}, {"id": "b", "status": "delivered"}],
    /// }))
    /// .unwrap();
    /// assert_eq!(entries.len(), 2);
    ///
    /// // A single message may arrive on its own.
    /// let bare = MessageStatus::parse_all(&serde_json::json!({"id": "c", "status": "read"})).unwrap();
    /// assert_eq!(bare.len(), 1);
    ///
    /// // An answer that carries nothing decodes as no entries at all.
    /// assert!(MessageStatus::parse_all(&serde_json::json!({"messages": []})).unwrap().is_empty());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the body cannot carry a status at all (no
    /// `messages` array and no `id`/`status`), when `messages` is not an
    /// array. An entry that is not an object, or a field of a type that cannot
    /// be read (`"errors": "x"`, a string `code`), is [`Error::Json`] — the
    /// entry is malformed, not merely unfamiliar. A missing or unknown status
    /// is *neither*: it decodes as [`DeliveryStatus::Unknown`].
    pub fn parse_all(response: &Value) -> Result<Vec<Self>> {
        match response {
            Value::Object(map) => match map.get("messages") {
                Some(Value::Array(entries)) => entries.iter().map(Self::from_value).collect(),
                Some(other) => Err(Error::Payload(format!(
                    "`messages` is not an array: {other}"
                ))),
                None if map.contains_key("id") || map.contains_key("status") => {
                    Ok(vec![Self::from_value(response)?])
                }
                None => Err(Error::Payload(format!(
                    "response carries no message status: {response}"
                ))),
            },
            Value::Array(entries) => entries.iter().map(Self::from_value).collect(),
            other => Err(Error::Payload(format!(
                "expected a message status object, got: {other}"
            ))),
        }
    }

    /// The entry for the id you asked about — the first one the endpoint
    /// returned.
    ///
    /// ```
    /// use meta_whatsapp::status::{DeliveryStatus, MessageStatus};
    ///
    /// let status = MessageStatus::parse(&serde_json::json!(
    ///     {"messages": [{"id": "wamid.1", "status": "delivered", "recipient_id": "4915"}]}
    /// ))
    /// .unwrap();
    /// assert_eq!(status.status, DeliveryStatus::Delivered);
    ///
    /// // Nothing to report is an error, not an empty status.
    /// assert!(MessageStatus::parse(&serde_json::json!({"messages": []})).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] or [`Error::Json`] as for [`MessageStatus::parse_all`],
    /// plus [`Error::Payload`] when the body decodes to zero entries.
    pub fn parse(response: &Value) -> Result<Self> {
        Self::parse_all(response)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::Payload("the response carried no `messages` entries".to_string()))
    }

    /// `timestamp` parsed as unix seconds.
    ///
    /// ```
    /// use meta_whatsapp::status::MessageStatus;
    ///
    /// let status: MessageStatus =
    ///     serde_json::from_str(r#"{"id":"1","status":"sent","timestamp":"1626323163"}"#).unwrap();
    /// assert_eq!(status.timestamp_secs(), Some(1_626_323_163));
    /// assert_eq!(MessageStatus::default().timestamp_secs(), None);
    /// ```
    #[must_use]
    pub fn timestamp_secs(&self) -> Option<u64> {
        self.timestamp.trim().parse().ok()
    }

    /// The first [`crate::types::StatusError`], as a convenience for logging a
    /// failure without indexing.
    ///
    /// ```
    /// use meta_whatsapp::status::MessageStatus;
    ///
    /// let status: MessageStatus = serde_json::from_str(
    ///     r#"{"status":"failed","errors":[{"code":131047,"title":"Re-engagement message"}]}"#,
    /// )
    /// .unwrap();
    /// assert_eq!(status.first_error().map(|error| error.code), Some(131_047));
    /// assert_eq!(MessageStatus::default().first_error(), None);
    /// ```
    #[must_use]
    pub fn first_error(&self) -> Option<&StatusError> {
        self.errors.first()
    }

    fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wamid_is_bounded_at_its_documented_edge() {
        assert!(validate_wamid(&"a".repeat(MAX_WAMID_LEN)).is_ok());
        assert!(validate_wamid(&"a".repeat(MAX_WAMID_LEN + 1)).is_err());
    }

    #[test]
    fn the_real_alphabet_passes_and_the_dangerous_one_does_not() {
        assert!(
            validate_wamid("wamid.HBgLMTU3MjkwMDg0NTU5NRMVAgASGBQkQ0E4RjI0RjUyNTk4N0E4Qg==")
                .is_ok()
        );
        assert!(validate_wamid("wamid:1-2_3.4+5/6=").is_ok());
        for rejected in [
            "",
            " ",
            " ",
            "\t",
            "\n",
            "\0",
            "wamid.1 ",
            "w?amid",
            "wamid&x",
            "w#1",
            "w%41",
            "w@1",
            "w(1)",
            "wämid",
            "' OR 1=1",
            "../etc/passwd",
            "./x",
            "a..b",
            ".hidden",
            "trailing.",
        ] {
            assert!(validate_wamid(rejected).is_err(), "accepted: {rejected:?}");
        }
    }

    #[test]
    fn an_empty_wamid_names_its_field() {
        let error = validate_wamid("").unwrap_err();
        assert!(
            matches!(&error, Error::Validation { field, .. } if *field == "wamid"),
            "{error:?}"
        );
    }

    #[test]
    fn status_parsing_is_case_insensitive_and_keeps_the_unknown() {
        assert_eq!(DeliveryStatus::parse("Sent"), DeliveryStatus::Sent);
        assert_eq!(
            DeliveryStatus::parse(" delivered "),
            DeliveryStatus::Delivered
        );
        assert_eq!(DeliveryStatus::parse("failed"), DeliveryStatus::Failed);
        assert_eq!(
            DeliveryStatus::parse("played"),
            DeliveryStatus::Unknown("played".into())
        );
        assert_eq!(
            DeliveryStatus::parse("  spaced  "),
            DeliveryStatus::Unknown("spaced".into())
        );
    }

    #[test]
    fn a_non_string_status_is_carried_instead_of_failing() {
        for raw in ["7", "true", "null", r"[1]"] {
            let parsed: DeliveryStatus =
                serde_json::from_str(raw).expect("decoding a status never fails");
            let _ = parsed.as_str();
        }
    }

    #[test]
    fn a_bare_object_needs_to_look_like_a_status() {
        assert!(MessageStatus::parse_all(&serde_json::json!({"status": "sent"})).is_ok());
        assert!(MessageStatus::parse_all(&serde_json::json!({"id": "1"})).is_ok());
        assert!(MessageStatus::parse_all(&serde_json::json!({"success": true})).is_err());
        assert!(MessageStatus::parse_all(&serde_json::json!("nope")).is_err());
        assert!(
            MessageStatus::parse_all(&serde_json::json!({"messages": "wamid.1"})).is_err(),
            "a string where an array is documented"
        );
    }
}
