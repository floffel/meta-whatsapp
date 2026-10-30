//! Typed outbound messages: text, media, location, contacts, reactions,
//! interactive (buttons/list/flow/address/catalog products) and templates.
//!
//! Every family is a plain struct with a constructor and chainable setters.
//! [`MessageRequest::to_json`] validates before it renders, so a malformed
//! message fails with [`Error::Validation`] and is never sent.
//!
//! ```
//! use meta_whatsapp::{ImageMessage, MediaRef, MessageRequest, TextMessage};
//!
//! let text = MessageRequest::new("4915112345678", TextMessage::new("Hallo!"))
//!     .reply_to("wamid.inbound")
//!     .biz_opaque_callback_data("order-42")
//!     .to_json()?;
//! assert_eq!(text["type"], "text");
//! assert_eq!(text["context"]["message_id"], "wamid.inbound");
//!
//! let image = MessageRequest::new(
//!     "4915112345678",
//!     ImageMessage::new(MediaRef::id("123")).caption("Speisekarte"),
//! )
//! .to_json()?;
//! assert_eq!(image["image"]["caption"], "Speisekarte");
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use serde_json::{json, Map, Value};

use crate::address::AddressMessage;
use crate::error::{Error, Result};
use crate::media::MediaRef;
use crate::template::TemplateMessage;

/// Hard limit for a text body, measured in UTF-8 bytes.
///
/// Meta counts characters; bytes are never fewer than characters, so this is
/// the conservative side of the boundary.
pub const TEXT_MAX_BYTES: usize = 4096;

/// Limit for image/video/document captions.
pub const CAPTION_MAX_BYTES: usize = 1024;

/// Limit for interactive header and footer text (characters).
pub const HEADER_MAX_CHARS: usize = 60;

/// Limit for a quick-reply or list button label (characters).
pub const BUTTON_TITLE_MAX_CHARS: usize = 20;

/// Limit for a button/row id (characters).
pub const BUTTON_ID_MAX_CHARS: usize = 256;

/// Maximum number of quick-reply buttons Meta accepts.
pub const BUTTONS_MAX: usize = 3;

/// Maximum number of rows in a list message (across all sections).
pub const LIST_ROWS_MAX: usize = 10;

/// Maximum number of sections in a list message.
pub const LIST_SECTIONS_MAX: usize = 10;

/// Limit for a multi-product section title (characters).
///
/// ```
/// use meta_whatsapp::message::PRODUCT_SECTION_TITLE_MAX_CHARS;
/// assert_eq!(PRODUCT_SECTION_TITLE_MAX_CHARS, 24);
/// ```
pub const PRODUCT_SECTION_TITLE_MAX_CHARS: usize = 24;

/// Maximum number of sections in a multi-product message.
///
/// ```
/// use meta_whatsapp::message::PRODUCT_SECTIONS_MAX;
/// assert_eq!(PRODUCT_SECTIONS_MAX, 10);
/// ```
pub const PRODUCT_SECTIONS_MAX: usize = 10;

/// Maximum number of products in one section of a multi-product message.
///
/// ```
/// use meta_whatsapp::message::PRODUCT_ITEMS_MAX;
/// assert_eq!(PRODUCT_ITEMS_MAX, 30);
/// ```
pub const PRODUCT_ITEMS_MAX: usize = 30;

/// Maximum number of products across all sections of a multi-product message.
///
/// The spec caps `product_items` per section; Meta's reference additionally
/// caps the message as a whole at 30 products, so both apply.
///
/// ```
/// use meta_whatsapp::message::PRODUCTS_MAX;
/// assert_eq!(PRODUCTS_MAX, 30);
/// ```
pub const PRODUCTS_MAX: usize = 30;

/// Limit for location name/address and contact fields.
pub const FREEFORM_MAX_BYTES: usize = 1024;

/// Who receives the message. The Cloud API only supports individuals.
///
/// ```
/// use meta_whatsapp::RecipientType;
/// assert_eq!(RecipientType::Individual.as_str(), "individual");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum RecipientType {
    /// A single `wa_id`, the only value Meta documents.
    #[default]
    Individual,
}

impl RecipientType {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::RecipientType;
    /// assert_eq!(RecipientType::Individual.as_str(), "individual");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Individual => "individual",
        }
    }
}

/// A complete `/messages` payload: recipient, optional reply context and one
/// typed body.
///
/// ```
/// use meta_whatsapp::{MessageRequest, TextMessage};
///
/// let request = MessageRequest::new("4915112345678", TextMessage::new("Hallo"))
///     .reply_to("wamid.abc");
/// let json = request.to_json()?;
/// assert_eq!(json["messaging_product"], "whatsapp");
/// assert_eq!(json["recipient_type"], "individual");
/// assert_eq!(json["text"]["preview_url"], false);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MessageRequest {
    to: String,
    recipient_type: RecipientType,
    reply_to: Option<String>,
    biz_opaque_callback_data: Option<String>,
    body: MessageBody,
}

impl MessageRequest {
    /// Builds a request for `to` (digits only, no `+`) carrying `body`.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    /// let request = MessageRequest::new("4915112345678", TextMessage::new("hi"));
    /// assert_eq!(request.to(), "4915112345678");
    /// ```
    pub fn new(to: impl Into<String>, body: impl Into<MessageBody>) -> Self {
        Self {
            to: to.into(),
            recipient_type: RecipientType::Individual,
            reply_to: None,
            biz_opaque_callback_data: None,
            body: body.into(),
        }
    }

    /// Quotes an inbound message (`context.message_id`).
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    ///
    /// let json = MessageRequest::new("1", TextMessage::new("hi"))
    ///     .reply_to("wamid.quoted")
    ///     .to_json()?;
    /// assert_eq!(json["context"]["message_id"], "wamid.quoted");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to = Some(message_id.into());
        self
    }

    /// Attaches `biz_opaque_callback_data`, echoed back on status webhooks.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    ///
    /// let json = MessageRequest::new("1", TextMessage::new("hi"))
    ///     .biz_opaque_callback_data("ticket-7")
    ///     .to_json()?;
    /// assert_eq!(json["biz_opaque_callback_data"], "ticket-7");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn biz_opaque_callback_data(mut self, data: impl Into<String>) -> Self {
        self.biz_opaque_callback_data = Some(data.into());
        self
    }

    /// Overrides the recipient type (only [`RecipientType::Individual`] exists).
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, RecipientType, TextMessage};
    ///
    /// let request = MessageRequest::new("1", TextMessage::new("hi"))
    ///     .recipient_type(RecipientType::Individual);
    /// assert_eq!(request.to_json().unwrap()["recipient_type"], "individual");
    /// ```
    #[must_use]
    pub fn recipient_type(mut self, recipient_type: RecipientType) -> Self {
        self.recipient_type = recipient_type;
        self
    }

    /// The recipient `wa_id`.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    /// assert_eq!(MessageRequest::new("42", TextMessage::new("x")).to(), "42");
    /// ```
    #[must_use]
    pub fn to(&self) -> &str {
        &self.to
    }

    /// The message body.
    ///
    /// ```
    /// use meta_whatsapp::{MessageBody, MessageRequest, TextMessage};
    /// let request = MessageRequest::new("42", TextMessage::new("x"));
    /// assert!(matches!(request.body(), MessageBody::Text(_)));
    /// ```
    #[must_use]
    pub fn body(&self) -> &MessageBody {
        &self.body
    }

    /// The quoted message id, if any.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    /// assert_eq!(
    ///     MessageRequest::new("42", TextMessage::new("x")).reply_to("wamid.1").reply_to_id(),
    ///     Some("wamid.1")
    /// );
    /// ```
    #[must_use]
    pub fn reply_to_id(&self) -> Option<&str> {
        self.reply_to.as_deref()
    }

    /// Checks every field of the envelope and the body.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, TextMessage};
    ///
    /// assert!(MessageRequest::new("4915112345678", TextMessage::new("hi")).validate().is_ok());
    /// assert!(MessageRequest::new("+49 151", TextMessage::new("hi")).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field.
    pub fn validate(&self) -> Result<()> {
        require_digits("to", &self.to)?;
        if let Some(id) = &self.reply_to {
            require_non_empty("context.message_id", id)?;
            require_max_bytes("context.message_id", id, 256)?;
            require_no_controls("context.message_id", id)?;
        }
        if let Some(data) = &self.biz_opaque_callback_data {
            require_max_bytes("biz_opaque_callback_data", data, 512)?;
        }
        self.body.validate()
    }

    /// Validates and renders the JSON body posted to `/messages`.
    ///
    /// ```
    /// use meta_whatsapp::{MessageRequest, ReactionMessage};
    ///
    /// let json = MessageRequest::new("1", ReactionMessage::new("wamid.1", "👍")).to_json()?;
    /// assert_eq!(json["reaction"]["emoji"], "👍");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from [`MessageRequest::validate`].
    pub fn to_json(&self) -> Result<Value> {
        self.validate()?;
        let mut object = Map::new();
        object.insert("messaging_product".into(), Value::String("whatsapp".into()));
        object.insert(
            "recipient_type".into(),
            Value::String(self.recipient_type.as_str().into()),
        );
        object.insert("to".into(), Value::String(self.to.clone()));
        if let Some(id) = &self.reply_to {
            object.insert("context".into(), json!({ "message_id": id }));
        }
        if let Some(data) = &self.biz_opaque_callback_data {
            object.insert(
                "biz_opaque_callback_data".into(),
                Value::String(data.clone()),
            );
        }
        let (kind, body) = self.body.render()?;
        object.insert("type".into(), Value::String(kind.to_string()));
        object.insert(kind.to_string(), body);
        Ok(Value::Object(object))
    }
}

/// One message body. `#[non_exhaustive]`: new families are additive.
///
/// ```
/// use meta_whatsapp::{MessageBody, StickerMessage, MediaRef};
///
/// let body: MessageBody = StickerMessage::new(MediaRef::id("9")).into();
/// assert_eq!(body.message_type(), "sticker");
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum MessageBody {
    /// Plain text.
    Text(TextMessage),
    /// JPEG/PNG image.
    Image(ImageMessage),
    /// MP4/3GP video.
    Video(VideoMessage),
    /// Audio file or voice note.
    Audio(AudioMessage),
    /// Any document, with optional caption and filename.
    Document(DocumentMessage),
    /// WebP sticker.
    Sticker(StickerMessage),
    /// Latitude/longitude pin.
    Location(LocationMessage),
    /// One or more contact cards.
    Contacts(ContactsMessage),
    /// Emoji reaction to an inbound message.
    Reaction(ReactionMessage),
    /// Buttons, list or flow.
    Interactive(InteractiveMessage),
    /// Pre-approved template.
    Template(TemplateMessage),
}

