//! Template messages, listing and management.
//!
//! Templates are pre-approved by Meta. Sending one references its name and
//! language plus the positional parameters for the header, body and buttons;
//! listing them reads `/{waba_id}/message_templates` with cursor paging.
//! [`TemplateDefinition`] and [`TemplateUpdate`] build the definition Meta
//! reviews, for `create_template`, `update_template` and `delete_template`.
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
use crate::message::{require_max_bytes, require_max_chars, require_non_empty};

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

/// Maximum number of digits in a template id.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_ID_MAX_DIGITS;
/// assert_eq!(TEMPLATE_ID_MAX_DIGITS, 32);
/// ```
pub const TEMPLATE_ID_MAX_DIGITS: usize = 32;

/// Maximum authentication-code lifetime Meta accepts in a template footer.
///
/// ```
/// use meta_whatsapp::template::TEMPLATE_CODE_EXPIRATION_MINUTES_MAX;
/// assert_eq!(TEMPLATE_CODE_EXPIRATION_MINUTES_MAX, 90);
/// ```
pub const TEMPLATE_CODE_EXPIRATION_MINUTES_MAX: u16 = 90;

/// Category assigned to a template definition.
///
/// ```
/// use meta_whatsapp::TemplateCategory;
///
/// assert_eq!(TemplateCategory::Utility.as_str(), "UTILITY");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TemplateCategory {
    /// Promotional content.
    #[default]
    Marketing,
    /// Transactional content.
    Utility,
    /// One-time authentication codes.
    Authentication,
}

impl TemplateCategory {
    /// Upper-case wire representation.
    ///
    /// ```
    /// use meta_whatsapp::TemplateCategory;
    /// assert_eq!(TemplateCategory::Authentication.as_str(), "AUTHENTICATION");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Marketing => "MARKETING",
            Self::Utility => "UTILITY",
            Self::Authentication => "AUTHENTICATION",
        }
    }
}

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
                format!("must be at most {TEMPLATE_LANGUAGE_MAX_CHARS} characters, got {count}"),
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
        validate_template_name(&self.name)?;
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
                parts.push(format!("{key}={}", encode_query_value(value)));
            }
        }
        if !self.fields.is_empty() {
            let fields = self
                .fields
                .iter()
                .map(|field| encode_query_value(field))
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

/// Parameter naming style of a template definition.
///
/// ```
/// use meta_whatsapp::template::TemplateParameterFormat;
///
/// assert_eq!(TemplateParameterFormat::default().as_str(), "POSITIONAL");
/// assert_eq!(TemplateParameterFormat::Named.as_str(), "NAMED");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TemplateParameterFormat {
    /// `{{1}}`, `{{2}}` — Meta's default.
    #[default]
    Positional,
    /// `{{order_id}}`: the placeholder names stand in the component text.
    Named,
}
impl TemplateParameterFormat {
    /// Upper-case wire representation.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateParameterFormat;
    /// assert_eq!(TemplateParameterFormat::Positional.as_str(), "POSITIONAL");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positional => "POSITIONAL",
            Self::Named => "NAMED",
        }
    }
}

/// Header format of a template definition.
///
/// The three media formats are the send-side [`TemplateMediaKind`] in Meta's
/// upper-case spelling: [`TemplateHeaderFormat::from`] converts a media kind and
/// [`TemplateHeaderFormat::media_kind`] converts back.
///
/// ```
/// use meta_whatsapp::template::{TemplateHeaderFormat, TemplateMediaKind};
///
/// assert_eq!(TemplateHeaderFormat::Text.as_str(), "TEXT");
/// assert_eq!(TemplateHeaderFormat::from(TemplateMediaKind::Video).as_str(), "VIDEO");
/// assert_eq!(TemplateHeaderFormat::Location.media_kind(), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TemplateHeaderFormat {
    /// Text, optionally with placeholders.
    #[default]
    Text,
    /// Image; needs a handle in `example.header_handle`.
    Image,
    /// Video; needs a handle in `example.header_handle`.
    Video,
    /// Document; needs a handle in `example.header_handle`.
    Document,
    /// Location pin; carries neither text nor example.
    Location,
}

impl TemplateHeaderFormat {
    /// Upper-case wire representation.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateHeaderFormat;
    /// assert_eq!(TemplateHeaderFormat::Location.as_str(), "LOCATION");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "TEXT",
            Self::Image => "IMAGE",
            Self::Video => "VIDEO",
            Self::Document => "DOCUMENT",
            Self::Location => "LOCATION",
        }
    }

    /// The send-side media kind, for a media format.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateHeaderFormat, TemplateMediaKind};
    ///
    /// assert_eq!(
    ///     TemplateHeaderFormat::Document.media_kind(),
    ///     Some(TemplateMediaKind::Document)
    /// );
    /// assert_eq!(TemplateHeaderFormat::Text.media_kind(), None);
    /// ```
    #[must_use]
    pub fn media_kind(self) -> Option<TemplateMediaKind> {
        match self {
            Self::Image => Some(TemplateMediaKind::Image),
            Self::Video => Some(TemplateMediaKind::Video),
            Self::Document => Some(TemplateMediaKind::Document),
            Self::Text | Self::Location => None,
        }
    }
}

impl From<TemplateMediaKind> for TemplateHeaderFormat {
    fn from(kind: TemplateMediaKind) -> Self {
        match kind {
            TemplateMediaKind::Image => Self::Image,
            TemplateMediaKind::Video => Self::Video,
            TemplateMediaKind::Document => Self::Document,
        }
    }
}

/// How an authentication template delivers its one-time code.
///
/// ```
/// use meta_whatsapp::template::TemplateOtpType;
///
/// assert_eq!(TemplateOtpType::default().as_str(), "COPY_CODE");
/// assert_eq!(TemplateOtpType::OneTap.as_str(), "ONE_TAP");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum TemplateOtpType {
    /// The customer copies the code out of the message.
    #[default]
    CopyCode,
    /// Android autofills the code into the requesting app.
    OneTap,
}

impl TemplateOtpType {
    /// Upper-case wire representation.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateOtpType;
    /// assert_eq!(TemplateOtpType::CopyCode.as_str(), "COPY_CODE");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CopyCode => "COPY_CODE",
            Self::OneTap => "ONE_TAP",
        }
    }
}

