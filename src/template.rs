//! Template messages, listing and management.
//!
//! Templates are pre-approved by Meta. Sending one references its name and
//! language plus the positional parameters for the header, body and buttons;
//! listing them reads `/{waba_id}/message_templates` with cursor paging, and
//! [`crate::WhatsAppClient::create_template`],
//! [`crate::WhatsAppClient::update_template`] and
//! [`crate::WhatsAppClient::delete_template`] manage the definitions.
//!
//! ```
//! use meta_whatsapp::{MessageRequest, TemplateMessage};
//!
//! let json = MessageRequest::new(
//!     "4915112345678",
//!     TemplateMessage::new("order_update", "de")
//!         .body_text("Florian")
//!         .body_text("42"),
//! )
//! .to_json()?;
//!
//! assert_eq!(json["type"], "template");
//! assert_eq!(json["template"]["name"], "order_update");
//! assert_eq!(json["template"]["language"]["code"], "de");
//! assert_eq!(json["template"]["components"][0]["parameters"][1]["text"], "42");
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::error::{Error, Result};
use crate::media::MediaRef;

/// Maximum UTF-8 byte length of a template name.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_NAME_MAX_BYTES;
/// assert_eq!(TEMPLATE_NAME_MAX_BYTES, 512);
/// ```
pub const TEMPLATE_NAME_MAX_BYTES: usize = 512;

/// Maximum character length of a template language code.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_LANGUAGE_MAX_CHARS;
/// assert_eq!(TEMPLATE_LANGUAGE_MAX_CHARS, 32);
/// ```
pub const TEMPLATE_LANGUAGE_MAX_CHARS: usize = 32;

/// Maximum UTF-8 byte length of a template parameter example.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_PARAMETER_MAX_BYTES;
/// assert_eq!(TEMPLATE_PARAMETER_MAX_BYTES, 1024);
/// ```
pub const TEMPLATE_PARAMETER_MAX_BYTES: usize = 1024;

/// Maximum character length of a text header definition.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_HEADER_MAX_CHARS;
/// assert_eq!(TEMPLATE_HEADER_MAX_CHARS, 60);
/// ```
pub const TEMPLATE_HEADER_MAX_CHARS: usize = 60;

/// Maximum UTF-8 byte length of a body definition.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_BODY_MAX_BYTES;
/// assert_eq!(TEMPLATE_BODY_MAX_BYTES, 1024);
/// ```
pub const TEMPLATE_BODY_MAX_BYTES: usize = 1024;

/// Maximum character length of a footer definition.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_FOOTER_MAX_CHARS;
/// assert_eq!(TEMPLATE_FOOTER_MAX_CHARS, 60);
/// ```
pub const TEMPLATE_FOOTER_MAX_CHARS: usize = 60;

/// Maximum character length of a template button label.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_BUTTON_TEXT_MAX_CHARS;
/// assert_eq!(TEMPLATE_BUTTON_TEXT_MAX_CHARS, 25);
/// ```
pub const TEMPLATE_BUTTON_TEXT_MAX_CHARS: usize = 25;

/// Maximum character length of a URL button's destination.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_BUTTON_URL_MAX_CHARS;
/// assert_eq!(TEMPLATE_BUTTON_URL_MAX_CHARS, 2000);
/// ```
pub const TEMPLATE_BUTTON_URL_MAX_CHARS: usize = 2000;

/// Maximum character length of a copy-code button example.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS;
/// assert_eq!(TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS, 15);
/// ```
pub const TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS: usize = 15;

/// Maximum number of buttons in a template.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_BUTTONS_MAX;
/// assert_eq!(TEMPLATE_BUTTONS_MAX, 10);
/// ```
pub const TEMPLATE_BUTTONS_MAX: usize = 10;

/// Maximum number of URL buttons in a template.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_URL_BUTTONS_MAX;
/// assert_eq!(TEMPLATE_URL_BUTTONS_MAX, 2);
/// ```
pub const TEMPLATE_URL_BUTTONS_MAX: usize = 2;

