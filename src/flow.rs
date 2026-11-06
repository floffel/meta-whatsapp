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
//! This module is the round trip of issue #8 plus the read-side Flow management
//! calls: [`FlowGetRequest`], [`FlowPreviewRequest`],
//! [`FlowEncryptionPublicKeyRequest`] and their typed responses. Meta's Flow
//! endpoint decrypts data-exchange requests locally, using the private key that
//! corresponds to [`FlowEncryptionPublicKeyResponse::business_public_key`]; it
//! publishes no Graph API operation that decrypts those payloads. Serving a
//! flow's `data_channel_uri` and performing that local decryption are server
//! concerns, while this crate carries the message out and the reply in.
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

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Number, Value};

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

/// Longest Flow id accepted, in UTF-8 bytes.
///
/// Meta's OpenAPI file types `Flow-ID` as an unconstrained string, so this is a
/// crate-side URL guard rather than a provider limit. It matches the other
/// path-id bounds in this crate while leaving ample room for the ids Meta
/// returns today.
///
/// ```
/// use meta_whatsapp::flow::FLOW_ID_MAX_BYTES;
/// assert_eq!(FLOW_ID_MAX_BYTES, 256);
/// ```
pub const FLOW_ID_MAX_BYTES: usize = 256;

/// The only filename Meta accepts for a Flow JSON asset.
///
/// ```
/// use meta_whatsapp::flow::FLOW_JSON_FILENAME;
/// assert_eq!(FLOW_JSON_FILENAME, "flow.json");
/// ```
pub const FLOW_JSON_FILENAME: &str = "flow.json";

/// The only Flow asset type this endpoint accepts.
///
/// ```
/// use meta_whatsapp::flow::FLOW_ASSET_TYPE;
/// assert_eq!(FLOW_ASSET_TYPE, "FLOW_JSON");
/// ```
pub const FLOW_ASSET_TYPE: &str = "FLOW_JSON";

/// Maximum size of a Flow JSON asset, in bytes (10 MiB).
///
/// ```
/// use meta_whatsapp::flow::FLOW_JSON_MAX_BYTES;
/// assert_eq!(FLOW_JSON_MAX_BYTES, 10 * 1024 * 1024);
/// ```
pub const FLOW_JSON_MAX_BYTES: usize = 10 * 1024 * 1024;

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

/// Checks a Flow id before it becomes a URL path segment.
///
/// Meta returns ids such as `flow-1` but publishes no grammar or maximum. The
/// accepted shape is therefore intentionally broader than current examples:
/// ASCII letters, digits, `-` and `_`, up to [`FLOW_ID_MAX_BYTES`] bytes. An
/// empty id, a path separator, query metacharacters and control characters are
/// refused before any request is built.
///
/// ```
/// use meta_whatsapp::flow::validate_flow_id;
///
/// assert!(validate_flow_id("flow-1").is_ok());
/// assert!(validate_flow_id("").is_err());
/// assert!(validate_flow_id("../me").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `flow_id` when it is empty, longer than
/// [`FLOW_ID_MAX_BYTES`], or contains a character outside ASCII alphanumerics,
/// `-` and `_`.
pub fn validate_flow_id(flow_id: &str) -> Result<()> {
    if flow_id.is_empty() {
        return Err(Error::validation("flow_id", "must not be empty"));
    }
    if flow_id.len() > FLOW_ID_MAX_BYTES {
        return Err(Error::validation(
            "flow_id",
            format!(
                "must be at most {FLOW_ID_MAX_BYTES} bytes, got {}",
                flow_id.len()
            ),
        ));
    }
    if !flow_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(Error::validation(
            "flow_id",
            "only ASCII alphanumerics, '-' and '_' are allowed",
        ));
    }
    Ok(())
}

/// A Flow JSON asset ready to upload to `POST /{flow_id}/assets`.
///
/// Construction validates the Flow id and payload size. The filename and
/// asset type are fixed by the API, so neither can be set to an unsupported
/// value.
///
/// ```
/// use meta_whatsapp::flow::FlowAssetUpload;
///
/// let upload = FlowAssetUpload::new("1234567890", br#"{"version":"5.0"}"#.to_vec())?;
/// assert_eq!(upload.flow_id(), "1234567890");
/// assert_eq!(upload.len(), 17);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowAssetUpload {
    flow_id: String,
    flow_json: Vec<u8>,
}

impl FlowAssetUpload {
    /// Validates a Flow id and JSON payload.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    ///
    /// assert!(FlowAssetUpload::new("flow-1", b"{}".to_vec()).is_ok());
    /// assert!(FlowAssetUpload::new("flow-1", Vec::new()).is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed Flow id or an empty/oversized
    /// payload. Meta's schema validation is left to the service, whose answer
    /// is represented by [`FlowAssetUploadResponse::validation_errors`].
    pub fn new(flow_id: impl Into<String>, flow_json: impl Into<Vec<u8>>) -> Result<Self> {
        let upload = Self {
            flow_id: flow_id.into(),
            flow_json: flow_json.into(),
        };
        upload.validate()?;
        Ok(upload)
    }