/// A text parameter doubles as a template example value.
///
/// Examples are text only — the create payload's `example.header_text` and
/// `example.body_text` both take strings — so this conversion exists for
/// ergonomics, not for the currency/date-time/media variants.
///
/// ```
/// use meta_whatsapp::TemplateParameter;
///
/// let parameter: TemplateParameter = "Florian".into();
/// assert_eq!(parameter, TemplateParameter::text("Florian"));
/// ```
impl From<&str> for TemplateParameter {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

/// An owned string becomes a text parameter, so `format!`-built examples work
/// without an extra `TemplateParameter::text` call.
///
/// ```
/// use meta_whatsapp::TemplateParameter;
///
/// let parameter: TemplateParameter = format!("Order {}", 42).into();
/// assert_eq!(parameter, TemplateParameter::text("Order 42"));
/// ```
impl From<String> for TemplateParameter {
    fn from(text: String) -> Self {
        Self::Text { text }
    }
}

/// One button of a template *definition*.
///
/// Sending a template later uses [`TemplateComponent::button`] with the
/// send-side [`TemplateButtonType`]; a definition instead carries the label and
/// destination Meta reviews.
///
/// ```
/// use meta_whatsapp::template::TemplateButton;
///
/// let url = TemplateButton::url("Sendung verfolgen", "https://x.test/{{1}}")
///     .with_example("https://x.test/1234");
/// assert_eq!(url.kind(), "URL");
/// assert!(url.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateButton {
    /// A reply the customer taps to send its text back.
    QuickReply {
        /// Label shown on the button.
        text: String,
    },
    /// Opens a URL, optionally with one `{{1}}` variable.
    Url {
        /// Label shown on the button.
        text: String,
        /// Destination (`http://` or `https://`, may contain `{{1}}`).
        url: String,
        /// Sample destination; required when `url` contains a variable.
        example: Option<String>,
    },
    /// Copies a code into the clipboard.
    CopyCode {
        /// Label shown on the button.
        text: String,
        /// Sample code the customer sees in the preview.
        example: String,
    },
    /// Starts a phone call.
    PhoneNumber {
        /// Label shown on the button.
        text: String,
        /// E.164 number with an optional leading `+`.
        phone_number: String,
    },
    /// An authentication button that delivers the one-time code.
    Otp {
        /// Delivery mode.
        otp_type: TemplateOtpType,
        /// Label shown on the button.
        text: Option<String>,
        /// Label for the autofill prompt (one-tap).
        autofill_text: Option<String>,
    },
}

impl TemplateButton {
    /// A quick-reply button.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// assert!(TemplateButton::quick_reply("Ja").validate().is_ok());
    /// ```
    #[must_use]
    pub fn quick_reply(text: impl Into<String>) -> Self {
        Self::QuickReply { text: text.into() }
    }

    /// A URL button without an example (valid while the URL has no variable).
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// assert!(TemplateButton::url("Web", "https://x.test").validate().is_ok());
    /// ```
    #[must_use]
    pub fn url(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self::Url {
            text: text.into(),
            url: url.into(),
            example: None,
        }
    }

    /// A copy-code button, whose example is the code preview Meta shows.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// let button = TemplateButton::copy_code("Code kopieren", "ABC123");
    /// assert!(button.validate().is_ok());
    /// ```
    #[must_use]
    pub fn copy_code(text: impl Into<String>, example: impl Into<String>) -> Self {
        Self::CopyCode {
            text: text.into(),
            example: example.into(),
        }
    }

    /// A call button.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// assert!(TemplateButton::phone_number("Anrufen", "+4915112345678").validate().is_ok());
    /// ```
    #[must_use]
    pub fn phone_number(text: impl Into<String>, phone_number: impl Into<String>) -> Self {
        Self::PhoneNumber {
            text: text.into(),
            phone_number: phone_number.into(),
        }
    }

    /// An OTP button for an authentication template.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateButton, TemplateOtpType};
    ///
    /// let button = TemplateButton::otp(TemplateOtpType::CopyCode).with_text("Code kopieren");
    /// assert!(button.validate().is_ok());
    /// ```
    #[must_use]
    pub fn otp(otp_type: TemplateOtpType) -> Self {
        Self::Otp {
            otp_type,
            text: None,
            autofill_text: None,
        }
    }

    /// Sets the example of a URL or copy-code button; a no-op for the other
    /// kinds, which carry no example.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// let button = TemplateButton::url("Web", "https://x.test/{{1}}")
    ///     .with_example("https://x.test/1");
    /// assert!(button.validate().is_ok());
    /// ```
    #[must_use]
    pub fn with_example(mut self, example: impl Into<String>) -> Self {
        let example = example.into();
        match &mut self {
            Self::Url { example: slot, .. } => *slot = Some(example),
            Self::CopyCode { example: slot, .. } => *slot = example,
            _ => {}
        }
        self
    }