/// Maximum number of copy-code buttons in a template.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_COPY_CODE_BUTTONS_MAX;
/// assert_eq!(TEMPLATE_COPY_CODE_BUTTONS_MAX, 1);
/// ```
pub const TEMPLATE_COPY_CODE_BUTTONS_MAX: usize = 1;

/// Maximum number of example parameters in one component.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_EXAMPLES_MAX;
/// assert_eq!(TEMPLATE_EXAMPLES_MAX, 20);
/// ```
pub const TEMPLATE_EXAMPLES_MAX: usize = 20;

/// Maximum number of component groups in a template definition.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_COMPONENTS_MAX;
/// assert_eq!(TEMPLATE_COMPONENTS_MAX, 4);
/// ```
pub const TEMPLATE_COMPONENTS_MAX: usize = 4;

/// Language block of a template: `code` plus optional `policy`.
///
/// ```
/// use meta_whatsapp::TemplateLanguage;
///
/// let language = TemplateLanguage::new("de_DE").policy("deterministic");
/// assert_eq!(language.code, "de_DE");
/// assert_eq!(language.policy.as_deref(), Some("deterministic"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateLanguage {
    /// BCP-47-ish code Meta assigned, e.g. `de`, `en_US`.
    pub code: String,
    /// `deterministic` is the only documented policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
}

impl TemplateLanguage {
    /// Language block with no policy.
    ///
    /// ```
    /// use meta_whatsapp::TemplateLanguage;
    /// assert!(TemplateLanguage::new("de").policy.is_none());
    /// ```
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            policy: None,
        }
    }

    /// Sets the policy.
    ///
    /// ```
    /// use meta_whatsapp::TemplateLanguage;
    /// assert_eq!(TemplateLanguage::new("de").policy("deterministic").policy.unwrap(), "deterministic");
    /// ```
    #[must_use]
    pub fn policy(mut self, policy: impl Into<String>) -> Self {
        self.policy = Some(policy.into());
        self
    }

    fn validate(&self) -> Result<()> {
        if self.code.is_empty() {
            return Err(Error::validation(
                "template.language.code",
                "must not be empty",
            ));
        }
        let count = self.code.chars().count();
        if count > TEMPLATE_LANGUAGE_MAX_CHARS {
            return Err(Error::validation(
                "template.language.code",
                format!(
                    "must be at most {TEMPLATE_LANGUAGE_MAX_CHARS} characters, got {count}"
                ),
            ));
        }
        if !self
            .code
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(Error::validation(
                "template.language.code",
                "only ASCII alphanumerics, '_' and '-' are allowed",
            ));
        }
        Ok(())
    }
}

/// Media kind inside a template header parameter.
///
/// ```
/// use meta_whatsapp::TemplateMediaKind;
/// assert_eq!(TemplateMediaKind::Document.as_str(), "document");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TemplateMediaKind {
    /// Image header.
    #[default]
    Image,
    /// Video header.
    Video,
    /// Document header.
    Document,
}

impl TemplateMediaKind {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMediaKind;
    /// assert_eq!(TemplateMediaKind::Image.as_str(), "image");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Document => "document",
        }
    }
}

/// One positional parameter of a template component.
///
/// ```
/// use meta_whatsapp::TemplateParameter;
///
/// let text = TemplateParameter::text("Florian");
/// assert!(matches!(text, TemplateParameter::Text { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateParameter {
    /// Free text.
    Text {
        /// Value substituted into the placeholder.
        text: String,
    },
    /// Currency amount.
    Currency {
        /// What to show when the real amount is unavailable.
        fallback_value: String,
        /// ISO-4217 code.
        code: String,
        /// Amount multiplied by 1000 (Meta's unit).
        amount_1000: i64,
    },
    /// Date/time string, already formatted.
    DateTime {
        /// What to show when the timestamp is unavailable.
        fallback_value: String,
    },
    /// Image, video or document header parameter.
    Media {
        /// Uploaded id or public https link.
        media: MediaRef,
        /// Which media kind the placeholder expects.
        kind: TemplateMediaKind,
    },
}

