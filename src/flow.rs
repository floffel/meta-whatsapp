//! `WhatsApp` Flows: the typed inbound `nfm_reply` and the limits the outbound
//! flow message is checked against.
//!
//! A flow message leaves as an interactive `flow` message carrying a
//! `flow_token`; when the customer finishes, Meta posts an `nfm_reply` back
//! whose `response_json` is a *stringified* JSON document. The typed
//! [`crate::types::FlowReply`] in `types.rs` decodes that string through
//! [`crate::types::FlowResponse`]; this module owns the limits the outbound
//! message is checked against and the two validators for the `navigate` payload.
//!
//! This module is the round trip of issue #8. The rest of the Flows API lives in
//! its own issues: asset upload (#27), the data-exchange encryption endpoints
//! (#28) and preview/get (#29). Serving a flow's `data_channel_uri` is a server
//! concern; this crate carries the message out and the reply in.
//!
//! ```
//! use meta_whatsapp::types::{FlowReply, Message};
//!
//! let message: Message = serde_json::from_str(
//!     r#"{"id":"1","from":"2","type":"interactive","interactive":{"type":"nfm_reply",
//!         "nfm_reply":{"name":"flow","body":"Sent",
//!                      "response_json":"{\"flow_token\":\"order-42\"}"}}}"#,
//! ).unwrap();
//!
//! let reply: &FlowReply = message.flow_reply().unwrap();
//! assert_eq!(reply.name.as_deref(), Some("flow"));
//! assert_eq!(reply.response().unwrap().flow_token(), Some("order-42"));
//! ```

use serde_json::Value;

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
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_screen_name_of_64_characters_is_accepted_and_65_are_not() {
        assert!(validate_screen_name("WELCOME_SCREEN").is_ok());
        assert!(validate_screen_name("screen-2").is_ok());
        assert!(validate_screen_name(&"a".repeat(FLOW_SCREEN_MAX_CHARS)).is_ok());
        assert!(validate_screen_name(&"a".repeat(FLOW_SCREEN_MAX_CHARS + 1)).is_err());
        // Just outside on the other side of the range.
        assert!(validate_screen_name("").is_err());
        assert!(validate_screen_name(" ").is_err());
    }

    #[test]
    fn only_ascii_letters_digits_underscore_and_hyphen_pass_the_screen_name() {
        for bad in ["WELCOME SCREEN", "welcome/screen", "wélcome", "a\tb", "a.b"] {
            assert!(validate_screen_name(bad).is_err(), "{bad:?} was accepted");
        }
        assert!(validate_screen_name("A_b-9").is_ok());
    }

    #[test]
    fn screen_data_must_be_a_non_empty_object() {
        assert!(validate_screen_data(&json!({"name": "Ada"})).is_ok());
        assert!(validate_screen_data(&json!({})).is_err());
        for bad in [json!("Ada"), json!([1, 2]), json!(null), json!(7)] {
            assert!(validate_screen_data(&bad).is_err(), "{bad:?} was accepted");
        }
    }
}