    /// Sets the label of an OTP button; a no-op for the other kinds, whose
    /// label is set by their constructor.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateButton, TemplateOtpType};
    ///
    /// let button = TemplateButton::otp(TemplateOtpType::OneTap).with_text("Autofill");
    /// assert!(matches!(button, TemplateButton::Otp { text: Some(_), .. }));
    /// ```
    #[must_use]
    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        if let Self::Otp { text: slot, .. } = &mut self {
            *slot = Some(text.into());
        }
        self
    }

    /// Sets the autofill label of an OTP button; a no-op for the other kinds.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateButton, TemplateOtpType};
    ///
    /// let button = TemplateButton::otp(TemplateOtpType::OneTap)
    ///     .with_autofill_text("Code einsetzen");
    /// assert!(button.validate().is_ok());
    /// ```
    #[must_use]
    pub fn with_autofill_text(mut self, text: impl Into<String>) -> Self {
        if let Self::Otp { autofill_text, .. } = &mut self {
            *autofill_text = Some(text.into());
        }
        self
    }

    /// Upper-case wire type: `QUICK_REPLY`, `URL`, `COPY_CODE`,
    /// `PHONE_NUMBER` or `OTP`.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// assert_eq!(TemplateButton::quick_reply("Ja").kind(), "QUICK_REPLY");
    /// assert_eq!(TemplateButton::copy_code("Code", "ABC").kind(), "COPY_CODE");
    /// ```
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::QuickReply { .. } => "QUICK_REPLY",
            Self::Url { .. } => "URL",
            Self::CopyCode { .. } => "COPY_CODE",
            Self::PhoneNumber { .. } => "PHONE_NUMBER",
            Self::Otp { .. } => "OTP",
        }
    }

    /// Checks the label, the destination and the example.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateButton;
    ///
    /// assert!(TemplateButton::url("Web", "ftp://x.test").validate().is_err());
    /// assert!(TemplateButton::quick_reply("x".repeat(26)).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or over-long label, a URL that is not
    /// `http(s)` or has a variable but no example, a copy-code example outside
    /// `1..=15` characters, a phone number that is not 7–15 digits, or an OTP
    /// button without any label.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::QuickReply { text } => {
                validate_button_text("template.components.buttons.text", text)
            }
            Self::Url { text, url, example } => {
                validate_button_text("template.components.buttons.text", text)?;
                require_non_empty("template.components.buttons.url", url)?;
                require_max_chars(
                    "template.components.buttons.url",
                    url,
                    TEMPLATE_BUTTON_URL_MAX_CHARS,
                )?;
                if !(url.starts_with("https://") || url.starts_with("http://")) {
                    return Err(Error::validation(
                        "template.components.buttons.url",
                        "must be an http:// or https:// URL",
                    ));
                }
                match example {
                    Some(example) => {
                        require_non_empty("template.components.buttons.example", example)?;
                        require_max_chars(
                            "template.components.buttons.example",
                            example,
                            TEMPLATE_BUTTON_URL_MAX_CHARS,
                        )?;
                    }
                    None if url.contains("{{") => {
                        return Err(Error::validation(
                            "template.components.buttons.example",
                            "a URL with a variable needs an example value",
                        ));
                    }
                    None => {}
                }
                Ok(())
            }
            Self::CopyCode { text, example } => {
                validate_button_text("template.components.buttons.text", text)?;
                require_non_empty("template.components.buttons.example", example)?;
                require_max_chars(
                    "template.components.buttons.example",
                    example,
                    TEMPLATE_BUTTON_EXAMPLE_MAX_CHARS,
                )
            }
            Self::PhoneNumber { text, phone_number } => {
                validate_button_text("template.components.buttons.text", text)?;
                validate_button_phone_number(phone_number)
            }
            Self::Otp {
                text,
                autofill_text,
                ..
            } => {
                for label in [text, autofill_text].into_iter().flatten() {
                    require_non_empty("template.components.buttons.text", label)?;
                    require_max_chars(
                        "template.components.buttons.text",
                        label,
                        TEMPLATE_BUTTON_TEXT_MAX_CHARS,
                    )?;
                }
                if text.is_none() && autofill_text.is_none() {
                    return Err(Error::validation(
                        "template.components.buttons.text",
                        "an OTP button needs a text or an autofill_text",
                    ));
                }
                Ok(())
            }
        }
    }

    fn render(&self) -> Value {
        match self {
            Self::QuickReply { text } => json!({ "type": "QUICK_REPLY", "text": text }),
            Self::Url { text, url, example } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("URL"));
                object.insert("text".into(), json!(text));
                object.insert("url".into(), json!(url));
                if let Some(example) = example {
                    object.insert("example".into(), json!([example]));
                }
                Value::Object(object)
            }
            Self::CopyCode { text, example } => {
                json!({ "type": "COPY_CODE", "text": text, "example": example })
            }
            Self::PhoneNumber { text, phone_number } => json!({
                "type": "PHONE_NUMBER",
                "text": text,
                "phone_number": phone_number,
            }),
            Self::Otp {
                otp_type,
                text,
                autofill_text,
            } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("OTP"));
                object.insert("otp_type".into(), json!(otp_type.as_str()));
                if let Some(text) = text {
                    object.insert("text".into(), json!(text));
                }
                if let Some(autofill_text) = autofill_text {
                    object.insert("autofill_text".into(), json!(autofill_text));
                }
                Value::Object(object)
            }
        }
    }
}

/// Checks a template button label.
fn validate_button_text(field: &'static str, text: &str) -> Result<()> {
    require_non_empty(field, text)?;
    require_max_chars(field, text, TEMPLATE_BUTTON_TEXT_MAX_CHARS)
}

/// Checks a call-button number: 7–15 digits with an optional leading `+`.
fn validate_button_phone_number(value: &str) -> Result<()> {
    let digits = value.strip_prefix('+').unwrap_or(value);
    let valid = (PHONE_NUMBER_MIN_DIGITS..=PHONE_NUMBER_MAX_DIGITS).contains(&digits.len())
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(Error::validation(
            "template.components.buttons.phone_number",
            format!(
                "must be {PHONE_NUMBER_MIN_DIGITS}..={PHONE_NUMBER_MAX_DIGITS} digits with an optional leading '+', got {value:?}"
            ),
        ))
    }
}

/// Shortest E.164 subscriber number (the same range the blocklist accepts).
const PHONE_NUMBER_MIN_DIGITS: usize = 7;

/// Longest E.164 subscriber number.
const PHONE_NUMBER_MAX_DIGITS: usize = 15;

/// One component of a template *definition*.
///
/// A definition is what Meta reviews: component text with `{{1}}` placeholders
/// plus one example value per placeholder. [`TemplateComponent`] is the
/// send-side twin that carries the positional values, so the kinds line up
/// (`HEADER`, `BODY`, `FOOTER`, `BUTTONS`) while the payloads differ.
///
/// ```
/// use meta_whatsapp::template::{TemplateDefinitionComponent, TemplateParameter};
///
/// let body = TemplateDefinitionComponent::body_text("Hallo {{1}}")
///     .with_example(TemplateParameter::text("Florian"));
/// assert_eq!(body.kind(), "BODY");
/// assert!(body.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateDefinitionComponent {
    /// `HEADER`: text with an example, or one media handle.
    Header {
        /// Header format.
        format: TemplateHeaderFormat,
        /// Text of a [`TemplateHeaderFormat::Text`] header.
        text: Option<String>,
        /// Example values: `header_text` for text, `header_handle` for media.
        example: Vec<TemplateParameter>,
    },
    /// `BODY`: the message text, or the security recommendation of an
    /// authentication template.
    Body {
        /// Text with placeholders.
        text: Option<String>,
        /// `add_security_recommendation` instead of a text (authentication).
        security_recommendation: bool,
        /// Example values (`body_text`).
        example: Vec<TemplateParameter>,
    },
    /// `FOOTER`: one short line, or the lifetime of an authentication code.
    Footer {
        /// Footer text.
        text: Option<String>,
        /// `code_expiration_minutes` (authentication templates).
        code_expiration_minutes: Option<u16>,
    },
    /// `BUTTONS`: the buttons below the message.
    Buttons {
        /// Buttons in display order.
        buttons: Vec<TemplateButton>,
    },
}

impl TemplateDefinitionComponent {
    /// A text header.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// let header = TemplateDefinitionComponent::header_text("Hallo {{1}}").with_example("Florian");
    /// assert!(header.validate().is_ok());
    /// ```
    #[must_use]
    pub fn header_text(text: impl Into<String>) -> Self {
        Self::Header {
            format: TemplateHeaderFormat::Text,
            text: Some(text.into()),
            example: Vec::new(),
        }
    }