impl MessageBody {
    /// The `type` value Meta expects (`text`, `image`, `interactive`, ...).
    ///
    /// ```
    /// use meta_whatsapp::{MessageBody, TextMessage};
    /// assert_eq!(MessageBody::Text(TextMessage::new("x")).message_type(), "text");
    /// ```
    #[must_use]
    pub fn message_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "text",
            Self::Image(_) => "image",
            Self::Video(_) => "video",
            Self::Audio(_) => "audio",
            Self::Document(_) => "document",
            Self::Sticker(_) => "sticker",
            Self::Location(_) => "location",
            Self::Contacts(_) => "contacts",
            Self::Reaction(_) => "reaction",
            Self::Interactive(_) => "interactive",
            Self::Template(_) => "template",
        }
    }

    /// Validates the body without rendering it.
    ///
    /// ```
    /// use meta_whatsapp::{MessageBody, TextMessage};
    ///
    /// assert!(MessageBody::Text(TextMessage::new("ok")).validate().is_ok());
    /// assert!(MessageBody::Text(TextMessage::new("")).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Text(m) => m.validate(),
            Self::Image(m) => m.validate(),
            Self::Video(m) => m.validate(),
            Self::Audio(m) => m.validate(),
            Self::Document(m) => m.validate(),
            Self::Sticker(m) => m.validate(),
            Self::Location(m) => m.validate(),
            Self::Contacts(m) => m.validate(),
            Self::Reaction(m) => m.validate(),
            Self::Interactive(m) => m.validate(),
            Self::Template(m) => m.validate(),
        }
    }

    /// Validates and returns the `(type, body)` pair.
    fn render(&self) -> Result<(&'static str, Value)> {
        self.validate()?;
        let value = match self {
            Self::Text(m) => m.render(),
            Self::Image(m) => m.render(),
            Self::Video(m) => m.render(),
            Self::Audio(m) => m.render(),
            Self::Document(m) => m.render(),
            Self::Sticker(m) => m.render(),
            Self::Location(m) => m.render(),
            Self::Contacts(m) => m.render(),
            Self::Reaction(m) => m.render(),
            Self::Interactive(m) => m.render(),
            Self::Template(m) => m.render()?,
        };
        Ok((self.message_type(), value))
    }
}

macro_rules! impl_from_body {
    ($($variant:ident => $ty:ty),+ $(,)?) => {
        $(
            impl From<$ty> for MessageBody {
                fn from(value: $ty) -> Self {
                    Self::$variant(value)
                }
            }
        )+
    };
}

impl_from_body! {
    Text => TextMessage,
    Image => ImageMessage,
    Video => VideoMessage,
    Audio => AudioMessage,
    Document => DocumentMessage,
    Sticker => StickerMessage,
    Location => LocationMessage,
    Contacts => ContactsMessage,
    Reaction => ReactionMessage,
    Interactive => InteractiveMessage,
    Template => TemplateMessage,
}

/// Plain text, optionally unfurling the first URL.
///
/// ```
/// use meta_whatsapp::{MessageRequest, TextMessage};
///
/// let json = MessageRequest::new("1", TextMessage::new("see https://example.com").preview_url(true))
///     .to_json()?;
/// assert_eq!(json["text"]["preview_url"], true);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextMessage {
    /// Message text, at most [`TEXT_MAX_BYTES`] UTF-8 bytes.
    pub body: String,
    /// Whether Meta should render a link preview (default `false`).
    pub preview_url: bool,
}

impl TextMessage {
    /// A text message with link previews off.
    ///
    /// ```
    /// use meta_whatsapp::TextMessage;
    /// assert!(!TextMessage::new("hi").preview_url);
    /// ```
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            preview_url: false,
        }
    }

    /// Enables or disables the link preview.
    ///
    /// ```
    /// use meta_whatsapp::TextMessage;
    /// assert!(TextMessage::new("x").preview_url(true).preview_url);
    /// ```
    #[must_use]
    pub fn preview_url(mut self, yes: bool) -> Self {
        self.preview_url = yes;
        self
    }

    /// Rejects empty bodies and bodies over 4096 UTF-8 bytes.
    ///
    /// ```
    /// use meta_whatsapp::TextMessage;
    ///
    /// assert!(TextMessage::new("x".repeat(4096)).validate().is_ok());
    /// assert!(TextMessage::new("x".repeat(4097)).validate().is_err());
    /// assert!(TextMessage::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for empty or oversized text.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("text.body", &self.body)?;
        require_max_bytes("text.body", &self.body, TEXT_MAX_BYTES)
    }

    fn render(&self) -> Value {
        json!({ "preview_url": self.preview_url, "body": self.body })
    }
}

/// An image by id or link, with an optional caption.
///
/// ```
/// use meta_whatsapp::{ImageMessage, MediaRef, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     ImageMessage::new(MediaRef::link("https://example.com/cat.png")).caption("Katze"),
/// )
/// .to_json()?;
/// assert_eq!(json["image"]["link"], "https://example.com/cat.png");
/// assert_eq!(json["image"]["caption"], "Katze");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMessage {
    /// Uploaded id or public https link.
    pub media: MediaRef,
    /// Caption, at most [`CAPTION_MAX_BYTES`] bytes.
    pub caption: Option<String>,
}

impl ImageMessage {
    /// An image without caption.
    ///
    /// ```
    /// use meta_whatsapp::{ImageMessage, MediaRef};
    /// assert!(ImageMessage::new(MediaRef::id("1")).caption.is_none());
    /// ```
    #[must_use]
    pub fn new(media: MediaRef) -> Self {
        Self {
            media,
            caption: None,
        }
    }

    /// Adds a caption.
    ///
    /// ```
    /// use meta_whatsapp::{ImageMessage, MediaRef};
    /// assert_eq!(ImageMessage::new(MediaRef::id("1")).caption("x").caption.unwrap(), "x");
    /// ```
    #[must_use]
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Checks the media reference and the caption length.
    ///
    /// ```
    /// use meta_whatsapp::{ImageMessage, MediaRef};
    ///
    /// assert!(ImageMessage::new(MediaRef::id("1")).validate().is_ok());
    /// assert!(ImageMessage::new(MediaRef::id("../x")).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media reference or long caption.
    pub fn validate(&self) -> Result<()> {
        self.media.validate()?;
        validate_optional_caption("image.caption", self.caption.as_deref())
    }

    fn render(&self) -> Value {
        render_media(&self.media, &[("caption", self.caption.as_deref())])
    }
}

/// A video by id or link, with an optional caption.
///
/// ```
/// use meta_whatsapp::{MediaRef, MessageRequest, VideoMessage};
///
/// let json = MessageRequest::new("1", VideoMessage::new(MediaRef::id("5"))).to_json()?;
/// assert_eq!(json["video"]["id"], "5");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoMessage {
    /// Uploaded id or public https link.
    pub media: MediaRef,
    /// Caption, at most [`CAPTION_MAX_BYTES`] bytes.
    pub caption: Option<String>,
}

impl VideoMessage {
    /// A video without caption.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, VideoMessage};
    /// assert!(VideoMessage::new(MediaRef::id("1")).caption.is_none());
    /// ```
    #[must_use]
    pub fn new(media: MediaRef) -> Self {
        Self {
            media,
            caption: None,
        }
    }

    /// Adds a caption.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, VideoMessage};
    /// assert!(VideoMessage::new(MediaRef::id("1")).caption("x").caption.is_some());
    /// ```
    #[must_use]
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Checks the media reference and the caption length.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, VideoMessage};
    /// assert!(VideoMessage::new(MediaRef::id("1")).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media reference or long caption.
    pub fn validate(&self) -> Result<()> {
        self.media.validate()?;
        validate_optional_caption("video.caption", self.caption.as_deref())
    }

    fn render(&self) -> Value {
        render_media(&self.media, &[("caption", self.caption.as_deref())])
    }
}

/// An audio file, or a voice note when `voice` is set.
///
/// ```
/// use meta_whatsapp::{AudioMessage, MediaRef, MessageRequest};
///
/// let json = MessageRequest::new("1", AudioMessage::new(MediaRef::id("7")).voice(true)).to_json()?;
/// assert_eq!(json["audio"]["voice"], true);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioMessage {
    /// Uploaded id or public https link.
    pub media: MediaRef,
    /// Render as a voice note (opus/ogg only).
    pub voice: bool,
}

impl AudioMessage {
    /// An audio message that is not a voice note.
    ///
    /// ```
    /// use meta_whatsapp::{AudioMessage, MediaRef};
    /// assert!(!AudioMessage::new(MediaRef::id("1")).voice);
    /// ```
    #[must_use]
    pub fn new(media: MediaRef) -> Self {
        Self {
            media,
            voice: false,
        }
    }

    /// Marks the audio as a voice note.
    ///
    /// ```
    /// use meta_whatsapp::{AudioMessage, MediaRef};
    /// assert!(AudioMessage::new(MediaRef::id("1")).voice(true).voice);
    /// ```
    #[must_use]
    pub fn voice(mut self, yes: bool) -> Self {
        self.voice = yes;
        self
    }

    /// Checks the media reference.
    ///
    /// ```
    /// use meta_whatsapp::{AudioMessage, MediaRef};
    /// assert!(AudioMessage::new(MediaRef::id("1")).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media reference.
    pub fn validate(&self) -> Result<()> {
        self.media.validate()
    }

    fn render(&self) -> Value {
        let mut object = match self.media.to_json() {
            Value::Object(map) => map,
            other => return other,
        };
        if self.voice {
            object.insert("voice".into(), Value::Bool(true));
        }
        Value::Object(object)
    }
}

/// A document by id or link, with optional caption and filename.
///
/// ```
/// use meta_whatsapp::{DocumentMessage, MediaRef, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     DocumentMessage::new(MediaRef::id("3")).filename("Rechnung.pdf"),
/// )
/// .to_json()?;
/// assert_eq!(json["document"]["filename"], "Rechnung.pdf");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentMessage {
    /// Uploaded id or public https link.
    pub media: MediaRef,
    /// Caption, at most [`CAPTION_MAX_BYTES`] bytes.
    pub caption: Option<String>,
    /// Filename shown to the recipient.
    pub filename: Option<String>,
}