    /// The Flow that receives the asset.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    /// assert_eq!(FlowAssetUpload::new("42", b"{}".to_vec())?.flow_id(), "42");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn flow_id(&self) -> &str {
        &self.flow_id
    }

    /// The raw Flow JSON bytes.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    /// assert_eq!(FlowAssetUpload::new("42", b"{}".to_vec())?.flow_json(), b"{}");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn flow_json(&self) -> &[u8] {
        &self.flow_json
    }

    /// Length of the Flow JSON payload in bytes.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    /// assert_eq!(FlowAssetUpload::new("42", b"{}".to_vec())?.len(), 2);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.flow_json.len()
    }

    /// Whether the payload is empty. Always `false` after validation.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    /// assert!(!FlowAssetUpload::new("42", b"{}".to_vec())?.is_empty());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flow_json.is_empty()
    }

    /// Re-checks the invariants enforced by [`FlowAssetUpload::new`].
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUpload;
    /// assert!(FlowAssetUpload::new("42", b"{}".to_vec())?.validate().is_ok());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// See [`FlowAssetUpload::new`].
    pub fn validate(&self) -> Result<()> {
        validate_flow_id(&self.flow_id)?;
        if self.flow_json.is_empty() {
            return Err(Error::validation("flow_json", "must not be empty"));
        }
        if self.flow_json.len() > FLOW_JSON_MAX_BYTES {
            return Err(Error::validation(
                "flow_json",
                format!(
                    "{} bytes exceeds the {FLOW_JSON_MAX_BYTES} byte limit",
                    self.flow_json.len()
                ),
            ));
        }
        Ok(())
    }

    pub(crate) fn encode_multipart(&self) -> (String, Vec<u8>) {
        let boundary = crate::media::next_boundary(&self.flow_json);
        let mut body = Vec::with_capacity(self.flow_json.len() + 512);

        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"asset_type\"\r\n\r\n{FLOW_ASSET_TYPE}\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"name\"\r\n\r\n{FLOW_JSON_FILENAME}\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"file\"; filename=\"{FLOW_JSON_FILENAME}\"\r\nContent-Type: application/json\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&self.flow_json);
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

        (format!("multipart/form-data; boundary={boundary}"), body)
    }
}

/// Meta's answer to a Flow JSON asset upload.
///
/// The documented response is `{success, validation_errors}`, not an upload
/// handle or media id. `success` can be `true` while validation errors are
/// present, so call [`FlowAssetUploadResponse::is_valid`] when publishing.
///
/// ```
/// use meta_whatsapp::flow::FlowAssetUploadResponse;
///
/// let response = FlowAssetUploadResponse::from_value(&serde_json::json!({
///     "success": true,
///     "validation_errors": []
/// }))?;
/// assert!(response.success);
/// assert!(response.is_valid());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowAssetUploadResponse {
    /// Whether Meta accepted the upload request.
    #[serde(default)]
    pub success: bool,
    /// Diagnostics against the uploaded Flow JSON.
    #[serde(default)]
    pub validation_errors: Vec<FlowValidationError>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl FlowAssetUploadResponse {
    /// Decodes a 2xx response body.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUploadResponse;
    ///
    /// let response = FlowAssetUploadResponse::from_value(&serde_json::json!({
    ///     "success": true,
    ///     "future_field": 1
    /// }))?;
    /// assert_eq!(response.extra["future_field"], 1);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the body is malformed or not an object.
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }

    /// Whether the upload succeeded and produced no validation diagnostics.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowAssetUploadResponse;
    ///
    /// let clean = FlowAssetUploadResponse::from_value(&serde_json::json!({
    ///     "success": true,
    ///     "validation_errors": []
    /// }))?;
    /// assert!(clean.is_valid());
    /// let invalid = FlowAssetUploadResponse::from_value(&serde_json::json!({
    ///     "success": true,
    ///     "validation_errors": [{}]
    /// }))?;
    /// assert!(!invalid.is_valid());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.success && self.validation_errors.is_empty()
    }
}

/// Request for `GET /{Flow-ID}`.
///
/// An empty field list leaves Meta's default fields in place. Fields may use
/// the dotted and parenthesized syntax the Graph API documents for nested
/// selections, such as `health_status.phone_number(123)`.
///
/// ```
/// use meta_whatsapp::flow::FlowGetRequest;
///
/// let request = FlowGetRequest::new("flow-1")
///     .fields(["id", "name", "health_status.phone_number(123)"])
///     .date_format("U");
/// assert!(request.validate().is_ok());
/// assert_eq!(
///     request.query_string(),
///     "fields=id%2Cname%2Chealth_status.phone_number%28123%29&date_format=U"
/// );
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowGetRequest {
    /// Flow id to read.
    pub flow_id: String,
    /// Fields to request; empty means Meta's default field set.
    pub fields: Vec<String>,
    /// Optional date format, for example `U` for unix seconds.
    pub date_format: Option<String>,
}

impl FlowGetRequest {
    /// A request for one Flow with Meta's default fields.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowGetRequest;
    ///
    /// let request = FlowGetRequest::new("flow-1");
    /// assert_eq!(request.flow_id, "flow-1");
    /// assert!(request.fields.is_empty());
    /// ```
    pub fn new(flow_id: impl Into<String>) -> Self {
        Self {
            flow_id: flow_id.into(),
            ..Self::default()
        }
    }