    /// A media header with the handle Meta gave the upload.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateDefinitionComponent, TemplateHeaderFormat};
    ///
    /// let header = TemplateDefinitionComponent::header_media(TemplateHeaderFormat::Image, "4:handle");
    /// assert!(header.validate().is_ok());
    /// ```
    #[must_use]
    pub fn header_media(format: TemplateHeaderFormat, handle: impl Into<String>) -> Self {
        Self::Header {
            format,
            text: None,
            example: vec![TemplateParameter::text(handle)],
        }
    }

    /// A body with `{{1}}`-style placeholders.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// let body = TemplateDefinitionComponent::body_text("Ihre Bestellung {{1}}").with_example("42");
    /// assert!(body.validate().is_ok());
    /// ```
    #[must_use]
    pub fn body_text(text: impl Into<String>) -> Self {
        Self::Body {
            text: Some(text.into()),
            security_recommendation: false,
            example: Vec::new(),
        }
    }

    /// The body of an authentication template: Meta inserts the security
    /// recommendation instead of a text.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// let body = TemplateDefinitionComponent::body_security_recommendation();
    /// assert!(body.validate().is_ok());
    /// ```
    #[must_use]
    pub fn body_security_recommendation() -> Self {
        Self::Body {
            text: None,
            security_recommendation: true,
            example: Vec::new(),
        }
    }

    /// A footer line.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// assert!(TemplateDefinitionComponent::footer_text("Danke").validate().is_ok());
    /// ```
    #[must_use]
    pub fn footer_text(text: impl Into<String>) -> Self {
        Self::Footer {
            text: Some(text.into()),
            code_expiration_minutes: None,
        }
    }

    /// The footer of an authentication template: how long the code stays valid.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// assert!(TemplateDefinitionComponent::footer_code_expiration(10).validate().is_ok());
    /// ```
    #[must_use]
    pub fn footer_code_expiration(minutes: u16) -> Self {
        Self::Footer {
            text: None,
            code_expiration_minutes: Some(minutes),
        }
    }

    /// A `BUTTONS` component.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateButton, TemplateDefinitionComponent};
    ///
    /// let buttons = TemplateDefinitionComponent::buttons(vec![TemplateButton::quick_reply("Ja")]);
    /// assert_eq!(buttons.kind(), "BUTTONS");
    /// ```
    #[must_use]
    pub fn buttons(buttons: Vec<TemplateButton>) -> Self {
        Self::Buttons { buttons }
    }

    /// Appends an example value to a header or body; a no-op for a footer or a
    /// button group, which carry no example.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// let body = TemplateDefinitionComponent::body_text("{{1}} + {{2}}")
    ///     .with_example("1")
    ///     .with_example("2");
    /// assert!(body.validate().is_ok());
    /// ```
    #[must_use]
    pub fn with_example(mut self, example: impl Into<TemplateParameter>) -> Self {
        let example = example.into();
        match &mut self {
            Self::Header {
                example: values, ..
            }
            | Self::Body {
                example: values, ..
            } => {
                values.push(example);
            }
            _ => {}
        }
        self
    }

    fn push_example(&mut self, example: TemplateParameter) {
        match self {
            Self::Header {
                example: values, ..
            }
            | Self::Body {
                example: values, ..
            } => {
                values.push(example);
            }
            _ => {}
        }
    }

    fn set_header_text(&mut self, value: String) {
        if let Self::Header { format, text, .. } = self {
            *format = TemplateHeaderFormat::Text;
            *text = Some(value);
        }
    }

    fn set_body_text(&mut self, value: String) {
        if let Self::Body {
            text,
            security_recommendation,
            ..
        } = self
        {
            *text = Some(value);
            *security_recommendation = false;
        }
    }

    fn push_button(&mut self, button: TemplateButton) {
        if let Self::Buttons { buttons } = self {
            buttons.push(button);
        }
    }

    /// Upper-case wire type: `HEADER`, `BODY`, `FOOTER` or `BUTTONS`.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// assert_eq!(TemplateDefinitionComponent::body_text("x").kind(), "BODY");
    /// assert_eq!(
    ///     TemplateDefinitionComponent::footer_text("f").kind(),
    ///     "FOOTER"
    /// );
    /// ```
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Header { .. } => "HEADER",
            Self::Body { .. } => "BODY",
            Self::Footer { .. } => "FOOTER",
            Self::Buttons { .. } => "BUTTONS",
        }
    }

    /// Checks the text, the example values and any nested button.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    ///
    /// // A placeholder without an example is rejected by Meta, so it is
    /// // rejected here and never reaches the network.
    /// assert!(TemplateDefinitionComponent::body_text("Hallo {{1}}").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the offending field for a missing or
    /// over-long text, a placeholder without an example, a non-text example, a
    /// media header without exactly one handle, a footer with both or neither
    /// variant, and for too many or invalid buttons.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Header {
                format,
                text,
                example,
            } => validate_header(*format, text.as_deref(), example),
            Self::Body {
                text,
                security_recommendation,
                example,
            } => validate_body(text.as_deref(), *security_recommendation, example),
            Self::Footer {
                text,
                code_expiration_minutes,
            } => validate_footer(text.as_deref(), *code_expiration_minutes),
            Self::Buttons { buttons } => validate_buttons(buttons),
        }
    }

    fn render(&self) -> Value {
        match self {
            Self::Header {
                format,
                text,
                example,
            } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("HEADER"));
                object.insert("format".into(), json!(format.as_str()));
                if let Some(text) = text {
                    object.insert("text".into(), json!(text));
                }
                if !example.is_empty() {
                    let key = if format.media_kind().is_some() {
                        "header_handle"
                    } else {
                        "header_text"
                    };
                    let mut values = Map::new();
                    values.insert(key.to_string(), json!(example_values(example)));
                    object.insert("example".into(), Value::Object(values));
                }
                Value::Object(object)
            }
            Self::Body {
                text,
                security_recommendation,
                example,
            } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("BODY"));
                if let Some(text) = text {
                    object.insert("text".into(), json!(text));
                }
                if *security_recommendation {
                    object.insert("add_security_recommendation".into(), json!(true));
                }
                if !example.is_empty() {
                    object.insert(
                        "example".into(),
                        json!({ "body_text": [example_values(example)] }),
                    );
                }
                Value::Object(object)
            }
            Self::Footer {
                text,
                code_expiration_minutes,
            } => {
                let mut object = Map::new();
                object.insert("type".into(), json!("FOOTER"));
                if let Some(text) = text {
                    object.insert("text".into(), json!(text));
                }
                if let Some(minutes) = code_expiration_minutes {
                    object.insert("code_expiration_minutes".into(), json!(minutes));
                }
                Value::Object(object)
            }
            Self::Buttons { buttons } => json!({
                "type": "BUTTONS",
                "buttons": buttons.iter().map(TemplateButton::render).collect::<Vec<_>>(),
            }),
        }
    }
}