impl DocumentMessage {
    /// A document without caption or filename.
    ///
    /// ```
    /// use meta_whatsapp::{DocumentMessage, MediaRef};
    /// assert!(DocumentMessage::new(MediaRef::id("1")).filename.is_none());
    /// ```
    #[must_use]
    pub fn new(media: MediaRef) -> Self {
        Self {
            media,
            caption: None,
            filename: None,
        }
    }

    /// Adds a caption.
    ///
    /// ```
    /// use meta_whatsapp::{DocumentMessage, MediaRef};
    /// assert!(DocumentMessage::new(MediaRef::id("1")).caption("x").caption.is_some());
    /// ```
    #[must_use]
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Sets the filename shown in chat.
    ///
    /// ```
    /// use meta_whatsapp::{DocumentMessage, MediaRef};
    /// assert_eq!(DocumentMessage::new(MediaRef::id("1")).filename("a.pdf").filename.unwrap(), "a.pdf");
    /// ```
    #[must_use]
    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Checks media, caption and filename.
    ///
    /// ```
    /// use meta_whatsapp::{DocumentMessage, MediaRef};
    ///
    /// assert!(DocumentMessage::new(MediaRef::id("1")).filename("a.pdf").validate().is_ok());
    /// assert!(DocumentMessage::new(MediaRef::id("1")).filename("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for unsafe media, long caption or bad filename.
    pub fn validate(&self) -> Result<()> {
        self.media.validate()?;
        validate_optional_caption("document.caption", self.caption.as_deref())?;
        if let Some(filename) = &self.filename {
            require_non_empty("document.filename", filename)?;
            require_max_bytes(
                "document.filename",
                filename,
                crate::media::MAX_FILENAME_LEN,
            )?;
            require_no_controls("document.filename", filename)?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        render_media(
            &self.media,
            &[
                ("caption", self.caption.as_deref()),
                ("filename", self.filename.as_deref()),
            ],
        )
    }
}

/// A sticker (static or animated WebP).
///
/// ```
/// use meta_whatsapp::{MediaRef, MessageRequest, StickerMessage};
///
/// let json = MessageRequest::new("1", StickerMessage::new(MediaRef::id("11"))).to_json()?;
/// assert_eq!(json["sticker"]["id"], "11");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StickerMessage {
    /// Uploaded id or public https link.
    pub media: MediaRef,
}

impl StickerMessage {
    /// A sticker pointing at `media`.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, StickerMessage};
    /// assert_eq!(StickerMessage::new(MediaRef::id("1")).media.as_id(), Some("1"));
    /// ```
    #[must_use]
    pub fn new(media: MediaRef) -> Self {
        Self { media }
    }

    /// Checks the media reference.
    ///
    /// ```
    /// use meta_whatsapp::{MediaRef, StickerMessage};
    /// assert!(StickerMessage::new(MediaRef::id("1")).validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media reference.
    pub fn validate(&self) -> Result<()> {
        self.media.validate()
    }

    fn render(&self) -> Value {
        self.media.to_json()
    }
}

/// A location pin with optional name and address.
///
/// ```
/// use meta_whatsapp::{LocationMessage, MessageRequest};
///
/// let json = MessageRequest::new("1", LocationMessage::new(48.7665, 11.4258).name("Hauptbahnhof"))
///     .to_json()?;
/// assert_eq!(json["location"]["name"], "Hauptbahnhof");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct LocationMessage {
    /// Degrees north, `-90.0..=90.0`.
    pub latitude: f64,
    /// Degrees east, `-180.0..=180.0`.
    pub longitude: f64,
    /// Optional place name, at most [`FREEFORM_MAX_BYTES`] bytes.
    pub name: Option<String>,
    /// Optional street address, at most [`FREEFORM_MAX_BYTES`] bytes.
    pub address: Option<String>,
}

impl LocationMessage {
    /// A bare coordinate pair.
    ///
    /// ```
    /// use meta_whatsapp::LocationMessage;
    /// let location = LocationMessage::new(52.52, 13.405);
    /// assert_eq!(location.latitude, 52.52);
    /// ```
    #[must_use]
    pub fn new(latitude: f64, longitude: f64) -> Self {
        Self {
            latitude,
            longitude,
            name: None,
            address: None,
        }
    }

    /// Adds a place name.
    ///
    /// ```
    /// use meta_whatsapp::LocationMessage;
    /// assert_eq!(LocationMessage::new(1.0, 2.0).name("X").name.unwrap(), "X");
    /// ```
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Adds a street address.
    ///
    /// ```
    /// use meta_whatsapp::LocationMessage;
    /// assert!(LocationMessage::new(1.0, 2.0).address("Street 1").address.is_some());
    /// ```
    #[must_use]
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Rejects non-finite or out-of-range coordinates.
    ///
    /// ```
    /// use meta_whatsapp::LocationMessage;
    ///
    /// assert!(LocationMessage::new(90.0, -180.0).validate().is_ok());
    /// assert!(LocationMessage::new(90.1, 0.0).validate().is_err());
    /// assert!(LocationMessage::new(f64::NAN, 0.0).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for out-of-range coordinates or long text.
    pub fn validate(&self) -> Result<()> {
        if !self.latitude.is_finite() || !(-90.0..=90.0).contains(&self.latitude) {
            return Err(Error::validation(
                "location.latitude",
                "must be finite and within -90.0..=90.0",
            ));
        }
        if !self.longitude.is_finite() || !(-180.0..=180.0).contains(&self.longitude) {
            return Err(Error::validation(
                "location.longitude",
                "must be finite and within -180.0..=180.0",
            ));
        }
        if let Some(name) = &self.name {
            require_max_bytes("location.name", name, FREEFORM_MAX_BYTES)?;
        }
        if let Some(address) = &self.address {
            require_max_bytes("location.address", address, FREEFORM_MAX_BYTES)?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert("latitude".into(), json!(self.latitude));
        object.insert("longitude".into(), json!(self.longitude));
        if let Some(name) = &self.name {
            object.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(address) = &self.address {
            object.insert("address".into(), Value::String(address.clone()));
        }
        Value::Object(object)
    }
}

/// Phone number kind, rendered upper-case as Meta expects.
///
/// ```
/// use meta_whatsapp::PhoneType;
/// assert_eq!(PhoneType::Cell.as_str(), "CELL");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum PhoneType {
    /// Mobile number.
    #[default]
    Cell,
    /// Main line.
    Main,
    /// iPhone.
    Iphone,
    /// Home.
    Home,
    /// Work.
    Work,
    /// Anything else.
    Other,
}

impl PhoneType {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::PhoneType;
    /// assert_eq!(PhoneType::Iphone.as_str(), "IPHONE");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cell => "CELL",
            Self::Main => "MAIN",
            Self::Iphone => "IPHONE",
            Self::Home => "HOME",
            Self::Work => "WORK",
            Self::Other => "OTHER",
        }
    }
}

/// E-mail kind.
///
/// ```
/// use meta_whatsapp::EmailType;
/// assert_eq!(EmailType::Work.as_str(), "WORK");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum EmailType {
    /// Work address.
    Work,
    /// Personal address.
    #[default]
    Home,
    /// Anything else.
    Other,
}

impl EmailType {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::EmailType;
    /// assert_eq!(EmailType::Home.as_str(), "HOME");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "WORK",
            Self::Home => "HOME",
            Self::Other => "OTHER",
        }
    }
}

/// URL kind.
///
/// ```
/// use meta_whatsapp::UrlType;
/// assert_eq!(UrlType::Work.as_str(), "WORK");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UrlType {
    /// Work profile.
    #[default]
    Work,
    /// Personal profile.
    Home,
    /// Anything else.
    Other,
}

impl UrlType {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::UrlType;
    /// assert_eq!(UrlType::Other.as_str(), "OTHER");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Work => "WORK",
            Self::Home => "HOME",
            Self::Other => "OTHER",
        }
    }
}

/// The `name` object of a contact card.
///
/// ```
/// use meta_whatsapp::ContactName;
/// let name = ContactName::new("Florian Floffel").first_name("Florian");
/// assert_eq!(name.formatted_name, "Florian Floffel");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactName {
    /// Full name as displayed; required by Meta.
    pub formatted_name: String,
    /// Given name.
    pub first_name: Option<String>,
    /// Family name.
    pub last_name: Option<String>,
    /// Middle name.
    pub middle_name: Option<String>,
    /// Honorific prefix.
    pub prefix: Option<String>,
    /// Honorific suffix.
    pub suffix: Option<String>,
}

impl ContactName {
    /// A card name with only the required formatted name.
    ///
    /// ```
    /// use meta_whatsapp::ContactName;
    /// assert!(ContactName::new("Ada Lovelace").first_name.is_none());
    /// ```
    pub fn new(formatted_name: impl Into<String>) -> Self {
        Self {
            formatted_name: formatted_name.into(),
            first_name: None,
            last_name: None,
            middle_name: None,
            prefix: None,
            suffix: None,
        }
    }

    /// Sets the given name.
    ///
    /// ```
    /// use meta_whatsapp::ContactName;
    /// assert_eq!(ContactName::new("A B").first_name("A").first_name.unwrap(), "A");
    /// ```
    #[must_use]
    pub fn first_name(mut self, value: impl Into<String>) -> Self {
        self.first_name = Some(value.into());
        self
    }

    /// Sets the family name.
    ///
    /// ```
    /// use meta_whatsapp::ContactName;
    /// assert_eq!(ContactName::new("A B").last_name("B").last_name.unwrap(), "B");
    /// ```
    #[must_use]
    pub fn last_name(mut self, value: impl Into<String>) -> Self {
        self.last_name = Some(value.into());
        self
    }

    fn validate(&self) -> Result<()> {
        require_non_empty("contacts.name.formatted_name", &self.formatted_name)?;
        require_max_bytes(
            "contacts.name.formatted_name",
            &self.formatted_name,
            FREEFORM_MAX_BYTES,
        )
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "formatted_name".into(),
            Value::String(self.formatted_name.clone()),
        );
        for (key, value) in [
            ("first_name", &self.first_name),
            ("last_name", &self.last_name),
            ("middle_name", &self.middle_name),
            ("prefix", &self.prefix),
            ("suffix", &self.suffix),
        ] {
            if let Some(value) = value {
                object.insert(key.into(), Value::String(value.clone()));
            }
        }
        Value::Object(object)
    }
}