    /// Sets the response fields, in wire order.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowGetRequest;
    ///
    /// let request = FlowGetRequest::new("flow-1").fields(["id", "name"]);
    /// assert_eq!(request.fields, ["id", "name"]);
    /// ```
    #[must_use]
    pub fn fields<I, S>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.fields = fields.into_iter().map(Into::into).collect();
        self
    }

    /// Sets the optional `date_format` query value.
    ///
    /// Meta's OpenAPI example uses `U`; it does not publish an exhaustive enum,
    /// so this remains a string instead of inventing one.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowGetRequest;
    ///
    /// assert_eq!(FlowGetRequest::new("flow-1").date_format("U").date_format.as_deref(), Some("U"));
    /// ```
    #[must_use]
    pub fn date_format(mut self, format: impl Into<String>) -> Self {
        self.date_format = Some(format.into());
        self
    }

    /// Validates the id, field expressions and date format before a URL is
    /// built.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowGetRequest;
    ///
    /// assert!(FlowGetRequest::new("flow-1").validate().is_ok());
    /// assert!(FlowGetRequest::new("flow/1").validate().is_err());
    /// assert!(FlowGetRequest::new("flow-1").fields(["id&limit=1"]).validate().is_err());
    /// assert!(FlowGetRequest::new("flow-1").date_format("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `flow_id`, `fields` or `date_format`.
    pub fn validate(&self) -> Result<()> {
        validate_flow_id(&self.flow_id)?;
        validate_flow_fields(&self.fields)?;
        validate_date_format(self.date_format.as_deref())
    }

    /// Renders the query string without the leading `?`.
    ///
    /// Values are percent-encoded. Call [`FlowGetRequest::validate`] first; the
    /// client always does.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowGetRequest;
    ///
    /// assert_eq!(FlowGetRequest::new("flow-1").query_string(), "");
    /// assert_eq!(FlowGetRequest::new("flow-1").fields(["id"]).query_string(), "fields=id");
    /// ```
    #[must_use]
    pub fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if !self.fields.is_empty() {
            parts.push(format!(
                "fields={}",
                encode_query_value(&self.fields.join(","))
            ));
        }
        if let Some(format) = &self.date_format {
            parts.push(format!("date_format={}", encode_query_value(format)));
        }
        parts.join("&")
    }
}

/// Request for a Flow web-preview URL.
///
/// This is the Flow `GET` endpoint with `fields=preview.invalidate(…)`.
/// `invalidate=false` reuses the current URL; `invalidate=true` creates a fresh
/// one and invalidates the previous link.
///
/// ```
/// use meta_whatsapp::flow::FlowPreviewRequest;
///
/// let request = FlowPreviewRequest::new("flow-1").invalidate(true);
/// assert_eq!(request.query_string(), "fields=preview.invalidate%28true%29");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowPreviewRequest {
    /// Flow id whose preview URL is requested.
    pub flow_id: String,
    /// Whether Meta should invalidate the previous URL and issue a new one.
    pub invalidate: bool,
    /// Optional date format, for example `U` for unix seconds.
    pub date_format: Option<String>,
}

impl FlowPreviewRequest {
    /// A non-invalidating preview request.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewRequest;
    ///
    /// let request = FlowPreviewRequest::new("flow-1");
    /// assert!(!request.invalidate);
    /// ```
    pub fn new(flow_id: impl Into<String>) -> Self {
        Self {
            flow_id: flow_id.into(),
            ..Self::default()
        }
    }

    /// Chooses whether the previous preview link is invalidated.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewRequest;
    ///
    /// assert!(FlowPreviewRequest::new("flow-1").invalidate(true).invalidate);
    /// ```
    #[must_use]
    pub fn invalidate(mut self, invalidate: bool) -> Self {
        self.invalidate = invalidate;
        self
    }

    /// Sets the optional `date_format` query value.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewRequest;
    ///
    /// assert_eq!(FlowPreviewRequest::new("flow-1").date_format("U").date_format.as_deref(), Some("U"));
    /// ```
    #[must_use]
    pub fn date_format(mut self, format: impl Into<String>) -> Self {
        self.date_format = Some(format.into());
        self
    }

    /// Validates the id and date format before the request reaches the wire.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewRequest;
    ///
    /// assert!(FlowPreviewRequest::new("flow-1").validate().is_ok());
    /// assert!(FlowPreviewRequest::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `flow_id` or `date_format`.
    pub fn validate(&self) -> Result<()> {
        validate_flow_id(&self.flow_id)?;
        validate_date_format(self.date_format.as_deref())
    }

    /// Renders the query string without the leading `?`.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewRequest;
    ///
    /// assert_eq!(
    ///     FlowPreviewRequest::new("flow-1").query_string(),
    ///     "fields=preview.invalidate%28false%29"
    /// );
    /// ```
    #[must_use]
    pub fn query_string(&self) -> String {
        let fields = format!("preview.invalidate({})", self.invalidate);
        let mut rendered = format!("fields={}", encode_query_value(&fields));
        if let Some(format) = &self.date_format {
            rendered.push_str("&date_format=");
            rendered.push_str(&encode_query_value(format));
        }
        rendered
    }
}

fn validate_flow_fields(fields: &[String]) -> Result<()> {
    for field in fields {
        if field.is_empty() {
            return Err(Error::validation("fields", "field names must not be empty"));
        }
        if !field.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'_' | b'.' | b',' | b'(' | b')')
        }) {
            return Err(Error::validation(
                "fields",
                "field expressions may contain only ASCII lowercase letters, digits, '_', '.', ',', '(' and ')'",
            ));
        }
    }
    Ok(())
}