/// Checks a header component: text with examples, one media handle, or a bare
/// location pin.
fn validate_header(
    format: TemplateHeaderFormat,
    text: Option<&str>,
    example: &[TemplateParameter],
) -> Result<()> {
    validate_examples("template.components.header.example", example)?;
    match format.media_kind() {
        Some(_) => {
            if text.is_some() {
                return Err(Error::validation(
                    "template.components.header.text",
                    "a media header carries no text",
                ));
            }
            if example.len() != 1 {
                return Err(Error::validation(
                    "template.components.header.example",
                    format!(
                        "a {} header needs exactly one handle, got {}",
                        format.as_str(),
                        example.len()
                    ),
                ));
            }
            Ok(())
        }
        None if matches!(format, TemplateHeaderFormat::Location) => {
            if text.is_some() || !example.is_empty() {
                return Err(Error::validation(
                    "template.components.header",
                    "a location header carries neither text nor example",
                ));
            }
            Ok(())
        }
        None => {
            let value = text.unwrap_or_default();
            require_non_empty("template.components.header.text", value)?;
            require_max_chars(
                "template.components.header.text",
                value,
                TEMPLATE_HEADER_MAX_CHARS,
            )?;
            require_examples_for_placeholders("template.components.header.text", value, example)
        }
    }
}

/// Checks a body component: text with examples, or Meta's security
/// recommendation.
fn validate_body(
    text: Option<&str>,
    security_recommendation: bool,
    example: &[TemplateParameter],
) -> Result<()> {
    validate_examples("template.components.body.example", example)?;
    if security_recommendation {
        if text.is_some_and(|text| !text.is_empty()) {
            return Err(Error::validation(
                "template.components.body",
                "either a text or add_security_recommendation, not both",
            ));
        }
        if !example.is_empty() {
            return Err(Error::validation(
                "template.components.body.example",
                "the security recommendation carries no example",
            ));
        }
        return Ok(());
    }
    let value = text.unwrap_or_default();
    require_non_empty("template.components.body.text", value)?;
    require_max_bytes(
        "template.components.body.text",
        value,
        TEMPLATE_BODY_MAX_BYTES,
    )?;
    require_examples_for_placeholders("template.components.body.text", value, example)
}

/// Checks a footer component: a text, or the lifetime of an authentication
/// code.
fn validate_footer(text: Option<&str>, code_expiration_minutes: Option<u16>) -> Result<()> {
    match (text, code_expiration_minutes) {
        (Some(text), None) => {
            require_non_empty("template.components.footer.text", text)?;
            require_max_chars(
                "template.components.footer.text",
                text,
                TEMPLATE_FOOTER_MAX_CHARS,
            )
        }
        (None, Some(minutes)) => {
            if !(1..=TEMPLATE_CODE_EXPIRATION_MINUTES_MAX).contains(&minutes) {
                return Err(Error::validation(
                    "template.components.footer.code_expiration_minutes",
                    format!(
                        "must be within 1..={TEMPLATE_CODE_EXPIRATION_MINUTES_MAX}, got {minutes}"
                    ),
                ));
            }
            Ok(())
        }
        (Some(_), Some(_)) => Err(Error::validation(
            "template.components.footer",
            "either a text or code_expiration_minutes, not both",
        )),
        (None, None) => Err(Error::validation(
            "template.components.footer",
            "needs a text or code_expiration_minutes",
        )),
    }
}

/// Checks the example list of a header or body component.
fn validate_examples(field: &'static str, example: &[TemplateParameter]) -> Result<()> {
    if example.len() > TEMPLATE_EXAMPLES_MAX {
        return Err(Error::validation(
            field,
            format!(
                "at most {TEMPLATE_EXAMPLES_MAX} example values are allowed, got {}",
                example.len()
            ),
        ));
    }
    for parameter in example {
        if !matches!(parameter, TemplateParameter::Text { .. }) {
            return Err(Error::validation(
                field,
                "only text example values are supported",
            ));
        }
        parameter.validate()?;
    }
    Ok(())
}

/// A text with `{{…}}` placeholders needs at least one example value per
/// placeholder, and examples are only meaningful next to a placeholder.
fn require_examples_for_placeholders(
    field: &'static str,
    text: &str,
    example: &[TemplateParameter],
) -> Result<()> {
    let placeholders = text.matches("{{").count();
    if placeholders == 0 {
        if example.is_empty() {
            return Ok(());
        }
        return Err(Error::validation(
            field,
            "an example value needs a placeholder in the text",
        ));
    }
    if example.is_empty() {
        return Err(Error::validation(
            field,
            format!("{placeholders} placeholder(s) need an example value"),
        ));
    }
    if example.len() > placeholders {
        return Err(Error::validation(
            field,
            format!(
                "at most {placeholders} example value(s) match the placeholders, got {}",
                example.len()
            ),
        ));
    }
    Ok(())
}

/// Checks the button list of a `BUTTONS` component.
fn validate_buttons(buttons: &[TemplateButton]) -> Result<()> {
    if buttons.is_empty() {
        return Err(Error::validation(
            "template.components.buttons",
            "need at least one button",
        ));
    }
    if buttons.len() > TEMPLATE_BUTTONS_MAX {
        return Err(Error::validation(
            "template.components.buttons",
            format!(
                "at most {TEMPLATE_BUTTONS_MAX} buttons are allowed, got {}",
                buttons.len()
            ),
        ));
    }
    let urls = buttons
        .iter()
        .filter(|button| matches!(button, TemplateButton::Url { .. }))
        .count();
    if urls > TEMPLATE_URL_BUTTONS_MAX {
        return Err(Error::validation(
            "template.components.buttons",
            format!("at most {TEMPLATE_URL_BUTTONS_MAX} URL buttons are allowed, got {urls}"),
        ));
    }
    let copy_codes = buttons
        .iter()
        .filter(|button| matches!(button, TemplateButton::CopyCode { .. }))
        .count();
    if copy_codes > TEMPLATE_COPY_CODE_BUTTONS_MAX {
        return Err(Error::validation(
            "template.components.buttons",
            format!(
                "at most {TEMPLATE_COPY_CODE_BUTTONS_MAX} copy-code button is allowed, got {copy_codes}"
            ),
        ));
    }
    for button in buttons {
        button.validate()?;
    }
    Ok(())
}

/// The example texts of a component, in placeholder order.
fn example_values(example: &[TemplateParameter]) -> Vec<&str> {
    example
        .iter()
        .filter_map(|parameter| match parameter {
            TemplateParameter::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Checks a template name: non-empty and at most
/// [`TEMPLATE_NAME_MAX_BYTES`] bytes.
pub(crate) fn validate_template_name(name: &str) -> Result<()> {
    require_non_empty("template.name", name)?;
    require_max_bytes("template.name", name, TEMPLATE_NAME_MAX_BYTES)
}

/// Checks an id that becomes a URL path segment of the template endpoints.
pub(crate) fn validate_template_owner_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::validation("phone_number_id", "must not be empty"));
    }
    if id.len() > TEMPLATE_ID_MAX_DIGITS || !id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::validation(
            "phone_number_id",
            format!("must be digits only (at most {TEMPLATE_ID_MAX_DIGITS})"),
        ));
    }
    Ok(())
}