impl TemplateParameter {
    /// A text parameter.
    ///
    /// ```
    /// use meta_whatsapp::TemplateParameter;
    /// assert!(TemplateParameter::text("x").validate().is_ok());
    /// ```
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// A currency parameter.
    ///
    /// ```
    /// use meta_whatsapp::TemplateParameter;
    /// let price = TemplateParameter::currency("$10.00", "USD", 10_000);
    /// assert!(matches!(price, TemplateParameter::Currency { .. }));
    /// ```
    #[must_use]
    pub fn currency(
        fallback_value: impl Into<String>,
        code: impl Into<String>,
        amount_1000: i64,
    ) -> Self {
        Self::Currency {
            fallback_value: fallback_value.into(),
            code: code.into(),
            amount_1000,
        }
    }

    /// A date/time parameter.
    ///
    /// ```
    /// use meta_whatsapp::TemplateParameter;
    /// assert!(matches!(
    ///     TemplateParameter::date_time("2026-10-24"),
    ///     TemplateParameter::DateTime { .. }
    /// ));
    /// ```
    #[must_use]
    pub fn date_time(fallback_value: impl Into<String>) -> Self {
        Self::DateTime {
            fallback_value: fallback_value.into(),
        }
    }

    /// A media parameter (image/video/document header).
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, TemplateMediaKind, TemplateParameter};
    ///
    /// let media = TemplateParameter::media(MediaRef::id("1"), TemplateMediaKind::Image);
    /// assert!(matches!(media, TemplateParameter::Media { .. }));
    /// ```
    #[must_use]
    pub fn media(media: MediaRef, kind: TemplateMediaKind) -> Self {
        Self::Media { media, kind }
    }

    /// Checks text lengths and media references.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, TemplateMediaKind, TemplateParameter};
    ///
    /// let bad = TemplateParameter::media(MediaRef::id(""), TemplateMediaKind::Image);
    /// assert!(bad.validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for oversized text or an unsafe media reference.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Text { text } => {
                if text.is_empty() {
                    return Err(Error::validation(
                        "template.parameters.text",
                        "must not be empty",
                    ));
                }
                require_max_bytes(
                    "template.parameters.text",
                    text,
                    TEMPLATE_PARAMETER_MAX_BYTES,
                )
            }
            Self::Currency {
                fallback_value,
                code,
                ..
            } => {
                require_max_bytes(
                    "template.parameters.currency.fallback_value",
                    fallback_value,
                    64,
                )?;
                if code.len() != 3 || !code.bytes().all(|b| b.is_ascii_uppercase()) {
                    return Err(Error::validation(
                        "template.parameters.currency.code",
                        "must be a three-letter upper-case ISO-4217 code",
                    ));
                }
                Ok(())
            }
            Self::DateTime { fallback_value } => require_max_bytes(
                "template.parameters.date_time.fallback_value",
                fallback_value,
                64,
            ),
            Self::Media { media, .. } => media.validate(),
        }
    }

    fn render(&self, quick_reply: bool) -> Value {
        match self {
            Self::Text { text } => {
                if quick_reply {
                    json!({ "type": "payload", "payload": text })
                } else {
                    json!({ "type": "text", "text": text })
                }
            }
            Self::Currency {
                fallback_value,
                code,
                amount_1000,
            } => json!({
                "type": "currency",
                "currency": {
                    "fallback_value": fallback_value,
                    "code": code,
                    "amount_1000": amount_1000,
                },
            }),
            Self::DateTime { fallback_value } => json!({
                "type": "date_time",
                "date_time": { "fallback_value": fallback_value },
            }),
            Self::Media { media, kind } => json!({
                "type": kind.as_str(),
                kind.as_str(): media.to_json(),
            }),
        }
    }
}

/// Button kind a template component targets.
///
/// ```
/// use meta_whatsapp::TemplateButtonType;
/// assert_eq!(TemplateButtonType::Url.as_str(), "url");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TemplateButtonType {
    /// Quick-reply button (parameter type `payload`).
    #[default]
    QuickReply,
    /// URL button (parameter type `text`).
    Url,
    /// Copy-code button (parameter type `text`).
    CopyCode,
}