/// One phone number on a contact card.
///
/// ```
/// use meta_whatsapp::{ContactPhone, PhoneType};
///
/// let phone = ContactPhone::new("+4915112345678", PhoneType::Cell);
/// assert_eq!(phone.number, "+4915112345678");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactPhone {
    /// Number in international format.
    pub number: String,
    /// Kind of number.
    pub kind: PhoneType,
}

impl ContactPhone {
    /// Pairs a number with its kind.
    ///
    /// ```
    /// use meta_whatsapp::{ContactPhone, PhoneType};
    /// assert_eq!(ContactPhone::new("1", PhoneType::Main).kind, PhoneType::Main);
    /// ```
    pub fn new(number: impl Into<String>, kind: PhoneType) -> Self {
        Self {
            number: number.into(),
            kind,
        }
    }

    fn validate(&self) -> Result<()> {
        require_non_empty("contacts.phones.number", &self.number)?;
        require_max_bytes("contacts.phones.number", &self.number, 64)
    }

    fn render(&self) -> Value {
        json!({ "phone": self.number, "type": self.kind.as_str() })
    }
}

/// One e-mail address on a contact card.
///
/// ```
/// use meta_whatsapp::{ContactEmail, EmailType};
/// assert_eq!(ContactEmail::new("a@b.test", EmailType::Work).kind, EmailType::Work);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactEmail {
    /// The address.
    pub address: String,
    /// Kind of address.
    pub kind: EmailType,
}

impl ContactEmail {
    /// Pairs an address with its kind.
    ///
    /// ```
    /// use meta_whatsapp::{ContactEmail, EmailType};
    /// assert_eq!(ContactEmail::new("a@b.test", EmailType::Home).address, "a@b.test");
    /// ```
    pub fn new(address: impl Into<String>, kind: EmailType) -> Self {
        Self {
            address: address.into(),
            kind,
        }
    }

    fn validate(&self) -> Result<()> {
        require_non_empty("contacts.emails.email", &self.address)?;
        require_max_bytes("contacts.emails.email", &self.address, 254)
    }

    fn render(&self) -> Value {
        json!({ "email": self.address, "type": self.kind.as_str() })
    }
}

/// Organisation block of a contact card.
///
/// ```
/// use meta_whatsapp::ContactOrg;
/// let org = ContactOrg::new("Acme").title("Engineer");
/// assert_eq!(org.title.unwrap(), "Engineer");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactOrg {
    /// Company name.
    pub company: String,
    /// Department.
    pub department: Option<String>,
    /// Job title.
    pub title: Option<String>,
}

impl ContactOrg {
    /// A company with no department or title.
    ///
    /// ```
    /// use meta_whatsapp::ContactOrg;
    /// assert!(ContactOrg::new("Acme").department.is_none());
    /// ```
    pub fn new(company: impl Into<String>) -> Self {
        Self {
            company: company.into(),
            department: None,
            title: None,
        }
    }

    /// Sets the department.
    ///
    /// ```
    /// use meta_whatsapp::ContactOrg;
    /// assert!(ContactOrg::new("A").department("R&D").department.is_some());
    /// ```
    #[must_use]
    pub fn department(mut self, value: impl Into<String>) -> Self {
        self.department = Some(value.into());
        self
    }

    /// Sets the job title.
    ///
    /// ```
    /// use meta_whatsapp::ContactOrg;
    /// assert!(ContactOrg::new("A").title("CTO").title.is_some());
    /// ```
    #[must_use]
    pub fn title(mut self, value: impl Into<String>) -> Self {
        self.title = Some(value.into());
        self
    }

    fn validate(&self) -> Result<()> {
        require_non_empty("contacts.org.company", &self.company)?;
        require_max_bytes("contacts.org.company", &self.company, FREEFORM_MAX_BYTES)
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert("company".into(), Value::String(self.company.clone()));
        if let Some(department) = &self.department {
            object.insert("department".into(), Value::String(department.clone()));
        }
        if let Some(title) = &self.title {
            object.insert("title".into(), Value::String(title.clone()));
        }
        Value::Object(object)
    }
}

/// A single contact card.
///
/// ```
/// use meta_whatsapp::{ContactCard, PhoneType};
///
/// let card = ContactCard::new("Florian")
///     .phone("+4915112345678", PhoneType::Cell)
///     .email("florian@example.com", meta_whatsapp::EmailType::Work);
/// assert_eq!(card.phones.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactCard {
    /// Name block; required.
    pub name: ContactName,
    /// Phone numbers.
    pub phones: Vec<ContactPhone>,
    /// E-mail addresses.
    pub emails: Vec<ContactEmail>,
    /// Organisation details.
    pub org: Option<ContactOrg>,
    /// Profile URLs, max 255 bytes each.
    pub urls: Vec<(String, UrlType)>,
    /// Birthday, `YYYY-MM-DD`.
    pub birthday: Option<String>,
}

impl ContactCard {
    /// A card with only the required formatted name.
    ///
    /// ```
    /// use meta_whatsapp::ContactCard;
    /// assert!(ContactCard::new("Ada").phones.is_empty());
    /// ```
    pub fn new(formatted_name: impl Into<String>) -> Self {
        Self {
            name: ContactName::new(formatted_name),
            phones: Vec::new(),
            emails: Vec::new(),
            org: None,
            urls: Vec::new(),
            birthday: None,
        }
    }

    /// Adds a phone number.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, PhoneType};
    /// assert_eq!(ContactCard::new("A").phone("1", PhoneType::Work).phones.len(), 1);
    /// ```
    #[must_use]
    pub fn phone(mut self, number: impl Into<String>, kind: PhoneType) -> Self {
        self.phones.push(ContactPhone::new(number, kind));
        self
    }

    /// Adds an e-mail address.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, EmailType};
    /// assert_eq!(ContactCard::new("A").email("a@b.test", EmailType::Home).emails.len(), 1);
    /// ```
    #[must_use]
    pub fn email(mut self, address: impl Into<String>, kind: EmailType) -> Self {
        self.emails.push(ContactEmail::new(address, kind));
        self
    }

    /// Adds a profile URL.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, UrlType};
    /// assert_eq!(ContactCard::new("A").url("https://a.test", UrlType::Work).urls.len(), 1);
    /// ```
    #[must_use]
    pub fn url(mut self, url: impl Into<String>, kind: UrlType) -> Self {
        self.urls.push((url.into(), kind));
        self
    }

    /// Sets the organisation block.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, ContactOrg};
    /// assert!(ContactCard::new("A").org(ContactOrg::new("Acme")).org.is_some());
    /// ```
    #[must_use]
    pub fn org(mut self, org: ContactOrg) -> Self {
        self.org = Some(org);
        self
    }

    /// Sets the birthday (`YYYY-MM-DD`).
    ///
    /// ```
    /// use meta_whatsapp::ContactCard;
    /// assert_eq!(ContactCard::new("A").birthday("1990-01-01").birthday.unwrap(), "1990-01-01");
    /// ```
    #[must_use]
    pub fn birthday(mut self, date: impl Into<String>) -> Self {
        self.birthday = Some(date.into());
        self
    }

    /// Checks the name and every attached entry.
    ///
    /// ```
    /// use meta_whatsapp::ContactCard;
    ///
    /// assert!(ContactCard::new("Ada").validate().is_ok());
    /// assert!(ContactCard::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty name or malformed entry.
    pub fn validate(&self) -> Result<()> {
        self.name.validate()?;
        for phone in &self.phones {
            phone.validate()?;
        }
        for email in &self.emails {
            email.validate()?;
        }
        if let Some(org) = &self.org {
            org.validate()?;
        }
        for (url, _) in &self.urls {
            require_non_empty("contacts.urls.url", url)?;
            require_max_bytes("contacts.urls.url", url, 255)?;
        }
        if let Some(birthday) = &self.birthday {
            require_max_bytes("contacts.birthday", birthday, 10)?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert("name".into(), self.name.render());
        if !self.phones.is_empty() {
            object.insert(
                "phones".into(),
                Value::Array(self.phones.iter().map(ContactPhone::render).collect()),
            );
        }
        if !self.emails.is_empty() {
            object.insert(
                "emails".into(),
                Value::Array(self.emails.iter().map(ContactEmail::render).collect()),
            );
        }
        if let Some(org) = &self.org {
            object.insert("org".into(), org.render());
        }
        if !self.urls.is_empty() {
            object.insert(
                "urls".into(),
                Value::Array(
                    self.urls
                        .iter()
                        .map(|(url, kind)| json!({ "url": url, "type": kind.as_str() }))
                        .collect(),
                ),
            );
        }
        if let Some(birthday) = &self.birthday {
            object.insert("birthday".into(), Value::String(birthday.clone()));
        }
        Value::Object(object)
    }
}

/// One or more contact cards.
///
/// ```
/// use meta_whatsapp::{ContactCard, ContactsMessage, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     ContactsMessage::new(vec![ContactCard::new("Ada")]),
/// )
/// .to_json()?;
/// assert_eq!(json["contacts"][0]["name"]["formatted_name"], "Ada");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContactsMessage {
    /// Cards to send; Meta rejects an empty list.
    pub contacts: Vec<ContactCard>,
}

impl ContactsMessage {
    /// Wraps one or more cards.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, ContactsMessage};
    /// assert_eq!(ContactsMessage::new(vec![ContactCard::new("A")]).contacts.len(), 1);
    /// ```
    #[must_use]
    pub fn new(contacts: Vec<ContactCard>) -> Self {
        Self { contacts }
    }