/// A template definition for `POST /{phone_number_id}/message_templates`.
///
/// A definition is the reviewable shape Meta stores: name, language, category
/// and the component texts with their example values. Sending the approved
/// template later is [`TemplateMessage`], which carries the values instead.
///
/// The category-specific review rules (an OTP button only in an
/// `AUTHENTICATION` template, a media header only with an uploaded handle) are
/// Meta's, not local checks: what this crate rejects is what would be malformed
/// on the wire, so a well-formed definition that Meta dislikes fails with
/// [`Error::Api`] and a message.
///
/// ```
/// use meta_whatsapp::{TemplateButton, TemplateCategory, TemplateDefinition};
///
/// let definition = TemplateDefinition::new("order_update", "de_DE", TemplateCategory::Utility)
///     .header_text("Ihre Bestellung {{1}}")
///     .header_example("1234")
///     .body_text("Hallo {{1}}, die Bestellung {{2}} ist unterwegs.")
///     .body_example("Florian")
///     .body_example("1234")
///     .footer_text("Danke für Ihren Einkauf")
///     .button(TemplateButton::quick_reply("Alles klar"));
///
/// let payload = definition.render()?;
/// assert_eq!(payload["language"], "de_DE");
/// assert_eq!(payload["category"], "UTILITY");
/// assert_eq!(payload["components"][0]["format"], "TEXT");
/// assert_eq!(payload["components"][1]["example"]["body_text"][0][1], "1234");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateDefinition {
    /// Name Meta references the template by, and the key of an edit.
    pub name: String,
    /// Language this definition is for.
    pub language: TemplateLanguage,
    /// Category Meta reviews the template against.
    pub category: TemplateCategory,
    /// Placeholder style (`POSITIONAL` unless set otherwise).
    pub parameter_format: TemplateParameterFormat,
    /// Components in render order.
    pub components: Vec<TemplateDefinitionComponent>,
}

impl TemplateDefinition {
    /// A definition with no components — invalid until a body is added.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("hello_world", "en_US", TemplateCategory::Marketing);
    /// assert_eq!(definition.category.as_str(), "MARKETING");
    /// assert!(definition.validate().is_err(), "a body is still missing");
    /// ```
    pub fn new(
        name: impl Into<String>,
        language_code: impl Into<String>,
        category: TemplateCategory,
    ) -> Self {
        Self {
            name: name.into(),
            language: TemplateLanguage::new(language_code),
            category,
            parameter_format: TemplateParameterFormat::default(),
            components: Vec::new(),
        }
    }

    /// Sets the placeholder style.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateParameterFormat;
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .parameter_format(TemplateParameterFormat::Named);
    /// assert_eq!(definition.parameter_format.as_str(), "NAMED");
    /// ```
    #[must_use]
    pub fn parameter_format(mut self, format: TemplateParameterFormat) -> Self {
        self.parameter_format = format;
        self
    }

    /// Appends a component verbatim; a duplicate kind is rejected by
    /// [`TemplateDefinition::validate`].
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDefinitionComponent;
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .component(TemplateDefinitionComponent::footer_text("Danke"));
    /// assert_eq!(definition.components.len(), 1);
    /// ```
    #[must_use]
    pub fn component(mut self, component: TemplateDefinitionComponent) -> Self {
        self.components.push(component);
        self
    }