impl TemplateButtonType {
    /// Wire representation (`quick_reply`, `url`, `copy_code`).
    ///
    /// ```
    /// use meta_whatsapp::TemplateButtonType;
    /// assert_eq!(TemplateButtonType::QuickReply.as_str(), "quick_reply");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QuickReply => "quick_reply",
            Self::Url => "url",
            Self::CopyCode => "copy_code",
        }
    }
}

/// One component of a template message.
///
/// ```
/// use meta_whatsapp::{TemplateComponent, TemplateParameter};
///
/// let body = TemplateComponent::body(vec![TemplateParameter::text("Florian")]);
/// assert_eq!(body.kind(), "body");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateComponent {
    /// Header parameters (text or media).
    Header {
        /// Positional parameters.
        parameters: Vec<TemplateParameter>,
    },
    /// Body parameters.
    Body {
        /// Positional parameters.
        parameters: Vec<TemplateParameter>,
    },
    /// Button parameters.
    Button {
        /// Which button kind the template declares.
        sub_type: TemplateButtonType,
        /// Zero-based button index as a string on the wire.
        index: u16,
        /// Positional parameters.
        parameters: Vec<TemplateParameter>,
    },
}

impl TemplateComponent {
    /// A header component.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateComponent, TemplateParameter};
    /// assert_eq!(TemplateComponent::header(vec![TemplateParameter::text("H")]).kind(), "header");
    /// ```
    #[must_use]
    pub fn header(parameters: Vec<TemplateParameter>) -> Self {
        Self::Header { parameters }
    }

    /// A body component.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateComponent, TemplateParameter};
    /// assert_eq!(TemplateComponent::body(vec![TemplateParameter::text("B")]).kind(), "body");
    /// ```
    #[must_use]
    pub fn body(parameters: Vec<TemplateParameter>) -> Self {
        Self::Body { parameters }
    }

    /// A button component.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateButtonType, TemplateComponent, TemplateParameter};
    ///
    /// let button = TemplateComponent::button(
    ///     TemplateButtonType::QuickReply,
    ///     0,
    ///     vec![TemplateParameter::text("yes")],
    /// );
    /// assert_eq!(button.kind(), "button");
    /// ```
    #[must_use]
    pub fn button(
        sub_type: TemplateButtonType,
        index: u16,
        parameters: Vec<TemplateParameter>,
    ) -> Self {
        Self::Button {
            sub_type,
            index,
            parameters,
        }
    }

    /// `header`, `body` or `button`.
    ///
    /// ```
    /// use meta_whatsapp::TemplateComponent;
    /// assert_eq!(TemplateComponent::body(Vec::new()).kind(), "body");
    /// ```
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Header { .. } => "header",
            Self::Body { .. } => "body",
            Self::Button { .. } => "button",
        }
    }

    /// The positional parameters of this component.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateComponent, TemplateParameter};
    ///
    /// let body = TemplateComponent::body(vec![TemplateParameter::text("a")]);
    /// assert_eq!(body.parameters().len(), 1);
    /// ```
    #[must_use]
    pub fn parameters(&self) -> &[TemplateParameter] {
        match self {
            Self::Header { parameters }
            | Self::Body { parameters }
            | Self::Button { parameters, .. } => parameters,
        }
    }

    /// Validates every parameter in the component.
    ///
    /// ```
    /// use meta_whatsapp::TemplateComponent;
    ///
    /// assert!(TemplateComponent::body(Vec::new()).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from any parameter.
    pub fn validate(&self) -> Result<()> {
        for parameter in self.parameters() {
            parameter.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        match self {
            Self::Header { parameters } => json!({
                "type": "header",
                "parameters": parameters.iter().map(|p| p.render(false)).collect::<Vec<_>>(),
            }),
            Self::Body { parameters } => json!({
                "type": "body",
                "parameters": parameters.iter().map(|p| p.render(false)).collect::<Vec<_>>(),
            }),
            Self::Button {
                sub_type,
                index,
                parameters,
            } => json!({
                "type": "button",
                "sub_type": sub_type.as_str(),
                "index": index.to_string(),
                "parameters": parameters
                    .iter()
                    .map(|p| p.render(matches!(sub_type, TemplateButtonType::QuickReply)))
                    .collect::<Vec<_>>(),
            }),
        }
    }
}