fn validate_date_format(format: Option<&str>) -> Result<()> {
    if let Some(format) = format {
        if format.is_empty() {
            return Err(Error::validation("date_format", "must not be empty"));
        }
        if format.chars().any(char::is_control) {
            return Err(Error::validation(
                "date_format",
                "must not contain control characters",
            ));
        }
    }
    Ok(())
}

fn encode_query_value(value: &str) -> String {
    crate::template::encode_query_value(value)
}

/// A field that can be selected from the business-encryption endpoint.
///
/// Meta's OpenAPI description lists these two fields for
/// `GET /{Phone-Number-ID}/whatsapp_business_encryption`.
///
/// ```
/// use meta_whatsapp::flow::FlowEncryptionPublicKeyField;
///
/// assert_eq!(
///     FlowEncryptionPublicKeyField::BusinessPublicKey.as_str(),
///     "business_public_key"
/// );
/// assert_eq!(
///     FlowEncryptionPublicKeyField::SignatureStatus.as_str(),
///     "business_public_key_signature_status"
/// );
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FlowEncryptionPublicKeyField {
    /// The PEM-encoded 2048-bit RSA business public key.
    BusinessPublicKey,
    /// Whether Meta considers the stored public-key signature valid.
    SignatureStatus,
}

impl FlowEncryptionPublicKeyField {
    /// The field name as Meta spells it on the wire.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowEncryptionPublicKeyField;
    ///
    /// assert_eq!(
    ///     FlowEncryptionPublicKeyField::SignatureStatus.as_str(),
    ///     "business_public_key_signature_status"
    /// );
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BusinessPublicKey => "business_public_key",
            Self::SignatureStatus => "business_public_key_signature_status",
        }
    }
}

impl std::fmt::Display for FlowEncryptionPublicKeyField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for FlowEncryptionPublicKeyField {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FlowEncryptionPublicKeyField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        match String::deserialize(deserializer)?.as_str() {
            "business_public_key" => Ok(Self::BusinessPublicKey),
            "business_public_key_signature_status" => Ok(Self::SignatureStatus),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "business_public_key",
                    "business_public_key_signature_status",
                ],
            )),
        }
    }
}

/// Request for `GET /{Phone-Number-ID}/whatsapp_business_encryption`.
///
/// An empty [`FlowEncryptionPublicKeyRequest::fields`] list lets Meta return
/// its defaults. The phone-number id is checked before URL construction, so an
/// empty or path-like id cannot reach the transport.
///
/// ```
/// use meta_whatsapp::flow::{
///     FlowEncryptionPublicKeyField, FlowEncryptionPublicKeyRequest,
/// };
///
/// let request = FlowEncryptionPublicKeyRequest::new("106540352242922")
///     .fields([FlowEncryptionPublicKeyField::BusinessPublicKey]);
/// assert_eq!(
///     request.query_string(),
///     "fields=business_public_key"
/// );
/// assert!(request.validate().is_ok());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowEncryptionPublicKeyRequest {
    /// WhatsApp Business phone-number id whose configured key is requested.
    pub phone_number_id: String,
    /// Fields to request; empty means Meta's default field set.
    pub fields: Vec<FlowEncryptionPublicKeyField>,
}

impl FlowEncryptionPublicKeyRequest {
    /// A request for the bound number with Meta's default fields.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowEncryptionPublicKeyRequest;
    ///
    /// let request = FlowEncryptionPublicKeyRequest::new("1234");
    /// assert_eq!(request.phone_number_id, "1234");
    /// assert!(request.fields.is_empty());
    /// ```
    pub fn new(phone_number_id: impl Into<String>) -> Self {
        Self {
            phone_number_id: phone_number_id.into(),
            fields: Vec::new(),
        }
    }

    /// Sets the response fields, in wire order.
    ///
    /// ```
    /// use meta_whatsapp::flow::{
    ///     FlowEncryptionPublicKeyField, FlowEncryptionPublicKeyRequest,
    /// };
    ///
    /// let request = FlowEncryptionPublicKeyRequest::new("1234").fields([
    ///     FlowEncryptionPublicKeyField::BusinessPublicKey,
    ///     FlowEncryptionPublicKeyField::SignatureStatus,
    /// ]);
    /// assert_eq!(request.fields.len(), 2);
    /// ```
    #[must_use]
    pub fn fields<I>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = FlowEncryptionPublicKeyField>,
    {
        self.fields = fields.into_iter().collect();
        self
    }

    /// Validates the phone-number id before any URL is built.
    ///
    /// Meta's OpenAPI path parameter is digits-only. This crate applies its
    /// existing 32-digit ceiling as a concrete URL bound.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowEncryptionPublicKeyRequest;
    ///
    /// assert!(FlowEncryptionPublicKeyRequest::new("1234").validate().is_ok());
    /// assert!(FlowEncryptionPublicKeyRequest::new("../me").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `phone_number_id` when it is empty, longer
    /// than 32 bytes, or contains anything other than ASCII digits.
    pub fn validate(&self) -> Result<()> {
        crate::phone_number::validate_phone_number_id(&self.phone_number_id)
    }