    /// Sets the text header, replacing a media header of the same definition.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .header_text("Neuigkeiten");
    /// assert_eq!(definition.components[0].kind(), "HEADER");
    /// ```
    #[must_use]
    pub fn header_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        match self.component_index("HEADER") {
            Some(index) => self.components[index].set_header_text(text),
            None => self
                .components
                .push(TemplateDefinitionComponent::header_text(text)),
        }
        self
    }

    /// Sets a media header (image, video or document) with the handle Meta gave
    /// the upload.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateHeaderFormat;
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .header_media(TemplateHeaderFormat::Image, "4:handle");
    /// assert!(definition.components[0].validate().is_ok());
    /// ```
    #[must_use]
    pub fn header_media(mut self, format: TemplateHeaderFormat, handle: impl Into<String>) -> Self {
        let header = TemplateDefinitionComponent::header_media(format, handle);
        match self.component_index("HEADER") {
            Some(index) => self.components[index] = header,
            None => self.components.push(header),
        }
        self
    }

    /// Appends a header example value.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .header_text("Hallo {{1}}")
    ///     .header_example("Florian");
    /// assert!(definition.components[0].validate().is_ok());
    /// ```
    #[must_use]
    pub fn header_example(mut self, example: impl Into<TemplateParameter>) -> Self {
        let example = example.into();
        match self.component_index("HEADER") {
            Some(index) => self.components[index].push_example(example),
            None => self.components.push(TemplateDefinitionComponent::Header {
                format: TemplateHeaderFormat::Text,
                text: None,
                example: vec![example],
            }),
        }
        self
    }

    /// Sets the body text, replacing a security-recommendation body.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .body_text("Danke für Ihren Einkauf");
    /// assert!(definition.validate().is_ok());
    /// ```
    #[must_use]
    pub fn body_text(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        match self.component_index("BODY") {
            Some(index) => self.components[index].set_body_text(text),
            None => self
                .components
                .push(TemplateDefinitionComponent::body_text(text)),
        }
        self
    }

    /// Replaces the body with Meta's security recommendation (authentication
    /// templates only).
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("otp_login", "de", TemplateCategory::Authentication)
    ///     .security_recommendation();
    /// assert!(definition.validate().is_ok());
    /// ```
    #[must_use]
    pub fn security_recommendation(mut self) -> Self {
        let body = TemplateDefinitionComponent::body_security_recommendation();
        match self.component_index("BODY") {
            Some(index) => self.components[index] = body,
            None => self.components.push(body),
        }
        self
    }

    /// Appends a body example value.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .body_text("Hallo {{1}}")
    ///     .body_example("Florian");
    /// assert!(definition.validate().is_ok());
    /// ```
    #[must_use]
    pub fn body_example(mut self, example: impl Into<TemplateParameter>) -> Self {
        let example = example.into();
        match self.component_index("BODY") {
            Some(index) => self.components[index].push_example(example),
            None => self.components.push(TemplateDefinitionComponent::Body {
                text: None,
                security_recommendation: false,
                example: vec![example],
            }),
        }
        self
    }

    /// Sets the footer text, replacing a code-expiration footer.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .body_text("Danke")
    ///     .footer_text("Kein Reply nötig");
    /// assert!(definition.validate().is_ok());
    /// ```
    #[must_use]
    pub fn footer_text(mut self, text: impl Into<String>) -> Self {
        let footer = TemplateDefinitionComponent::footer_text(text);
        match self.component_index("FOOTER") {
            Some(index) => self.components[index] = footer,
            None => self.components.push(footer),
        }
        self
    }

    /// Sets how long an authentication code stays valid (`1..=90` minutes).
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("otp_login", "de", TemplateCategory::Authentication)
    ///     .security_recommendation()
    ///     .code_expiration_minutes(10);
    /// assert!(definition.validate().is_ok());
    /// assert!(definition.clone().code_expiration_minutes(91).validate().is_err());
    /// ```
    #[must_use]
    pub fn code_expiration_minutes(mut self, minutes: u16) -> Self {
        let footer = TemplateDefinitionComponent::footer_code_expiration(minutes);
        match self.component_index("FOOTER") {
            Some(index) => self.components[index] = footer,
            None => self.components.push(footer),
        }
        self
    }

    /// Appends a button, creating the `BUTTONS` component on first use.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateButton, TemplateCategory, TemplateDefinition};
    ///
    /// let definition = TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .body_text("Danke")
    ///     .button(TemplateButton::quick_reply("Ja"))
    ///     .button(TemplateButton::quick_reply("Nein"));
    /// assert!(definition.validate().is_ok());
    /// ```
    #[must_use]
    pub fn button(mut self, button: TemplateButton) -> Self {
        match self.component_index("BUTTONS") {
            Some(index) => self.components[index].push_button(button),
            None => self
                .components
                .push(TemplateDefinitionComponent::buttons(vec![button])),
        }
        self
    }

    /// Checks the name, the language, the component set and every component.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// assert!(TemplateDefinition::new("t", "de", TemplateCategory::Utility)
    ///     .body_text("Danke")
    ///     .validate()
    ///     .is_ok());
    /// assert!(TemplateDefinition::new("", "de", TemplateCategory::Utility)
    ///     .body_text("Danke")
    ///     .validate()
    ///     .is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or over-long name, a bad language
    /// code, a missing body, a duplicate or unsupported component, and every
    /// limit the components themselves enforce.
    pub fn validate(&self) -> Result<()> {
        validate_template_name(&self.name)?;
        self.language.validate()?;
        validate_component_set(&self.components)?;
        if !self
            .components
            .iter()
            .any(|component| component.kind() == "BODY")
        {
            return Err(Error::validation(
                "template.components",
                "a body component is required",
            ));
        }
        Ok(())
    }

    /// Validates and renders the create payload.
    ///
    /// ```
    /// use meta_whatsapp::{TemplateCategory, TemplateDefinition};
    ///
    /// let payload = TemplateDefinition::new("hello_world", "en_US", TemplateCategory::Utility)
    ///     .body_text("Danke")
    ///     .render()?;
    /// assert_eq!(payload["parameter_format"], "POSITIONAL");
    /// assert_eq!(payload["components"][0]["type"], "BODY");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from [`TemplateDefinition::validate`].
    pub fn render(&self) -> Result<Value> {
        self.validate()?;
        let mut object = Map::new();
        object.insert("name".into(), json!(self.name));
        object.insert("language".into(), json!(self.language.code));
        object.insert("category".into(), json!(self.category.as_str()));
        object.insert(
            "parameter_format".into(),
            json!(self.parameter_format.as_str()),
        );
        object.insert(
            "components".into(),
            Value::Array(
                self.components
                    .iter()
                    .map(TemplateDefinitionComponent::render)
                    .collect(),
            ),
        );
        Ok(Value::Object(object))
    }

    fn component_index(&self, kind: &'static str) -> Option<usize> {
        self.components
            .iter()
            .position(|component| component.kind() == kind)
    }
}

/// An edit of an existing template for
/// `PUT /{phone_number_id}/message_templates`.
///
/// Name and language identify the template; `category` and the components are
/// what the edit changes. At least one of them has to be present, otherwise the
/// request could not change anything.
///
/// ```
/// use meta_whatsapp::template::{TemplateDefinitionComponent, TemplateUpdate};
/// use meta_whatsapp::TemplateCategory;
///
/// let update = TemplateUpdate::new("order_update", "de")
///     .category(TemplateCategory::Utility)
///     .component(TemplateDefinitionComponent::body_text("Neu: {{1}}").with_example("42"));
///
/// let payload = update.render()?;
/// assert_eq!(payload["name"], "order_update");
/// assert_eq!(payload["category"], "UTILITY");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateUpdate {
    /// Name of the template to edit.
    pub name: String,
    /// Language of the template to edit.
    pub language: TemplateLanguage,
    /// New category, when the edit moves the template.
    pub category: Option<TemplateCategory>,
    /// New components, in render order (may be empty).
    pub components: Vec<TemplateDefinitionComponent>,
}

impl TemplateUpdate {
    /// An edit that only names the template.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateUpdate;
    ///
    /// let update = TemplateUpdate::new("order_update", "de");
    /// assert_eq!(update.name, "order_update");
    /// assert!(update.validate().is_err(), "nothing would change");
    /// ```
    pub fn new(name: impl Into<String>, language_code: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            language: TemplateLanguage::new(language_code),
            category: None,
            components: Vec::new(),
        }
    }

    /// Sets the new category.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateUpdate;
    /// use meta_whatsapp::TemplateCategory;
    ///
    /// let update = TemplateUpdate::new("t", "de").category(TemplateCategory::Marketing);
    /// assert!(update.validate().is_ok());
    /// ```
    #[must_use]
    pub fn category(mut self, category: TemplateCategory) -> Self {
        self.category = Some(category);
        self
    }

    /// Appends a component to replace the stored one of the same kind.
    ///
    /// ```
    /// use meta_whatsapp::template::{TemplateDefinitionComponent, TemplateUpdate};
    ///
    /// let update = TemplateUpdate::new("t", "de")
    ///     .component(TemplateDefinitionComponent::footer_text("Danke"));
    /// assert_eq!(update.components.len(), 1);
    /// ```
    #[must_use]
    pub fn component(mut self, component: TemplateDefinitionComponent) -> Self {
        self.components.push(component);
        self
    }

    /// Checks the name, the language and any component that is sent.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateUpdate;
    ///
    /// assert!(TemplateUpdate::new("t", "de").validate().is_err());
    /// assert!(TemplateUpdate::new("", "de")
    ///     .category(meta_whatsapp::TemplateCategory::Utility)
    ///     .validate()
    ///     .is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or over-long name, a bad language
    /// code, an edit that changes nothing, or an invalid component.
    pub fn validate(&self) -> Result<()> {
        validate_template_name(&self.name)?;
        self.language.validate()?;
        if self.category.is_none() && self.components.is_empty() {
            return Err(Error::validation(
                "template.update",
                "an edit needs a category or at least one component",
            ));
        }
        validate_component_set(&self.components)
    }

    /// Validates and renders the edit payload.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateUpdate;
    ///
    /// let payload = TemplateUpdate::new("t", "de")
    ///     .category(meta_whatsapp::TemplateCategory::Utility)
    ///     .render()?;
    /// assert_eq!(payload["language"], "de");
    /// assert!(payload.get("components").is_none());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from [`TemplateUpdate::validate`].
    pub fn render(&self) -> Result<Value> {
        self.validate()?;
        let mut object = Map::new();
        object.insert("name".into(), json!(self.name));
        object.insert("language".into(), json!(self.language.code));
        if let Some(category) = self.category {
            object.insert("category".into(), json!(category.as_str()));
        }
        if !self.components.is_empty() {
            object.insert(
                "components".into(),
                Value::Array(
                    self.components
                        .iter()
                        .map(TemplateDefinitionComponent::render)
                        .collect(),
                ),
            );
        }
        Ok(Value::Object(object))
    }
}