/// A template message body.
///
/// ```
/// use meta_whatsapp::TemplateMessage;
///
/// let template = TemplateMessage::new("hello_world", "en_US")
///     .language_policy("deterministic");
/// assert_eq!(template.language.policy.as_deref(), Some("deterministic"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateMessage {
    /// Template name as approved by Meta.
    pub name: String,
    /// Language block.
    pub language: TemplateLanguage,
    /// Components in render order.
    pub components: Vec<TemplateComponent>,
}

impl TemplateMessage {
    /// A template with no components (valid for templates without variables).
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    /// assert!(TemplateMessage::new("hello_world", "en").components.is_empty());
    /// ```
    pub fn new(name: impl Into<String>, language_code: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            language: TemplateLanguage::new(language_code),
            components: Vec::new(),
        }
    }

    /// Sets the language policy.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    /// assert_eq!(
    ///     TemplateMessage::new("t", "de").language_policy("deterministic").language.policy.unwrap(),
    ///     "deterministic"
    /// );
    /// ```
    #[must_use]
    pub fn language_policy(mut self, policy: impl Into<String>) -> Self {
        self.language = self.language.policy(policy);
        self
    }

    /// Appends a header text parameter.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    /// assert_eq!(TemplateMessage::new("t", "de").header_text("Hallo").components.len(), 1);
    /// ```
    #[must_use]
    pub fn header_text(mut self, text: impl Into<String>) -> Self {
        self.push_text_parameter("header", text);
        self
    }

    /// Appends a body text parameter, merging into the existing body
    /// component so parameters keep their positional order.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    ///
    /// let template = TemplateMessage::new("t", "de").body_text("a").body_text("b");
    /// assert_eq!(template.components.len(), 1);
    /// assert_eq!(template.components[0].parameters().len(), 2);
    /// ```
    #[must_use]
    pub fn body_text(mut self, text: impl Into<String>) -> Self {
        self.push_text_parameter("body", text);
        self
    }

    /// Appends a quick-reply button parameter at `index`, merging with an
    /// existing quick-reply component for the same index.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    ///
    /// let template = TemplateMessage::new("t", "de").button_text(0, "yes");
    /// assert_eq!(template.components[0].kind(), "button");
    /// ```
    #[must_use]
    pub fn button_text(mut self, index: u16, text: impl Into<String>) -> Self {
        let text = text.into();
        let existing = self.components.iter_mut().find(|component| {
            matches!(
                component,
                TemplateComponent::Button {
                    sub_type: TemplateButtonType::QuickReply,
                    index: existing,
                    ..
                } if *existing == index
            )
        });
        if let Some(TemplateComponent::Button { parameters, .. }) = existing {
            parameters.push(TemplateParameter::text(text));
            return self;
        }
        self.components.push(TemplateComponent::button(
            TemplateButtonType::QuickReply,
            index,
            vec![TemplateParameter::text(text)],
        ));
        self
    }

    /// Appends an arbitrary component (media header, URL button, ...).
    ///
    /// ```
    /// use meta_whatsapp::{TemplateComponent, TemplateMessage};
    ///
    /// let template = TemplateMessage::new("t", "de")
    ///     .component(TemplateComponent::body(vec![]));
    /// assert_eq!(template.components.len(), 1);
    /// ```
    #[must_use]
    pub fn component(mut self, component: TemplateComponent) -> Self {
        self.components.push(component);
        self
    }

    /// Merges a text parameter into the component of the same kind, so
    /// repeated builder calls accumulate instead of producing duplicate
    /// components.
    fn push_text_parameter(&mut self, kind: &'static str, text: impl Into<String>) {
        let text = text.into();
        let existing = self
            .components
            .iter_mut()
            .find(|component| component.kind() == kind);
        if let Some(
            TemplateComponent::Header { parameters } | TemplateComponent::Body { parameters },
        ) = existing
        {
            parameters.push(TemplateParameter::text(text));
            return;
        }
        let parameters = vec![TemplateParameter::text(text)];
        self.components.push(if kind == "header" {
            TemplateComponent::header(parameters)
        } else {
            TemplateComponent::body(parameters)
        });
    }

    /// Checks the name, language, component count and every parameter.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    ///
    /// assert!(TemplateMessage::new("hello_world", "en_US").validate().is_ok());
    /// assert!(TemplateMessage::new("", "en").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty name, bad language code or bad
    /// parameter.
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(Error::validation("template.name", "must not be empty"));
        }
        require_max_bytes("template.name", &self.name, TEMPLATE_NAME_MAX_BYTES)?;
        self.language.validate()?;
        if self.components.len() > 3 {
            return Err(Error::validation(
                "template.components",
                "at most three components (header, body, button) are allowed",
            ));
        }
        for component in &self.components {
            component.validate()?;
        }
        Ok(())
    }

    /// Validates and renders the `template` object.
    ///
    /// ```
    /// use meta_whatsapp::TemplateMessage;
    ///
    /// let json = TemplateMessage::new("hello_world", "en_US").render().unwrap();
    /// assert_eq!(json["language"]["code"], "en_US");
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from [`TemplateMessage::validate`].
    pub fn render(&self) -> Result<Value> {
        self.validate()?;
        let mut object = Map::new();
        object.insert("name".into(), Value::String(self.name.clone()));
        object.insert("language".into(), json!(self.language));
        if !self.components.is_empty() {
            object.insert(
                "components".into(),
                Value::Array(
                    self.components
                        .iter()
                        .map(TemplateComponent::render)
                        .collect(),
                ),
            );
        }
        Ok(Value::Object(object))
    }
}