    /// Renders the optional `fields` query string without the leading `?`.
    ///
    /// ```
    /// use meta_whatsapp::flow::{
    ///     FlowEncryptionPublicKeyField, FlowEncryptionPublicKeyRequest,
    /// };
    ///
    /// assert_eq!(
    ///     FlowEncryptionPublicKeyRequest::new("1234").query_string(),
    ///     ""
    /// );
    /// assert_eq!(
    ///     FlowEncryptionPublicKeyRequest::new("1234")
    ///         .fields([
    ///             FlowEncryptionPublicKeyField::BusinessPublicKey,
    ///             FlowEncryptionPublicKeyField::SignatureStatus,
    ///         ])
    ///         .query_string(),
    ///     "fields=business_public_key%2Cbusiness_public_key_signature_status"
    /// );
    /// ```
    #[must_use]
    pub fn query_string(&self) -> String {
        if self.fields.is_empty() {
            return String::new();
        }
        let joined = self
            .fields
            .iter()
            .map(|field| field.as_str())
            .collect::<Vec<_>>()
            .join(",");
        format!("fields={}", encode_query_value(&joined))
    }
}

/// Meta's verification state for the stored business public key.
///
/// The documented values are typed. A value Meta adds later is preserved in
/// [`FlowPublicKeySignatureStatus::Unknown`] rather than making the response
/// unreadable.
///
/// ```
/// use meta_whatsapp::flow::FlowPublicKeySignatureStatus;
///
/// assert_eq!(
///     FlowPublicKeySignatureStatus::parse("VALID"),
///     FlowPublicKeySignatureStatus::Valid
/// );
/// assert_eq!(
///     FlowPublicKeySignatureStatus::parse("FUTURE"),
///     FlowPublicKeySignatureStatus::Unknown("FUTURE".into())
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FlowPublicKeySignatureStatus {
    /// Meta verified the stored signature.
    Valid,
    /// The stored signature does not match the public key.
    Mismatch,
    /// A status this crate does not know; the raw value is kept.
    Unknown(String),
}

impl Default for FlowPublicKeySignatureStatus {
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl FlowPublicKeySignatureStatus {
    /// Parses a status case-insensitively, retaining unknown values.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPublicKeySignatureStatus;
    ///
    /// assert_eq!(
    ///     FlowPublicKeySignatureStatus::parse(" mismatch "),
    ///     FlowPublicKeySignatureStatus::Mismatch
    /// );
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "VALID" => Self::Valid,
            "MISMATCH" => Self::Mismatch,
            _ => Self::Unknown(raw.trim().to_string()),
        }
    }

    /// The status as Meta spells it, or the raw unknown value.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPublicKeySignatureStatus;
    ///
    /// assert_eq!(FlowPublicKeySignatureStatus::Valid.as_str(), "VALID");
    /// assert_eq!(
    ///     FlowPublicKeySignatureStatus::Unknown("FUTURE".into()).as_str(),
    ///     "FUTURE"
    /// );
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Valid => "VALID",
            Self::Mismatch => "MISMATCH",
            Self::Unknown(raw) => raw,
        }
    }
}

impl std::fmt::Display for FlowPublicKeySignatureStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for FlowPublicKeySignatureStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FlowPublicKeySignatureStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// Public-key information returned by the business-encryption endpoint.
///
/// The schema marks both fields required, but `fields` can select either one,
/// so both are optional here. Parsing accepts the flat guide example and the
/// OpenAPI `{"data":[{...}]}` envelope; members this crate does not model stay
/// in [`FlowEncryptionPublicKeyResponse::extra`].
///
/// ```
/// use meta_whatsapp::flow::{
///     FlowEncryptionPublicKeyResponse, FlowPublicKeySignatureStatus,
/// };
///
/// let response = FlowEncryptionPublicKeyResponse::parse(&serde_json::json!({
///     "data": [{
///         "business_public_key": "-----BEGIN PUBLIC KEY-----",
///         "business_public_key_signature_status": "VALID"
///     }]
/// }))?;
/// assert_eq!(
///     response.business_public_key_signature_status,
///     Some(FlowPublicKeySignatureStatus::Valid)
/// );
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowEncryptionPublicKeyResponse {
    /// PEM-encoded RSA public key, when selected.
    #[serde(default)]
    pub business_public_key: Option<String>,
    /// Meta's signature verification status, when selected.
    #[serde(default)]
    pub business_public_key_signature_status: Option<FlowPublicKeySignatureStatus>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl FlowEncryptionPublicKeyResponse {
    /// Reads the key information out of a flat or `data[0]`-wrapped response.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowEncryptionPublicKeyResponse;
    ///
    /// assert!(
    ///     FlowEncryptionPublicKeyResponse::parse(&serde_json::json!({
    ///         "business_public_key_signature_status": "MISMATCH"
    ///     }))
    ///     .is_ok()
    /// );
    /// assert!(FlowEncryptionPublicKeyResponse::parse(&serde_json::json!({
    ///     "data": []
    /// })).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the response is not an object omitting `data`,
    /// or when `data` is not a non-empty array whose first member is an
    /// object. [`Error::Json`] when a modeled member has the wrong JSON type.
    pub fn parse(value: &Value) -> Result<Self> {
        let candidate = match value {
            Value::Object(map) => match map.get("data") {
                Some(Value::Array(entries)) => entries.first().map_or_else(
                    || Err(Error::Payload("`data` was empty".to_string())),
                    |entry| {
                        entry.as_object().map(|_| entry).ok_or_else(|| {
                            Error::Payload("`data[0]` was not an object".to_string())
                        })
                    },
                )?,
                Some(_) => {
                    return Err(Error::Payload(
                        "`data` was not an array of objects".to_string(),
                    ))
                }
                None => value,
            },
            _ => {
                return Err(Error::Payload(
                    "expected a JSON object for the Graph API response".to_string(),
                ))
            }
        };

        serde_json::from_value(candidate.clone()).map_err(|source| Error::Json { source })
    }
}

/// Lifecycle state of a Flow.
///
/// The documented states are typed; a state Meta adds later is retained
/// verbatim in [`FlowStatus::Unknown`] instead of failing the response.
///
/// ```
/// use meta_whatsapp::flow::FlowStatus;
///
/// assert_eq!(FlowStatus::parse("published"), FlowStatus::Published);
/// assert_eq!(FlowStatus::parse("PAUSED"), FlowStatus::Unknown("PAUSED".into()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FlowStatus {
    /// Editable and not yet published.
    Draft,
    /// Published and immutable.
    Published,
    /// Published previously but no longer openable or sendable.
    Deprecated,
    /// Monitoring found the endpoint unhealthy; the Flow cannot be opened or
    /// sent until it returns to `PUBLISHED`.
    Blocked,
    /// Monitoring found the endpoint unhealthy; the Flow can be opened but only
    /// a limited number of messages can be sent until it returns to `PUBLISHED`.
    Throttled,
    /// A status this crate does not know; the raw value is kept.
    Unknown(String),
}