/// Meta's answer to a successful create or edit.
///
/// The two endpoints answer differently — create returns `{id, status,
/// category}`, edit `{"success": true}` — so every field is optional and
/// unknown members are kept.
///
/// ```
/// use meta_whatsapp::template::TemplateAck;
///
/// let ack = TemplateAck::from_value(&serde_json::json!({"id": "1234", "status": "PENDING"}))?;
/// assert_eq!(ack.id.as_deref(), Some("1234"));
/// assert!(ack.is_success());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TemplateAck {
    /// Id Meta assigned to a new template.
    #[serde(default)]
    pub id: Option<String>,
    /// Review status, e.g. `PENDING`.
    #[serde(default)]
    pub status: Option<String>,
    /// Category Meta stored.
    #[serde(default)]
    pub category: Option<String>,
    /// Meta's success flag, when the endpoint sends one.
    #[serde(default)]
    pub success: Option<bool>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl TemplateAck {
    /// Decodes a 2xx body; nothing is required, unknown members are kept.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateAck;
    ///
    /// let ack = TemplateAck::from_value(&serde_json::json!({"success": true, "new_field": 7}))?;
    /// assert_eq!(ack.extra["new_field"], 7);
    /// assert!(TemplateAck::from_value(&serde_json::json!([1, 2])).is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the body is not a JSON object.
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }

    /// Whether Meta reported the call as accepted: `true` unless it explicitly
    /// said `success: false` (a 2xx already passed the client's status check).
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateAck;
    ///
    /// assert!(TemplateAck::from_value(&serde_json::json!({"status": "PENDING"}))?.is_success());
    /// assert!(!TemplateAck::from_value(&serde_json::json!({"success": false}))?.is_success());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_success(&self) -> bool {
        !matches!(self.success, Some(false))
    }
}

/// Meta's answer to a successful delete (`{"success": true}`).
///
/// ```
/// use meta_whatsapp::template::TemplateDeleteAck;
///
/// let ack = TemplateDeleteAck::from_value(&serde_json::json!({"success": true}))?;
/// assert!(ack.is_success());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TemplateDeleteAck {
    /// Meta's success flag, when it sends one.
    #[serde(default)]
    pub success: Option<bool>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl TemplateDeleteAck {
    /// Decodes a 2xx body; nothing is required, unknown members are kept.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDeleteAck;
    ///
    /// let ack = TemplateDeleteAck::from_value(&serde_json::json!({}))?;
    /// assert_eq!(ack.success, None);
    /// assert!(ack.is_success(), "a 2xx without a flag is still accepted");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the body is not a JSON object.
    pub fn from_value(value: &Value) -> Result<Self> {
        serde_json::from_value(value.clone()).map_err(Error::from)
    }

    /// Whether Meta reported the delete as accepted: `true` unless it
    /// explicitly said `success: false`.
    ///
    /// ```
    /// use meta_whatsapp::template::TemplateDeleteAck;
    ///
    /// assert!(!TemplateDeleteAck::from_value(&serde_json::json!({"success": false}))?.is_success());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_success(&self) -> bool {
        !matches!(self.success, Some(false))
    }
}

/// Checks the component set of a definition or an edit: at most
/// [`TEMPLATE_COMPONENTS_MAX`] components, one per kind, each valid.
fn validate_component_set(components: &[TemplateDefinitionComponent]) -> Result<()> {
    if components.len() > TEMPLATE_COMPONENTS_MAX {
        return Err(Error::validation(
            "template.components",
            format!(
                "at most {TEMPLATE_COMPONENTS_MAX} components are allowed, got {}",
                components.len()
            ),
        ));
    }
    for (index, component) in components.iter().enumerate() {
        if components[..index]
            .iter()
            .any(|other| other.kind() == component.kind())
        {
            return Err(Error::validation(
                "template.components",
                format!("duplicate {} component", component.kind()),
            ));
        }
    }
    for component in components {
        component.validate()?;
    }
    Ok(())
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

pub(crate) fn encode_query_value(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE_SET).to_string()
}

pub(crate) fn percent_encode(value: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(byte));
            }
            _ => {
                // Writing into a String cannot fail.
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
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

    #[test]
    fn only_text_examples_reach_the_wire() {
        // `validate` rejects a non-text example, so this is the total render
        // rule: values without a string form contribute nothing.
        let examples = [
            TemplateParameter::text("a"),
            TemplateParameter::currency("$1", "USD", 1_000),
            TemplateParameter::media(MediaRef::id("1"), TemplateMediaKind::Image),
        ];
        let values = example_values(&examples);
        assert_eq!(values, vec!["a"]);
    }

    #[test]
    fn the_definition_helpers_ignore_a_component_of_another_kind() {
        // The builder helpers look the component up by kind, so a helper that
        // meets the wrong variant must leave it untouched instead of panicking.
        let mut footer = TemplateDefinitionComponent::footer_text("Fuß");
        footer.set_header_text("Kopf".into());
        footer.set_body_text("Text".into());
        footer.push_button(TemplateButton::quick_reply("Ja"));
        assert_eq!(footer, TemplateDefinitionComponent::footer_text("Fuß"));

        let mut header = TemplateDefinitionComponent::header_text("{{1}}");
        header.push_example(TemplateParameter::text("a"));
        assert!(header.validate().is_ok());
    }
}