    /// Rejects an empty list and validates every card.
    ///
    /// ```
    /// use meta_whatsapp::{ContactCard, ContactsMessage};
    ///
    /// assert!(ContactsMessage::new(vec![ContactCard::new("A")]).validate().is_ok());
    /// assert!(ContactsMessage::new(Vec::new()).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty list or a bad card.
    pub fn validate(&self) -> Result<()> {
        if self.contacts.is_empty() {
            return Err(Error::validation(
                "contacts",
                "must contain at least one contact card",
            ));
        }
        for card in &self.contacts {
            card.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        Value::Array(self.contacts.iter().map(ContactCard::render).collect())
    }
}

/// A reaction to an inbound message. An empty emoji removes the reaction.
///
/// ```
/// use meta_whatsapp::{MessageRequest, ReactionMessage};
///
/// let add = MessageRequest::new("1", ReactionMessage::new("wamid.1", "❤")).to_json()?;
/// assert_eq!(add["reaction"]["emoji"], "❤");
///
/// let remove = MessageRequest::new("1", ReactionMessage::new("wamid.1", "")).to_json()?;
/// assert_eq!(remove["reaction"]["emoji"], "");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionMessage {
    /// Inbound message being reacted to.
    pub message_id: String,
    /// Emoji, or `""` to remove an earlier reaction.
    pub emoji: String,
}

impl ReactionMessage {
    /// Reacts to `message_id` with `emoji`.
    ///
    /// ```
    /// use meta_whatsapp::ReactionMessage;
    /// assert_eq!(ReactionMessage::new("wamid.1", "👍").emoji, "👍");
    /// ```
    pub fn new(message_id: impl Into<String>, emoji: impl Into<String>) -> Self {
        Self {
            message_id: message_id.into(),
            emoji: emoji.into(),
        }
    }

    /// Checks the message id and keeps the emoji short (Meta caps it at a
    /// single grapheme; 64 bytes is generous for ZWJ sequences).
    ///
    /// ```
    /// use meta_whatsapp::ReactionMessage;
    ///
    /// assert!(ReactionMessage::new("wamid.1", "").validate().is_ok());
    /// assert!(ReactionMessage::new("", "👍").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty id or an oversized emoji.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("reaction.message_id", &self.message_id)?;
        require_max_bytes("reaction.message_id", &self.message_id, BUTTON_ID_MAX_CHARS)?;
        require_no_controls("reaction.message_id", &self.message_id)?;
        require_max_bytes("reaction.emoji", &self.emoji, 64)
    }

    fn render(&self) -> Value {
        json!({ "message_id": self.message_id, "emoji": self.emoji })
    }
}

impl From<ButtonMessage> for MessageBody {
    fn from(value: ButtonMessage) -> Self {
        Self::Interactive(InteractiveMessage::Buttons(value))
    }
}

impl From<ListMessage> for MessageBody {
    fn from(value: ListMessage) -> Self {
        Self::Interactive(InteractiveMessage::List(value))
    }
}

impl From<FlowMessage> for MessageBody {
    fn from(value: FlowMessage) -> Self {
        Self::Interactive(InteractiveMessage::Flow(value))
    }
}

impl From<AddressMessage> for MessageBody {
    fn from(value: AddressMessage) -> Self {
        Self::Interactive(InteractiveMessage::Address(value))
    }
}

/// A text header on an interactive message.
///
/// ```
/// use meta_whatsapp::HeaderText;
/// assert_eq!(HeaderText::new("Menü").text, "Menü");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderText {
    /// Header text, at most [`HEADER_MAX_CHARS`] characters.
    pub text: String,
}

impl HeaderText {
    /// Wraps header text.
    ///
    /// ```
    /// use meta_whatsapp::HeaderText;
    /// assert_eq!(HeaderText::new("Hallo").text, "Hallo");
    /// ```
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        require_non_empty("interactive.header.text", &self.text)?;
        require_max_chars("interactive.header.text", &self.text, HEADER_MAX_CHARS)
    }
}

/// A quick-reply button: an id the webhook echoes plus a title.
///
/// ```
/// use meta_whatsapp::ReplyButton;
///
/// let button = ReplyButton::new("yes", "Ja");
/// assert_eq!(button.id, "yes");
/// assert_eq!(button.title, "Ja");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyButton {
    /// Payload echoed in `button_reply.id` (max 256 characters).
    pub id: String,
    /// Visible label (max 20 characters).
    pub title: String,
}

impl ReplyButton {
    /// Creates a reply button.
    ///
    /// ```
    /// use meta_whatsapp::ReplyButton;
    /// assert!(ReplyButton::new("id", "Titel").validate().is_ok());
    /// ```
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }

    /// Checks id and title lengths.
    ///
    /// ```
    /// use meta_whatsapp::ReplyButton;
    ///
    /// assert!(ReplyButton::new("id", "123456789012345678901").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for empty or oversized fields.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.action.buttons.id", &self.id)?;
        require_max_chars(
            "interactive.action.buttons.id",
            &self.id,
            BUTTON_ID_MAX_CHARS,
        )?;
        require_non_empty("interactive.action.buttons.title", &self.title)?;
        require_max_chars(
            "interactive.action.buttons.title",
            &self.title,
            BUTTON_TITLE_MAX_CHARS,
        )
    }

    fn render(&self) -> Value {
        json!({ "type": "reply", "reply": { "id": self.id, "title": self.title } })
    }
}

/// Interactive message with up to three quick-reply buttons.
///
/// ```
/// use meta_whatsapp::{ButtonMessage, MessageRequest, ReplyButton};
///
/// let json = MessageRequest::new(
///     "1",
///     ButtonMessage::new("Möchten Sie bestellen?", vec![ReplyButton::new("yes", "Ja")]),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "button");
/// assert_eq!(json["interactive"]["action"]["buttons"][0]["reply"]["id"], "yes");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonMessage {
    /// Optional text header.
    pub header: Option<HeaderText>,
    /// Body text.
    pub body: String,
    /// Optional footer text.
    pub footer: Option<String>,
    /// One to three buttons.
    pub buttons: Vec<ReplyButton>,
}

impl ButtonMessage {
    /// Builds a button message.
    ///
    /// ```
    /// use meta_whatsapp::{ButtonMessage, ReplyButton};
    /// assert_eq!(ButtonMessage::new("Frage?", vec![]).buttons.len(), 0);
    /// ```
    pub fn new(body: impl Into<String>, buttons: Vec<ReplyButton>) -> Self {
        Self {
            header: None,
            body: body.into(),
            footer: None,
            buttons,
        }
    }

    /// Adds a text header.
    ///
    /// ```
    /// use meta_whatsapp::{ButtonMessage, HeaderText, ReplyButton};
    /// let message = ButtonMessage::new("b", vec![ReplyButton::new("i", "t")])
    ///     .header(HeaderText::new("H"));
    /// assert!(message.header.is_some());
    /// ```
    #[must_use]
    pub fn header(mut self, header: HeaderText) -> Self {
        self.header = Some(header);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::{ButtonMessage, ReplyButton};
    /// assert!(ButtonMessage::new("b", vec![ReplyButton::new("i", "t")]).footer("f").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Requires 1–3 valid buttons and bounded text.
    ///
    /// ```
    /// use meta_whatsapp::{ButtonMessage, ReplyButton};
    ///
    /// let too_many = vec![
    ///     ReplyButton::new("a", "A"),
    ///     ReplyButton::new("b", "B"),
    ///     ReplyButton::new("c", "C"),
    ///     ReplyButton::new("d", "D"),
    /// ];
    /// assert!(ButtonMessage::new("body", too_many).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty/oversized button list or bad text.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.body.text", &self.body)?;
        require_max_bytes("interactive.body.text", &self.body, CAPTION_MAX_BYTES)?;
        if let Some(header) = &self.header {
            header.validate()?;
        }
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        if self.buttons.is_empty() || self.buttons.len() > BUTTONS_MAX {
            return Err(Error::validation(
                "interactive.action.buttons",
                format!("must contain 1..={BUTTONS_MAX} buttons"),
            ));
        }
        for button in &self.buttons {
            button.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("button".into()));
        if let Some(header) = &self.header {
            interactive.insert(
                "header".into(),
                json!({ "type": "text", "text": header.text }),
            );
        }
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({ "buttons": self.buttons.iter().map(ReplyButton::render).collect::<Vec<_>>() }),
        );
        Value::Object(interactive)
    }
}

/// One selectable row in a list message.
///
/// ```
/// use meta_whatsapp::ListRow;
///
/// let row = ListRow::new("pizza", "Pizza").description("Mit Salami");
/// assert_eq!(row.title, "Pizza");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    /// Payload echoed in `list_reply.id`.
    pub id: String,
    /// Visible label (max 24 characters).
    pub title: String,
    /// Optional secondary line (max 72 characters).
    pub description: Option<String>,
}

impl ListRow {
    /// A row with just an id and title.
    ///
    /// ```
    /// use meta_whatsapp::ListRow;
    /// assert!(ListRow::new("id", "Titel").description.is_none());
    /// ```
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: None,
        }
    }

    /// Adds the description line.
    ///
    /// ```
    /// use meta_whatsapp::ListRow;
    /// assert_eq!(ListRow::new("i", "t").description("d").description.unwrap(), "d");
    /// ```
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Checks id, title and description bounds.
    ///
    /// ```
    /// use meta_whatsapp::ListRow;
    ///
    /// assert!(ListRow::new("i", "1234567890123456789012345").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for empty or oversized fields.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.action.sections.rows.id", &self.id)?;
        require_max_chars(
            "interactive.action.sections.rows.id",
            &self.id,
            BUTTON_ID_MAX_CHARS,
        )?;
        require_non_empty("interactive.action.sections.rows.title", &self.title)?;
        require_max_chars("interactive.action.sections.rows.title", &self.title, 24)?;
        if let Some(description) = &self.description {
            require_max_chars(
                "interactive.action.sections.rows.description",
                description,
                72,
            )?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        object.insert("id".into(), Value::String(self.id.clone()));
        object.insert("title".into(), Value::String(self.title.clone()));
        if let Some(description) = &self.description {
            object.insert("description".into(), Value::String(description.clone()));
        }
        Value::Object(object)
    }
}

/// A titled group of rows.
///
/// ```
/// use meta_whatsapp::{ListRow, ListSection};
///
/// let section = ListSection::new(vec![ListRow::new("a", "A")]).title("Beliebt");
/// assert_eq!(section.rows.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListSection {
    /// Optional section title (max 24 characters).
    pub title: Option<String>,
    /// Rows in this section.
    pub rows: Vec<ListRow>,
}

impl ListSection {
    /// A section with no title.
    ///
    /// ```
    /// use meta_whatsapp::{ListRow, ListSection};
    /// assert!(ListSection::new(vec![ListRow::new("i", "t")]).title.is_none());
    /// ```
    #[must_use]
    pub fn new(rows: Vec<ListRow>) -> Self {
        Self { title: None, rows }
    }