impl Default for FlowStatus {
    fn default() -> Self {
        Self::Unknown(String::new())
    }
}

impl FlowStatus {
    /// Parses a status case-insensitively, retaining unknown values.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowStatus;
    ///
    /// assert_eq!(FlowStatus::parse(" DRAFT "), FlowStatus::Draft);
    /// assert_eq!(FlowStatus::parse("DEPRECATED"), FlowStatus::Deprecated);
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "DRAFT" => Self::Draft,
            "PUBLISHED" => Self::Published,
            "DEPRECATED" => Self::Deprecated,
            "BLOCKED" => Self::Blocked,
            "THROTTLED" => Self::Throttled,
            _ => Self::Unknown(raw.trim().to_string()),
        }
    }

    /// The status as Meta spells it, or the raw unknown value.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowStatus;
    ///
    /// assert_eq!(FlowStatus::Draft.as_str(), "DRAFT");
    /// assert_eq!(FlowStatus::Unknown("PAUSED".into()).as_str(), "PAUSED");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Draft => "DRAFT",
            Self::Published => "PUBLISHED",
            Self::Deprecated => "DEPRECATED",
            Self::Blocked => "BLOCKED",
            Self::Throttled => "THROTTLED",
            Self::Unknown(raw) => raw,
        }
    }
}