/// Query for `GET /{waba_id}/message_templates`.
///
/// ```
/// use meta_whatsapp::TemplateQuery;
///
/// let query = TemplateQuery::new("102290129340398")
///     .limit(50)
///     .status("APPROVED")
///     .fields(["name", "status"]);
/// assert!(query.query_string().contains("limit=50"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateQuery {
    /// `WhatsApp` Business Account id (digits only).
    pub waba_id: String,
    /// Page size; Meta caps it at 100.
    pub limit: Option<u32>,
    /// Cursor from `paging.next`.
    pub after: Option<String>,
    /// Cursor for the previous page.
    pub before: Option<String>,
    /// e.g. `APPROVED`, `PENDING`, `REJECTED`.
    pub status: Option<String>,
    /// Filter by template name.
    pub name: Option<String>,
    /// Filter by language code.
    pub language: Option<String>,
    /// Fields to include; empty means Meta's default set.
    pub fields: Vec<String>,
}

impl TemplateQuery {
    /// A query for one business account.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert_eq!(TemplateQuery::new("1").waba_id, "1");
    /// ```
    pub fn new(waba_id: impl Into<String>) -> Self {
        Self {
            waba_id: waba_id.into(),
            ..Self::default()
        }
    }

    /// Sets the page size.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert_eq!(TemplateQuery::new("1").limit(10).limit, Some(10));
    /// ```
    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the `after` cursor.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert!(TemplateQuery::new("1").after("abc").after.is_some());
    /// ```
    #[must_use]
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Sets the `before` cursor.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert!(TemplateQuery::new("1").before("abc").before.is_some());
    /// ```
    #[must_use]
    pub fn before(mut self, cursor: impl Into<String>) -> Self {
        self.before = Some(cursor.into());
        self
    }