    /// Sets the section title.
    ///
    /// ```
    /// use meta_whatsapp::{ListRow, ListSection};
    /// assert!(ListSection::new(vec![ListRow::new("i", "t")]).title("X").title.is_some());
    /// ```
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    fn validate(&self) -> Result<()> {
        if let Some(title) = &self.title {
            require_max_chars("interactive.action.sections.title", title, 24)?;
        }
        if self.rows.is_empty() {
            return Err(Error::validation(
                "interactive.action.sections.rows",
                "a section needs at least one row",
            ));
        }
        for row in &self.rows {
            row.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        if let Some(title) = &self.title {
            object.insert("title".into(), Value::String(title.clone()));
        }
        object.insert(
            "rows".into(),
            Value::Array(self.rows.iter().map(ListRow::render).collect()),
        );
        Value::Object(object)
    }
}

/// Interactive list message: a button opening up to 10 rows.
///
/// ```
/// use meta_whatsapp::{ListMessage, ListRow, ListSection, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     ListMessage::new("Wählen Sie", "Speisekarte")
///         .section(ListSection::new(vec![ListRow::new("pizza", "Pizza")])),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "list");
/// assert_eq!(json["interactive"]["action"]["button"], "Speisekarte");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMessage {
    /// Optional text header.
    pub header: Option<HeaderText>,
    /// Body text.
    pub body: String,
    /// Optional footer text.
    pub footer: Option<String>,
    /// Label of the button that opens the list (max 20 characters).
    pub button_text: String,
    /// One to ten sections.
    pub sections: Vec<ListSection>,
}

impl ListMessage {
    /// Builds a list with no sections yet.
    ///
    /// ```
    /// use meta_whatsapp::ListMessage;
    /// assert!(ListMessage::new("Body", "Öffnen").sections.is_empty());
    /// ```
    pub fn new(body: impl Into<String>, button_text: impl Into<String>) -> Self {
        Self {
            header: None,
            body: body.into(),
            footer: None,
            button_text: button_text.into(),
            sections: Vec::new(),
        }
    }

    /// Appends a section.
    ///
    /// ```
    /// use meta_whatsapp::{ListMessage, ListRow, ListSection};
    /// let message = ListMessage::new("b", "btn")
    ///     .section(ListSection::new(vec![ListRow::new("i", "t")]));
    /// assert_eq!(message.sections.len(), 1);
    /// ```
    #[must_use]
    pub fn section(mut self, section: ListSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Adds a text header.
    ///
    /// ```
    /// use meta_whatsapp::{HeaderText, ListMessage};
    /// assert!(ListMessage::new("b", "btn").header(HeaderText::new("H")).header.is_some());
    /// ```
    #[must_use]
    pub fn header(mut self, header: HeaderText) -> Self {
        self.header = Some(header);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::ListMessage;
    /// assert!(ListMessage::new("b", "btn").footer("f").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Requires 1–10 sections, 1–10 rows in total and bounded text.
    ///
    /// ```
    /// use meta_whatsapp::{ListMessage, ListRow, ListSection};
    ///
    /// let rows: Vec<_> = (0..11).map(|n| ListRow::new(format!("{n}"), "R")).collect();
    /// assert!(ListMessage::new("b", "btn").section(ListSection::new(rows)).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty/oversized structure or bad text.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.body.text", &self.body)?;
        require_max_bytes("interactive.body.text", &self.body, CAPTION_MAX_BYTES)?;
        if let Some(header) = &self.header {
            header.validate()?;
        }
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        require_non_empty("interactive.action.button", &self.button_text)?;
        require_max_chars(
            "interactive.action.button",
            &self.button_text,
            BUTTON_TITLE_MAX_CHARS,
        )?;
        if self.sections.is_empty() || self.sections.len() > LIST_SECTIONS_MAX {
            return Err(Error::validation(
                "interactive.action.sections",
                format!("must contain 1..={LIST_SECTIONS_MAX} sections"),
            ));
        }
        for section in &self.sections {
            section.validate()?;
        }
        let total_rows: usize = self.sections.iter().map(|s| s.rows.len()).sum();
        if total_rows > LIST_ROWS_MAX {
            return Err(Error::validation(
                "interactive.action.sections.rows",
                format!("must contain at most {LIST_ROWS_MAX} rows in total, got {total_rows}"),
            ));
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("list".into()));
        if let Some(header) = &self.header {
            interactive.insert(
                "header".into(),
                json!({ "type": "text", "text": header.text }),
            );
        }
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({
                "button": self.button_text,
                "sections": self.sections.iter().map(ListSection::render).collect::<Vec<_>>(),
            }),
        );
        Value::Object(interactive)
    }
}

/// What a flow should do when the user taps the call-to-action.
///
/// ```
/// use meta_whatsapp::FlowAction;
///
/// let navigate = FlowAction::navigate("WELCOME");
/// assert!(matches!(navigate, FlowAction::Navigate { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FlowAction {
    /// Open a specific screen.
    Navigate {
        /// Screen name defined in the flow.
        screen: String,
        /// Optional initial data.
        data: Option<serde_json::Value>,
    },
    /// Let the flow talk to your endpoint.
    DataExchange,
}

impl FlowAction {
    /// A `navigate` action for `screen`.
    ///
    /// ```
    /// use meta_whatsapp::FlowAction;
    /// assert!(matches!(FlowAction::navigate("S"), FlowAction::Navigate { .. }));
    /// ```
    #[must_use]
    pub fn navigate(screen: impl Into<String>) -> Self {
        Self::Navigate {
            screen: screen.into(),
            data: None,
        }
    }

    fn validate(&self) -> Result<()> {
        match self {
            Self::Navigate { screen, .. } => require_non_empty(
                "interactive.action.parameters.flow_action_payload.screen",
                screen,
            ),
            Self::DataExchange => Ok(()),
        }
    }

    fn wire_name(&self) -> &'static str {
        match self {
            Self::Navigate { .. } => "navigate",
            Self::DataExchange => "data_exchange",
        }
    }

    fn payload(&self) -> Option<Value> {
        match self {
            Self::Navigate { screen, data } => {
                let mut object = Map::new();
                object.insert("screen".into(), Value::String(screen.clone()));
                if let Some(data) = data {
                    object.insert("data".into(), data.clone());
                }
                Some(Value::Object(object))
            }
            Self::DataExchange => None,
        }
    }
}

/// Whether a flow is published or still a draft.
///
/// ```
/// use meta_whatsapp::FlowMode;
/// assert_eq!(FlowMode::Published.as_str(), "published");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum FlowMode {
    /// Live version.
    #[default]
    Published,
    /// Draft version, only reachable by test numbers.
    Draft,
}

impl FlowMode {
    /// Wire representation.
    ///
    /// ```
    /// use meta_whatsapp::FlowMode;
    /// assert_eq!(FlowMode::Draft.as_str(), "draft");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Published => "published",
            Self::Draft => "draft",
        }
    }
}

/// Interactive flow message (`WhatsApp` Flows).
///
/// ```
/// use meta_whatsapp::{FlowAction, FlowMessage, MessageRequest};
///
/// let json = MessageRequest::new(
///     "1",
///     FlowMessage::new("flow-123", "Buchen", FlowAction::navigate("WELCOME")),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["action"]["parameters"]["flow_id"], "flow-123");
/// assert_eq!(json["interactive"]["action"]["parameters"]["flow_cta"], "Buchen");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowMessage {
    /// Optional text header.
    pub header: Option<HeaderText>,
    /// Body text.
    pub body: String,
    /// Optional footer text.
    pub footer: Option<String>,
    /// Flow id from the Meta dashboard.
    pub flow_id: String,
    /// Opaque token echoed back to your endpoint.
    pub flow_token: String,
    /// Call-to-action label (max 30 characters).
    pub cta: String,
    /// What happens on tap.
    pub action: FlowAction,
    /// Published or draft.
    pub mode: FlowMode,
}

impl FlowMessage {
    /// Builds a flow message; the body defaults to a generic prompt.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage};
    /// let message = FlowMessage::new("f", "Öffnen", FlowAction::navigate("S"));
    /// assert_eq!(message.mode, meta_whatsapp::FlowMode::Published);
    /// ```
    pub fn new(flow_id: impl Into<String>, cta: impl Into<String>, action: FlowAction) -> Self {
        Self {
            header: None,
            body: "Tap the button below to continue.".to_string(),
            footer: None,
            flow_id: flow_id.into(),
            flow_token: String::new(),
            cta: cta.into(),
            action,
            mode: FlowMode::Published,
        }
    }

    /// Overrides the body text.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage};
    /// assert_eq!(
    ///     FlowMessage::new("f", "c", FlowAction::DataExchange).body("Hallo").body,
    ///     "Hallo"
    /// );
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// Sets the opaque flow token.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage};
    /// assert_eq!(FlowMessage::new("f", "c", FlowAction::DataExchange).flow_token("t").flow_token, "t");
    /// ```
    #[must_use]
    pub fn flow_token(mut self, token: impl Into<String>) -> Self {
        self.flow_token = token.into();
        self
    }