impl Serialize for FlowStatus {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FlowStatus {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// A Flow returned by `GET /{Flow-ID}`.
///
/// Every documented field is optional because a request can select only a
/// subset. Unknown members and endpoint metrics are preserved in
/// [`Flow::extra`].
///
/// ```
/// use meta_whatsapp::flow::{Flow, FlowStatus};
///
/// let flow = Flow::parse(&serde_json::json!({
///     "id": "flow-1", "name": "Appointments", "status": "DRAFT",
///     "json_version": "5.0", "validation_errors": []
/// }))?;
/// assert_eq!(flow.name.as_deref(), Some("Appointments"));
/// assert_eq!(flow.status, Some(FlowStatus::Draft));
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Flow {
    /// Flow id.
    #[serde(default)]
    pub id: Option<String>,
    /// Human-readable Flow name.
    #[serde(default)]
    pub name: Option<String>,
    /// Lifecycle state.
    #[serde(default)]
    pub status: Option<FlowStatus>,
    /// Categories assigned to the Flow, in wire order.
    #[serde(default)]
    pub categories: Vec<String>,
    /// Flow JSON schema version.
    #[serde(default)]
    pub json_version: Option<String>,
    /// Data-exchange API version, when configured.
    #[serde(default)]
    pub data_api_version: Option<String>,
    /// Data-channel URI, when configured.
    #[serde(default)]
    pub data_channel_uri: Option<String>,
    /// Data-exchange endpoint URI, when configured.
    #[serde(default)]
    pub endpoint_uri: Option<String>,
    /// Preview link, when requested.
    #[serde(default)]
    pub preview: Option<FlowPreviewLink>,
    /// Validation failures from the uploaded Flow JSON.
    #[serde(default)]
    pub validation_errors: Vec<FlowValidationError>,
    /// Health state and the entities that affect whether messages can be sent.
    #[serde(default)]
    pub health_status: Option<FlowHealthStatus>,
    /// Owning WhatsApp Business Account, when requested.
    #[serde(default)]
    pub whatsapp_business_account: Option<FlowWhatsAppBusinessAccount>,
    /// Connected Meta application, when requested.
    #[serde(default)]
    pub application: Option<FlowApplication>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl Flow {
    /// Parses a Flow response, retaining serde's error as the source.
    ///
    /// ```
    /// use meta_whatsapp::flow::Flow;
    ///
    /// assert!(Flow::parse(&serde_json::json!({"id": "flow-1"})).is_ok());
    /// assert!(Flow::parse(&serde_json::json!({"categories": "not an array"})).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when a modeled member has the wrong JSON type.
    pub fn parse(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(|source| Error::Json { source })
    }
}

/// A Flow preview link and its expiry.
///
/// Meta creates preview links for 30 days. A later call with
/// [`FlowPreviewRequest::invalidate`] set to `true` replaces the link.
///
/// ```
/// use meta_whatsapp::flow::FlowPreviewLink;
///
/// let preview: FlowPreviewLink = serde_json::from_value(serde_json::json!({
///     "preview_url": "https://business.facebook.com/wa/manage/flows/1/preview/?token=t",
///     "expires_at": 1735689600
/// }))?;
/// assert_eq!(preview.expires_at.as_deref(), Some("1735689600"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowPreviewLink {
    /// Public preview URL; viewers do not need to log in.
    #[serde(default)]
    pub preview_url: Option<String>,
    /// Expiry date as Meta returned it. A numeric unix timestamp is retained as
    /// its decimal string, because `date_format=U` can change the wire type.
    #[serde(default, deserialize_with = "deserialize_optional_date")]
    pub expires_at: Option<String>,
}

impl FlowPreviewLink {
    /// The expiry as unix seconds, when it is a decimal timestamp.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewLink;
    ///
    /// let preview: FlowPreviewLink = serde_json::from_value(serde_json::json!({
    ///     "expires_at": 1_735_689_600_u64
    /// }))?;
    /// assert_eq!(preview.expires_at_secs(), Some(1_735_689_600));
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn expires_at_secs(&self) -> Option<i64> {
        self.expires_at.as_deref()?.trim().parse().ok()
    }
}

/// The response of `GET /{Flow-ID}?fields=preview.invalidate(…)`.
///
/// ```
/// use meta_whatsapp::flow::FlowPreviewResponse;
///
/// let response: FlowPreviewResponse = serde_json::from_value(serde_json::json!({
///     "id": "flow-1",
///     "preview": {"preview_url": "https://example.test", "expires_at": "2026-12-06T00:00:00+0000"}
/// }))?;
/// assert_eq!(response.id.as_deref(), Some("flow-1"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowPreviewResponse {
    /// Flow id.
    #[serde(default)]
    pub id: Option<String>,
    /// Preview link, absent only in an unexpected response.
    #[serde(default)]
    pub preview: Option<FlowPreviewLink>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl FlowPreviewResponse {
    /// Parses a preview response, retaining serde's error as the source.
    ///
    /// ```
    /// use meta_whatsapp::flow::FlowPreviewResponse;
    ///
    /// assert!(FlowPreviewResponse::parse(&serde_json::json!({"id": "flow-1"})).is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when a modeled member has the wrong JSON type.
    pub fn parse(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(|source| Error::Json { source })
    }
}

/// One validation failure in a Flow JSON document.
///
/// The OpenAPI schema leaves array members untyped while its examples are
/// objects. Non-object members are retained in [`FlowValidationError::extra`]
/// under `value` rather than making the whole Flow unreadable.
///
/// ```
/// use meta_whatsapp::flow::FlowValidationError;
///
/// let error: FlowValidationError = serde_json::from_value(serde_json::json!({
///     "error": "INVALID_PROPERTY",
///     "error_type": "JSON_SCHEMA_ERROR",
///     "message": "The property is not allowed",
///     "line_start": 46
/// }))?;
/// assert_eq!(error.line_start, Some(46));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct FlowValidationError {
    /// Machine-readable error kind.
    #[serde(default)]
    pub error: Option<String>,
    /// Error family, for example `JSON_SCHEMA_ERROR`.
    #[serde(default)]
    pub error_type: Option<String>,
    /// Human-readable explanation.
    #[serde(default)]
    pub message: Option<String>,
    /// First source line, when supplied.
    #[serde(default)]
    pub line_start: Option<u64>,
    /// Last source line, when supplied.
    #[serde(default)]
    pub line_end: Option<u64>,
    /// First source column, when supplied.
    #[serde(default)]
    pub column_start: Option<u64>,
    /// Last source column, when supplied.
    #[serde(default)]
    pub column_end: Option<u64>,
    /// More precise pointers into the Flow JSON.
    #[serde(default)]
    pub pointers: Vec<FlowValidationPointer>,
    /// Members this crate does not model or could not read, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl<'de> Deserialize<'de> for FlowValidationError {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        match Value::deserialize(deserializer)? {
            Value::Object(mut map) => {
                let error = take_string(&mut map, "error");
                let error_type = take_string(&mut map, "error_type");
                let message = take_string(&mut map, "message");
                let line_start = take_u64(&mut map, "line_start");
                let line_end = take_u64(&mut map, "line_end");
                let column_start = take_u64(&mut map, "column_start");
                let column_end = take_u64(&mut map, "column_end");
                let pointers = take_pointers(&mut map);
                Ok(Self {
                    error,
                    error_type,
                    message,
                    line_start,
                    line_end,
                    column_start,
                    column_end,
                    pointers,
                    extra: map,
                })
            }
            other => {
                let mut extra = Map::new();
                extra.insert("value".to_string(), other);
                Ok(Self {
                    extra,
                    ..Self::default()
                })
            }
        }
    }
}

/// One nested pointer inside a Flow validation error.
///
/// ```
/// use meta_whatsapp::flow::FlowValidationPointer;
///
/// let pointer: FlowValidationPointer = serde_json::from_value(serde_json::json!({
///     "line_start": 10, "column_start": 21,
///     "path": "screens [0].layout.children [0].type"
/// }))?;
/// assert_eq!(pointer.path.as_deref(), Some("screens [0].layout.children [0].type"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowValidationPointer {
    /// First source line.
    #[serde(default)]
    pub line_start: Option<u64>,
    /// Last source line.
    #[serde(default)]
    pub line_end: Option<u64>,
    /// First source column.
    #[serde(default)]
    pub column_start: Option<u64>,
    /// Last source column.
    #[serde(default)]
    pub column_end: Option<u64>,
    /// Path into the Flow JSON.
    #[serde(default)]
    pub path: Option<String>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// Overall health state for sending through a Flow.