    /// Filters by status.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert_eq!(TemplateQuery::new("1").status("APPROVED").status.unwrap(), "APPROVED");
    /// ```
    #[must_use]
    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = Some(status.into());
        self
    }

    /// Filters by name.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert!(TemplateQuery::new("1").name("hello").name.is_some());
    /// ```
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Filters by language.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert!(TemplateQuery::new("1").language("de").language.is_some());
    /// ```
    #[must_use]
    pub fn language(mut self, language: impl Into<String>) -> Self {
        self.language = Some(language.into());
        self
    }

    /// Selects the fields to return.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    /// assert_eq!(TemplateQuery::new("1").fields(["id", "name"]).fields.len(), 2);
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

    /// Checks the WABA id and page size.
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    ///
    /// assert!(TemplateQuery::new("102290129340398").validate().is_ok());
    /// assert!(TemplateQuery::new("../me").validate().is_err());
    /// assert!(TemplateQuery::new("1").limit(0).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a non-numeric WABA id or a limit outside
    /// `1..=100`.
    pub fn validate(&self) -> Result<()> {
        if self.waba_id.is_empty() {
            return Err(Error::validation("waba_id", "must not be empty"));
        }
        if self.waba_id.len() > 32 || !self.waba_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::validation("waba_id", "must be digits only"));
        }
        if let Some(limit) = self.limit {
            if !(1..=100).contains(&limit) {
                return Err(Error::validation(
                    "limit",
                    format!("must be within 1..=100, got {limit}"),
                ));
            }
        }
        Ok(())
    }

    /// Renders the query string without the leading `?` (empty when no
    /// filters are set).
    ///
    /// ```
    /// use meta_whatsapp::TemplateQuery;
    ///
    /// assert_eq!(TemplateQuery::new("1").query_string(), "");
    /// assert_eq!(
    ///     TemplateQuery::new("1").limit(5).fields(["name"]).query_string(),
    ///     "limit=5&fields=name"
    /// );
    /// ```
    #[must_use]
    pub fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(limit) = self.limit {
            parts.push(format!("limit={limit}"));
        }
        for (key, value) in [
            ("after", &self.after),
            ("before", &self.before),
            ("status", &self.status),
            ("name", &self.name),
            ("language", &self.language),
        ] {
            if let Some(value) = value {
                parts.push(format!(
                    "{key}={}",
                    utf8_percent_encode(value, QUERY_VALUE_SET)
                ));
            }
        }
        if !self.fields.is_empty() {
            let fields = self
                .fields
                .iter()
                .map(|field| utf8_percent_encode(field, QUERY_VALUE_SET).to_string())
                .collect::<Vec<_>>()
                .join("%2C");
            parts.push(format!("fields={fields}"));
        }
        parts.join("&")
    }
}

/// One page of templates.
///
/// ```
/// use meta_whatsapp::TemplateList;
///
/// let page: TemplateList = serde_json::from_str(
///     r#"{"data":[{"id":"1","name":"hello_world","status":"APPROVED"}],"paging":{"next":"https://graph.facebook.com/next"}}"#,
/// )?;
/// assert_eq!(page.data[0].name, "hello_world");
/// assert!(page.has_next_page());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateList {
    /// Templates on this page.
    #[serde(default)]
    pub data: Vec<TemplateSummary>,
    /// Cursor information, when Meta returned any.
    #[serde(default)]
    pub paging: Option<Paging>,
}

impl TemplateList {
    /// Whether `paging.next` is present — pass its cursor to
    /// [`TemplateQuery::after`] to continue.
    ///
    /// ```
    /// use meta_whatsapp::TemplateList;
    /// assert!(!TemplateList::default().has_next_page());
    /// ```
    #[must_use]
    pub fn has_next_page(&self) -> bool {
        self.paging
            .as_ref()
            .and_then(|paging| paging.next.as_ref())
            .is_some()
    }
}

/// Paging block of a Graph API list response.
///
/// ```
/// use meta_whatsapp::Paging;
///
/// let paging: Paging = serde_json::from_str(
///     r#"{"cursors":{"before":"b","after":"a"},"next":"https://x.test/next"}"#,
/// )?;
/// assert_eq!(paging.cursors.unwrap().after.unwrap(), "a");
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Paging {
    /// Opaque cursors for neighbouring pages.
    #[serde(default)]
    pub cursors: Option<Cursors>,
    /// Absolute URL of the next page.
    #[serde(default)]
    pub next: Option<String>,
    /// Absolute URL of the previous page.
    #[serde(default)]
    pub previous: Option<String>,
}