    /// Adds a text header.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage, HeaderText};
    /// assert!(FlowMessage::new("f", "c", FlowAction::DataExchange).header(HeaderText::new("H")).header.is_some());
    /// ```
    #[must_use]
    pub fn header(mut self, header: HeaderText) -> Self {
        self.header = Some(header);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage};
    /// assert!(FlowMessage::new("f", "c", FlowAction::DataExchange).footer("f").footer.is_some());
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Selects the published or draft flow.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage, FlowMode};
    /// assert_eq!(
    ///     FlowMessage::new("f", "c", FlowAction::DataExchange).mode(FlowMode::Draft).mode,
    ///     FlowMode::Draft
    /// );
    /// ```
    #[must_use]
    pub fn mode(mut self, mode: FlowMode) -> Self {
        self.mode = mode;
        self
    }

    /// Checks ids, the CTA length and the action payload.
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage};
    ///
    /// assert!(FlowMessage::new("f", "c", FlowAction::DataExchange).validate().is_ok());
    /// assert!(FlowMessage::new("", "c", FlowAction::DataExchange).validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for empty ids or oversized text.
    pub fn validate(&self) -> Result<()> {
        require_non_empty("interactive.body.text", &self.body)?;
        require_max_bytes("interactive.body.text", &self.body, CAPTION_MAX_BYTES)?;
        if let Some(header) = &self.header {
            header.validate()?;
        }
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        require_non_empty("interactive.action.parameters.flow_id", &self.flow_id)?;
        require_max_chars("interactive.action.parameters.flow_cta", &self.cta, 30)?;
        require_non_empty("interactive.action.parameters.flow_cta", &self.cta)?;
        require_max_bytes(
            "interactive.action.parameters.flow_token",
            &self.flow_token,
            1024,
        )?;
        self.action.validate()
    }

    fn render(&self) -> Value {
        let mut parameters = Map::new();
        parameters.insert("flow_message_version".into(), Value::String("3".into()));
        parameters.insert("flow_id".into(), Value::String(self.flow_id.clone()));
        parameters.insert("flow_token".into(), Value::String(self.flow_token.clone()));
        parameters.insert("flow_cta".into(), Value::String(self.cta.clone()));
        parameters.insert("mode".into(), Value::String(self.mode.as_str().into()));
        parameters.insert(
            "flow_action".into(),
            Value::String(self.action.wire_name().into()),
        );
        if let Some(payload) = self.action.payload() {
            parameters.insert("flow_action_payload".into(), payload);
        }

        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("flow".into()));
        if let Some(header) = &self.header {
            interactive.insert(
                "header".into(),
                json!({ "type": "text", "text": header.text }),
            );
        }
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({ "name": "flow", "parameters": Value::Object(parameters) }),
        );
        Value::Object(interactive)
    }
}

/// One product inside a catalog, referenced by its retailer id.
///
/// ```
/// use meta_whatsapp::ProductItem;
///
/// let item = ProductItem::new("sku-42");
/// assert_eq!(item.product_retailer_id, "sku-42");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductItem {
    /// The retailer id (SKU) of the product in the catalog.
    pub product_retailer_id: String,
}

impl ProductItem {
    /// A product reference; the id is validated when the message renders.
    ///
    /// ```
    /// use meta_whatsapp::ProductItem;
    /// assert!(ProductItem::new("di9ozbzfi4").validate().is_ok());
    /// ```
    pub fn new(product_retailer_id: impl Into<String>) -> Self {
        Self {
            product_retailer_id: product_retailer_id.into(),
        }
    }

    /// Checks that the retailer id is present and printable.
    ///
    /// ```
    /// use meta_whatsapp::ProductItem;
    ///
    /// assert!(ProductItem::new("sku-42").validate().is_ok());
    /// assert!(ProductItem::new("").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty id or one containing control
    /// characters. The contract pins no maximum length, so none is enforced.
    pub fn validate(&self) -> Result<()> {
        require_identifier(
            "interactive.action.sections.product_items.product_retailer_id",
            &self.product_retailer_id,
        )
    }

    fn render(&self) -> Value {
        json!({ "product_retailer_id": self.product_retailer_id })
    }
}

/// A titled group of products in a multi-product message.
///
/// ```
/// use meta_whatsapp::{ProductItem, ProductSection};
///
/// let section = ProductSection::new(vec![ProductItem::new("sku-42")]).title("Neu");
/// assert_eq!(section.product_items.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSection {
    /// Optional title, at most [`PRODUCT_SECTION_TITLE_MAX_CHARS`] characters.
    ///
    /// Required once the message carries more than one section — see
    /// [`MultiProductMessage::validate`].
    pub title: Option<String>,
    /// Products in this section, 1..=[`PRODUCT_ITEMS_MAX`].
    pub product_items: Vec<ProductItem>,
}

impl ProductSection {
    /// A section without a title.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    /// assert!(ProductSection::new(vec![ProductItem::new("sku")]).title.is_none());
    /// ```
    #[must_use]
    pub fn new(items: Vec<ProductItem>) -> Self {
        Self {
            title: None,
            product_items: items,
        }
    }

    /// A section built from bare retailer ids.
    ///
    /// ```
    /// use meta_whatsapp::ProductSection;
    ///
    /// let section = ProductSection::from_ids(["sku-a", "sku-b"]);
    /// assert_eq!(section.product_items[1].product_retailer_id, "sku-b");
    /// ```
    #[must_use]
    pub fn from_ids<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            title: None,
            product_items: ids.into_iter().map(ProductItem::new).collect(),
        }
    }

    /// Sets the section title.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    ///
    /// let section = ProductSection::new(vec![ProductItem::new("sku")]).title("Angebote");
    /// assert_eq!(section.title.as_deref(), Some("Angebote"));
    /// ```
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Appends one product.
    ///
    /// ```
    /// use meta_whatsapp::{ProductItem, ProductSection};
    ///
    /// let section = ProductSection::new(Vec::new()).item(ProductItem::new("sku"));
    /// assert_eq!(section.product_items.len(), 1);
    /// ```
    #[must_use]
    pub fn item(mut self, item: ProductItem) -> Self {
        self.product_items.push(item);
        self
    }

    fn validate(&self) -> Result<()> {
        if let Some(title) = &self.title {
            require_non_empty("interactive.action.sections.title", title)?;
            require_max_chars(
                "interactive.action.sections.title",
                title,
                PRODUCT_SECTION_TITLE_MAX_CHARS,
            )?;
        }
        if self.product_items.is_empty() || self.product_items.len() > PRODUCT_ITEMS_MAX {
            return Err(Error::validation(
                "interactive.action.sections.product_items",
                format!("must contain 1..={PRODUCT_ITEMS_MAX} products"),
            ));
        }
        for item in &self.product_items {
            item.validate()?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut object = Map::new();
        if let Some(title) = &self.title {
            object.insert("title".into(), Value::String(title.clone()));
        }
        object.insert(
            "product_items".into(),
            Value::Array(self.product_items.iter().map(ProductItem::render).collect()),
        );
        Value::Object(object)
    }
}

/// Single-product message (`interactive.type = "product"`): one product card
/// from a catalog.
///
/// The contract defines no header for this message type, so the struct has no
/// header field — the wire cannot be asked for one.
///
/// ```
/// use meta_whatsapp::{MessageRequest, SingleProductMessage};
///
/// let json = MessageRequest::new(
///     "1",
///     SingleProductMessage::new("367025965434465", "sku-42").body("Nur heute 10 %"),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "product");
/// assert_eq!(json["interactive"]["action"]["catalog_id"], "367025965434465");
/// assert_eq!(json["interactive"]["action"]["product_retailer_id"], "sku-42");
/// assert_eq!(json["interactive"]["body"]["text"], "Nur heute 10 %");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleProductMessage {
    /// Catalog id linked to the `WhatsApp` Business Account.
    pub catalog_id: String,
    /// The product to show.
    pub product_retailer_id: String,
    /// Optional body line above the card, at most [`CAPTION_MAX_BYTES`] bytes.
    pub body: Option<String>,
    /// Optional footer, at most [`HEADER_MAX_CHARS`] characters.
    pub footer: Option<String>,
}

impl SingleProductMessage {
    /// A product message with no body and no footer.
    ///
    /// ```
    /// use meta_whatsapp::SingleProductMessage;
    ///
    /// let message = SingleProductMessage::new("catalog", "sku");
    /// assert!(message.body.is_none());
    /// assert!(message.footer.is_none());
    /// ```
    pub fn new(catalog_id: impl Into<String>, product_retailer_id: impl Into<String>) -> Self {
        Self {
            catalog_id: catalog_id.into(),
            product_retailer_id: product_retailer_id.into(),
            body: None,
            footer: None,
        }
    }

    /// Adds the body line. Optional for this message type only.
    ///
    /// ```
    /// use meta_whatsapp::SingleProductMessage;
    ///
    /// let message = SingleProductMessage::new("c", "s").body("Hallo");
    /// assert_eq!(message.body.as_deref(), Some("Hallo"));
    /// ```
    #[must_use]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::SingleProductMessage;
    ///
    /// let message = SingleProductMessage::new("c", "s").footer("Impressum");
    /// assert_eq!(message.footer.as_deref(), Some("Impressum"));
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Checks both ids and the optional text bounds.
    ///
    /// ```
    /// use meta_whatsapp::SingleProductMessage;
    ///
    /// assert!(SingleProductMessage::new("c", "s").validate().is_ok());
    /// assert!(SingleProductMessage::new("c", "").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first offending field.
    pub fn validate(&self) -> Result<()> {
        require_identifier("interactive.action.catalog_id", &self.catalog_id)?;
        require_identifier(
            "interactive.action.product_retailer_id",
            &self.product_retailer_id,
        )?;
        if let Some(body) = &self.body {
            require_non_empty("interactive.body.text", body)?;
            require_max_bytes("interactive.body.text", body, CAPTION_MAX_BYTES)?;
        }
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("product".into()));
        if let Some(body) = &self.body {
            interactive.insert("body".into(), json!({ "text": body }));
        }
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({
                "catalog_id": self.catalog_id,
                "product_retailer_id": self.product_retailer_id,
            }),
        );
        Value::Object(interactive)
    }
}

/// Multi-product message (`interactive.type = "product_list"`): up to 30
/// products in up to 10 sections.
///
/// Header and body are required for this type, so the constructor takes both.
///
/// ```
/// use meta_whatsapp::{MessageRequest, MultiProductMessage, ProductSection};
///
/// let json = MessageRequest::new(
///     "1",
///     MultiProductMessage::new("146265584024623", "Unser Katalog", "Bitte wählen")
///         .section(ProductSection::from_ids(["sku-a", "sku-b"]).title("Neu")),
/// )
/// .to_json()?;
/// assert_eq!(json["interactive"]["type"], "product_list");
/// assert_eq!(json["interactive"]["header"]["text"], "Unser Katalog");
/// assert_eq!(
///     json["interactive"]["action"]["sections"][0]["product_items"][1]["product_retailer_id"],
///     "sku-b"
/// );
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiProductMessage {
    /// Text header, required by the contract (at most [`HEADER_MAX_CHARS`]).
    pub header: HeaderText,
    /// Body text, at most [`CAPTION_MAX_BYTES`] bytes.
    pub body: String,
    /// Optional footer, at most [`HEADER_MAX_CHARS`] characters.
    pub footer: Option<String>,
    /// Catalog id linked to the `WhatsApp` Business Account.
    pub catalog_id: String,
    /// 1..=[`PRODUCT_SECTIONS_MAX`] sections.
    pub sections: Vec<ProductSection>,
}