///
/// ```
/// use meta_whatsapp::flow::FlowHealthStatus;
///
/// let health: FlowHealthStatus = serde_json::from_value(serde_json::json!({
///     "can_send_message": "BLOCKED",
///     "entities": [{"id": "waba-1", "entity_type": "WABA", "can_send_message": "BLOCKED"}]
/// }))?;
/// assert_eq!(health.can_send_message.as_deref(), Some("BLOCKED"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowHealthStatus {
    /// Whether the Flow can currently send messages (`AVAILABLE`, `LIMITED` or
    /// `BLOCKED`). Kept as a string because Meta has not published an enum.
    #[serde(default)]
    pub can_send_message: Option<String>,
    /// Flow, WABA, business and app entities contributing to the health state.
    #[serde(default)]
    pub entities: Vec<FlowHealthEntity>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// One entity in a Flow health response.
///
/// ```
/// use meta_whatsapp::flow::FlowHealthEntity;
///
/// let entity: FlowHealthEntity = serde_json::from_value(serde_json::json!({
///     "id": "flow-1", "entity_type": "FLOW", "can_send_message": "AVAILABLE"
/// }))?;
/// assert_eq!(entity.id.as_deref(), Some("flow-1"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowHealthEntity {
    /// Entity id.
    #[serde(default)]
    pub id: Option<String>,
    /// Entity type, for example `FLOW`, `WABA`, `BUSINESS` or `APP`.
    #[serde(default)]
    pub entity_type: Option<String>,
    /// Entity-level send state.
    #[serde(default)]
    pub can_send_message: Option<String>,
    /// Reasons the entity cannot send.
    #[serde(default)]
    pub errors: Vec<FlowHealthError>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// One reason a Flow-health entity is blocked or limited.
///
/// ```
/// use meta_whatsapp::flow::FlowHealthError;
///
/// let error: FlowHealthError = serde_json::from_value(serde_json::json!({
///     "error_code": 141006,
///     "error_description": "The payment method is unavailable."
/// }))?;
/// assert_eq!(error.error_code.unwrap().as_u64(), Some(141006));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowHealthError {
    /// Numeric Meta error code, kept as a JSON number so integer and decimal
    /// forms both decode.
    #[serde(default)]
    pub error_code: Option<Number>,
    /// Explanation of the failure.
    #[serde(default)]
    pub error_description: Option<String>,
    /// Suggested remediation.
    #[serde(default)]
    pub possible_solution: Option<String>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// The WhatsApp Business Account owning a Flow.
///
/// ```
/// use meta_whatsapp::flow::FlowWhatsAppBusinessAccount;
///
/// let waba: FlowWhatsAppBusinessAccount = serde_json::from_value(serde_json::json!({
///     "id": "waba-1", "name": "Example WABA", "timezone_id": "54"
/// }))?;
/// assert_eq!(waba.id.as_deref(), Some("waba-1"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowWhatsAppBusinessAccount {
    /// WABA id.
    #[serde(default)]
    pub id: Option<String>,
    /// WABA display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Business type, for example `ent`.
    #[serde(default)]
    pub business_type: Option<String>,
    /// Message-template namespace.
    #[serde(default)]
    pub message_template_namespace: Option<String>,
    /// Meta timezone id.
    #[serde(default)]
    pub timezone_id: Option<String>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// The Meta application connected to a Flow's data-exchange endpoint.
///
/// ```
/// use meta_whatsapp::flow::FlowApplication;
///
/// let app: FlowApplication = serde_json::from_value(serde_json::json!({
///     "id": "app-1", "name": "Example App"
/// }))?;
/// assert_eq!(app.name.as_deref(), Some("Example App"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FlowApplication {
    /// App id.
    #[serde(default)]
    pub id: Option<String>,
    /// App display name.
    #[serde(default)]
    pub name: Option<String>,
    /// Link to the app's settings page.
    #[serde(default)]
    pub link: Option<String>,
    /// Members this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

fn take_string(map: &mut Map<String, Value>, key: &str) -> Option<String> {
    match map.remove(key) {
        Some(Value::String(value)) => Some(value),
        Some(other) => {
            map.insert(key.to_string(), other);
            None
        }
        None => None,
    }
}

fn take_u64(map: &mut Map<String, Value>, key: &str) -> Option<u64> {
    let value = map.remove(key)?;
    if let Some(number) = value.as_u64() {
        Some(number)
    } else {
        map.insert(key.to_string(), value);
        None
    }
}

fn take_pointers(map: &mut Map<String, Value>) -> Vec<FlowValidationPointer> {
    let Some(value) = map.remove("pointers") else {
        return Vec::new();
    };
    match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| serde_json::from_value(item).ok())
            .collect(),
        other => {
            map.insert("pointers".to_string(), other);
            Vec::new()
        }
    }
}

fn deserialize_optional_date<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<Value>::deserialize(deserializer)? {
        Some(Value::String(value)) => Some(value),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(_) | None => None,
    })
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