/// Cursor pair.
///
/// ```
/// let cursors: meta_whatsapp::Cursors = serde_json::from_str(r#"{"after":"x"}"#)?;
/// assert_eq!(cursors.after.as_deref(), Some("x"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursors {
    /// Cursor before this page.
    #[serde(default)]
    pub before: Option<String>,
    /// Cursor after this page.
    #[serde(default)]
    pub after: Option<String>,
}

/// A template as returned by the list endpoint (deliberately permissive: Meta
/// adds fields without notice).
///
/// ```
/// use meta_whatsapp::TemplateSummary;
///
/// let template: TemplateSummary = serde_json::from_str(
///     r#"{"id":"1","name":"hello_world","status":"APPROVED","category":"UTILITY","language":"en_US"}"#,
/// )?;
/// assert_eq!(template.status.as_deref(), Some("APPROVED"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TemplateSummary {
    /// Template id.
    #[serde(default)]
    pub id: String,
    /// Template name.
    #[serde(default)]
    pub name: String,
    /// `APPROVED`, `PENDING`, `REJECTED`, `PAUSED`, ...
    #[serde(default)]
    pub status: Option<String>,
    /// `MARKETING`, `UTILITY`, `AUTHENTICATION`.
    #[serde(default)]
    pub category: Option<String>,
    /// Language code.
    #[serde(default)]
    pub language: Option<String>,
    /// Component definitions, passed through untouched.
    #[serde(default)]
    pub components: Vec<Value>,
}

/// Characters escaped in a query value.
///
/// Everything outside RFC 3986's unreserved set is escaped, which keeps `-`,
/// `.`, `_` and `~` literal and turns a space into `%20` (never `+`, which the
/// Graph API would read as a literal plus).
const QUERY_VALUE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn require_max_bytes(field: &'static str, value: &str, max: usize) -> Result<()> {
    if value.len() > max {
        return Err(Error::validation(
            field,
            format!("must be at most {max} bytes, got {}", value.len()),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(value: &str) -> String {
        utf8_percent_encode(value, QUERY_VALUE_SET).to_string()
    }

    #[test]
    fn query_values_escape_everything_outside_the_unreserved_set() {
        assert_eq!(encoded("a+b/c="), "a%2Bb%2Fc%3D");
        assert_eq!(encoded("safe-_.~"), "safe-_.~");
        assert_eq!(encoded("ä"), "%C3%A4");
        assert_eq!(encoded(""), "");
        assert_eq!(encoded("%"), "%25", "an existing escape is escaped again");
        assert_eq!(encoded(" "), "%20", "a space is never a plus");
        assert_eq!(
            encoded("a&b=c"),
            "a%26b%3Dc",
            "a delimiter cannot split a value"
        );
        assert_eq!(
            encoded("a,b"),
            "a%2Cb",
            "the field-list separator stays unambiguous"
        );
    }

    #[test]
    fn query_string_is_empty_without_filters() {
        assert_eq!(TemplateQuery::new("1").query_string(), "");
    }

    #[test]
    fn repeated_button_text_merges_into_one_component() {
        let template = TemplateMessage::new("t", "de")
            .button_text(0, "a")
            .button_text(0, "b");
        assert_eq!(template.components.len(), 1);
        assert_eq!(template.components[0].parameters().len(), 2);
    }

    #[test]
    fn quick_reply_parameters_render_as_payload() {
        let template = TemplateMessage::new("t", "de").button_text(0, "yes");
        let json = template.render().unwrap();
        assert_eq!(json["components"][0]["parameters"][0]["type"], "payload");
        assert_eq!(json["components"][0]["parameters"][0]["payload"], "yes");
    }

    #[test]
    fn url_button_parameters_render_as_text() {
        let template = TemplateMessage::new("t", "de").component(TemplateComponent::button(
            TemplateButtonType::Url,
            0,
            vec![TemplateParameter::text("x")],
        ));
        let json = template.render().unwrap();
        assert_eq!(json["components"][0]["parameters"][0]["type"], "text");
    }
}