impl MultiProductMessage {
    /// A multi-product message with no sections yet.
    ///
    /// ```
    /// use meta_whatsapp::MultiProductMessage;
    ///
    /// let message = MultiProductMessage::new("catalog", "Katalog", "Bitte wählen");
    /// assert!(message.sections.is_empty());
    /// ```
    pub fn new(
        catalog_id: impl Into<String>,
        header: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self {
            header: HeaderText::new(header),
            body: body.into(),
            footer: None,
            catalog_id: catalog_id.into(),
            sections: Vec::new(),
        }
    }

    /// Appends a section.
    ///
    /// ```
    /// use meta_whatsapp::{MultiProductMessage, ProductSection};
    ///
    /// let message = MultiProductMessage::new("c", "K", "B")
    ///     .section(ProductSection::from_ids(["sku"]));
    /// assert_eq!(message.sections.len(), 1);
    /// ```
    #[must_use]
    pub fn section(mut self, section: ProductSection) -> Self {
        self.sections.push(section);
        self
    }

    /// Adds a footer.
    ///
    /// ```
    /// use meta_whatsapp::MultiProductMessage;
    ///
    /// let message = MultiProductMessage::new("c", "K", "B").footer("Versand frei");
    /// assert_eq!(message.footer.as_deref(), Some("Versand frei"));
    /// ```
    #[must_use]
    pub fn footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Requires 1–10 sections, at most 30 products overall, a catalog id and
    /// bounded text.
    ///
    /// ```
    /// use meta_whatsapp::{MultiProductMessage, ProductSection};
    ///
    /// let ok = MultiProductMessage::new("catalog", "Katalog", "Bitte wählen")
    ///     .section(ProductSection::from_ids(["sku"]));
    /// assert!(ok.validate().is_ok());
    ///
    /// // A message without a section has nothing to show.
    /// assert!(MultiProductMessage::new("catalog", "Katalog", "Bitte wählen")
    ///     .validate()
    ///     .is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty/oversized structure, a missing
    /// section title when there is more than one section, or bad text.
    pub fn validate(&self) -> Result<()> {
        require_identifier("interactive.action.catalog_id", &self.catalog_id)?;
        self.header.validate()?;
        require_non_empty("interactive.body.text", &self.body)?;
        require_max_bytes("interactive.body.text", &self.body, CAPTION_MAX_BYTES)?;
        if let Some(footer) = &self.footer {
            require_max_chars("interactive.footer.text", footer, HEADER_MAX_CHARS)?;
        }
        if self.sections.is_empty() || self.sections.len() > PRODUCT_SECTIONS_MAX {
            return Err(Error::validation(
                "interactive.action.sections",
                format!("must contain 1..={PRODUCT_SECTIONS_MAX} sections"),
            ));
        }
        // The spec asks for a title as soon as the message has more than one
        // section; without it WhatsApp renders an unlabelled block.
        let multiple = self.sections.len() > 1;
        for section in &self.sections {
            section.validate()?;
            if multiple && section.title.is_none() {
                return Err(Error::validation(
                    "interactive.action.sections.title",
                    "is required when the message has more than one section",
                ));
            }
        }
        let total: usize = self
            .sections
            .iter()
            .map(|section| section.product_items.len())
            .sum();
        if total > PRODUCTS_MAX {
            return Err(Error::validation(
                "interactive.action.sections.product_items",
                format!("must contain at most {PRODUCTS_MAX} products in total, got {total}"),
            ));
        }
        Ok(())
    }

    fn render(&self) -> Value {
        let mut interactive = Map::new();
        interactive.insert("type".into(), Value::String("product_list".into()));
        interactive.insert(
            "header".into(),
            json!({ "type": "text", "text": self.header.text }),
        );
        interactive.insert("body".into(), json!({ "text": self.body }));
        if let Some(footer) = &self.footer {
            interactive.insert("footer".into(), json!({ "text": footer }));
        }
        interactive.insert(
            "action".into(),
            json!({
                "catalog_id": self.catalog_id,
                "sections": self.sections.iter().map(ProductSection::render).collect::<Vec<_>>(),
            }),
        );
        Value::Object(interactive)
    }
}

impl From<SingleProductMessage> for MessageBody {
    fn from(value: SingleProductMessage) -> Self {
        Self::Interactive(InteractiveMessage::Product(value))
    }
}

impl From<MultiProductMessage> for MessageBody {
    fn from(value: MultiProductMessage) -> Self {
        Self::Interactive(InteractiveMessage::ProductList(value))
    }
}

/// Any interactive message: buttons, list, flow or a catalog product message.
///
/// ```
/// use meta_whatsapp::{InteractiveMessage, ListMessage, MessageRequest};
///
/// let json = MessageRequest::new("1", InteractiveMessage::List(ListMessage::new("b", "c")))
///     .to_json();
/// // Validation still fails: a list needs at least one section.
/// assert!(json.is_err());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InteractiveMessage {
    /// Up to three quick-reply buttons.
    Buttons(ButtonMessage),
    /// Single-choice list with up to ten rows.
    List(ListMessage),
    /// `WhatsApp` Flow.
    Flow(FlowMessage),
    /// Address form (`address_message`, documented for India).
    Address(AddressMessage),
    /// One catalog product card.
    Product(SingleProductMessage),
    /// Up to 30 catalog products in up to 10 sections.
    ProductList(MultiProductMessage),
}

impl InteractiveMessage {
    /// Which interactive kind this is (`button`, `list`, `flow`,
    /// `address_message`, `product`, `product_list`).
    ///
    /// ```
    /// use meta_whatsapp::{FlowAction, FlowMessage, InteractiveMessage};
    ///
    /// let flow = FlowMessage::new("f", "c", FlowAction::DataExchange);
    /// assert_eq!(InteractiveMessage::Flow(flow).kind(), "flow");
    /// ```
    ///
    /// ```
    /// use meta_whatsapp::{InteractiveMessage, MultiProductMessage};
    ///
    /// let list = MultiProductMessage::new("catalog", "Katalog", "Bitte wählen");
    /// assert_eq!(InteractiveMessage::ProductList(list).kind(), "product_list");
    /// ```
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Buttons(_) => "button",
            Self::List(_) => "list",
            Self::Flow(_) => "flow",
            Self::Address(_) => "address_message",
            Self::Product(_) => "product",
            Self::ProductList(_) => "product_list",
        }
    }

    /// Validates the selected variant.
    ///
    /// ```
    /// use meta_whatsapp::{ButtonMessage, InteractiveMessage, ReplyButton};
    ///
    /// let message = InteractiveMessage::Buttons(ButtonMessage::new(
    ///     "Frage?",
    ///     vec![ReplyButton::new("yes", "Ja")],
    /// ));
    /// assert!(message.validate().is_ok());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from the inner message.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Buttons(message) => message.validate(),
            Self::List(message) => message.validate(),
            Self::Flow(message) => message.validate(),
            Self::Address(message) => message.validate(),
            Self::Product(message) => message.validate(),
            Self::ProductList(message) => message.validate(),
        }
    }

    fn render(&self) -> Value {
        match self {
            Self::Buttons(message) => message.render(),
            Self::List(message) => message.render(),
            Self::Flow(message) => message.render(),
            Self::Address(message) => message.render(),
            Self::Product(message) => message.render(),
            Self::ProductList(message) => message.render(),
        }
    }
}

impl From<ButtonMessage> for InteractiveMessage {
    fn from(value: ButtonMessage) -> Self {
        Self::Buttons(value)
    }
}

impl From<ListMessage> for InteractiveMessage {
    fn from(value: ListMessage) -> Self {
        Self::List(value)
    }
}

impl From<FlowMessage> for InteractiveMessage {
    fn from(value: FlowMessage) -> Self {
        Self::Flow(value)
    }
}

impl From<AddressMessage> for InteractiveMessage {
    fn from(value: AddressMessage) -> Self {
        Self::Address(value)
    }
}

impl From<SingleProductMessage> for InteractiveMessage {
    fn from(value: SingleProductMessage) -> Self {
        Self::Product(value)
    }
}

impl From<MultiProductMessage> for InteractiveMessage {
    fn from(value: MultiProductMessage) -> Self {
        Self::ProductList(value)
    }
}

fn render_media(media: &MediaRef, extra: &[(&'static str, Option<&str>)]) -> Value {
    let mut object = match media.to_json() {
        Value::Object(map) => map,
        other => return other,
    };
    for (key, value) in extra {
        if let Some(value) = value {
            object.insert((*key).to_string(), Value::String((*value).to_string()));
        }
    }
    Value::Object(object)
}

fn validate_optional_caption(field: &'static str, caption: Option<&str>) -> Result<()> {
    match caption {
        Some(caption) => require_max_bytes(field, caption, CAPTION_MAX_BYTES),
        None => Ok(()),
    }
}

pub(crate) fn require_non_empty(field: &'static str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::validation(field, "must not be empty"));
    }
    Ok(())
}

/// A catalog id or retailer id: present and printable, no length rule because
/// neither the spec nor the reference pins one.
fn require_identifier(field: &'static str, value: &str) -> Result<()> {
    require_non_empty(field, value)?;
    require_no_controls(field, value)
}

pub(crate) fn require_max_bytes(field: &'static str, value: &str, max: usize) -> Result<()> {
    if value.len() > max {
        return Err(Error::validation(
            field,
            format!("must be at most {max} bytes, got {}", value.len()),
        ));
    }
    Ok(())
}

pub(crate) fn require_max_chars(field: &'static str, value: &str, max: usize) -> Result<()> {
    let count = value.chars().count();
    if count > max {
        return Err(Error::validation(
            field,
            format!("must be at most {max} characters, got {count}"),
        ));
    }
    Ok(())
}

pub(crate) fn require_no_controls(field: &'static str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(Error::validation(
            field,
            "must not contain control characters",
        ));
    }
    Ok(())
}

fn require_digits(field: &'static str, value: &str) -> Result<()> {
    require_non_empty(field, value)?;
    if value.len() > 32 {
        return Err(Error::validation(field, "must be at most 32 digits"));
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(Error::validation(
            field,
            "must contain digits only (no '+', spaces or dashes)",
        ));
    }
    Ok(())
}
