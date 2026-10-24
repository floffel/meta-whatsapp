//! Media references, uploads, downloads and the limits Meta enforces.
//!
//! Sending media works in two steps: upload the bytes once to `/media`, then
//! reference the returned id from any number of messages. Both spellings are
//! modelled by [`MediaRef`]: an uploaded id, or a public https link Meta
//! fetches itself.
//!
//! ```
//! use meta_whatsapp::{MediaRef, MediaUpload};
//!
//! // Upload first, then reuse the id (recommended for anything but one-offs).
//! let upload = MediaUpload::new("menu.pdf", "application/pdf", b"%PDF-1.7".to_vec())?;
//! assert_eq!(upload.mime_type, "application/pdf");
//!
//! let by_id = MediaRef::id("1234567890");
//! let by_link = MediaRef::link("https://example.com/menu.pdf");
//! by_id.validate()?;
//! by_link.validate()?;
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Longest media id the crate will put into a URL path.
pub const MAX_MEDIA_ID_LEN: usize = 256;
/// Longest filename accepted for a document upload.
pub const MAX_FILENAME_LEN: usize = 240;
/// Longest link accepted for `link`-based media.
pub const MAX_LINK_LEN: usize = 2048;

/// Upload ceiling for images (JPEG/PNG).
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
/// Upload ceiling for video.
pub const MAX_VIDEO_BYTES: u64 = 16 * 1024 * 1024;
/// Upload ceiling for audio.
pub const MAX_AUDIO_BYTES: u64 = 16 * 1024 * 1024;
/// Upload ceiling for documents.
pub const MAX_DOCUMENT_BYTES: u64 = 100 * 1024 * 1024;
/// Upload ceiling for stickers (animated webp included).
pub const MAX_STICKER_BYTES: u64 = 500 * 1024;

/// MIME types the Cloud API accepts, mapped to their upload ceiling.
const MIME_LIMITS: &[(&str, u64)] = &[
    ("image/jpeg", MAX_IMAGE_BYTES),
    ("image/png", MAX_IMAGE_BYTES),
    ("image/webp", MAX_STICKER_BYTES),
    ("video/mp4", MAX_VIDEO_BYTES),
    ("video/3gpp", MAX_VIDEO_BYTES),
    ("audio/aac", MAX_AUDIO_BYTES),
    ("audio/mp4", MAX_AUDIO_BYTES),
    ("audio/mpeg", MAX_AUDIO_BYTES),
    ("audio/amr", MAX_AUDIO_BYTES),
    ("audio/ogg", MAX_AUDIO_BYTES),
    ("application/pdf", MAX_DOCUMENT_BYTES),
    ("application/msword", MAX_DOCUMENT_BYTES),
    (
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        MAX_DOCUMENT_BYTES,
    ),
    ("application/vnd.ms-excel", MAX_DOCUMENT_BYTES),
    (
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        MAX_DOCUMENT_BYTES,
    ),
    ("application/vnd.ms-powerpoint", MAX_DOCUMENT_BYTES),
    (
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        MAX_DOCUMENT_BYTES,
    ),
    ("text/plain", MAX_DOCUMENT_BYTES),
];

/// Normalises a MIME type for lookups: lower-case, parameters stripped.
///
/// ```
/// use meta_whatsapp::media::base_mime;
///
/// assert_eq!(base_mime("Audio/OGG; codecs=opus"), "audio/ogg");
/// ```
#[must_use]
pub fn base_mime(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

/// The upload ceiling for `mime`, or `None` when the type is not accepted.
///
/// ```
/// use meta_whatsapp::media::max_bytes_for_mime;
///
/// assert_eq!(max_bytes_for_mime("image/png"), Some(5 * 1024 * 1024));
/// assert_eq!(max_bytes_for_mime("application/x-msdownload"), None);
/// ```
#[must_use]
pub fn max_bytes_for_mime(mime: &str) -> Option<u64> {
    let mime = base_mime(mime);
    MIME_LIMITS
        .iter()
        .find(|(known, _)| *known == mime)
        .map(|(_, limit)| *limit)
}

/// Rejects media ids that could escape the `/media/{id}` path or smuggle a
/// query string.
///
/// Meta ids are numeric today; the charset is deliberately wider so a future
/// id format does not break callers, but `/`, `.`, `?`, `#`, `%` and spaces are
/// never allowed. An empty id is rejected — `GET /media/` would list media.
///
/// ```
/// use meta_whatsapp::media::validate_media_id;
///
/// assert!(validate_media_id("1234567890").is_ok());
/// assert!(validate_media_id("../../v21.0/me").is_err());
/// assert!(validate_media_id("").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] when the id is empty, too long, or contains anything
/// but ASCII alphanumerics, `-` and `_`.
pub fn validate_media_id(id: &str) -> Result<()> {
    if id.is_empty() {
        return Err(Error::validation("media_id", "must not be empty"));
    }
    if id.len() > MAX_MEDIA_ID_LEN {
        return Err(Error::validation(
            "media_id",
            format!("must be at most {MAX_MEDIA_ID_LEN} bytes, got {}", id.len()),
        ));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(Error::validation(
            "media_id",
            "only ASCII alphanumerics, '-' and '_' are allowed",
        ));
    }
    Ok(())
}

/// How a message points at media: an uploaded id, or a public https link.
///
/// ```
/// use meta_whatsapp::MediaRef;
///
/// let image = MediaRef::id("987654321");
/// assert_eq!(image.as_id(), Some("987654321"));
///
/// let link = MediaRef::link("https://cdn.example.com/cat.png");
/// assert!(link.validate().is_ok());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaRef {
    /// An id returned by [`crate::WhatsAppClient::upload_media`].
    Id(String),
    /// A public https URL Meta downloads at send time.
    Link(String),
}

impl MediaRef {
    /// References already-uploaded media by id.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// assert!(matches!(MediaRef::id("1"), MediaRef::Id(_)));
    /// ```
    #[must_use]
    pub fn id(id: impl Into<String>) -> Self {
        Self::Id(id.into())
    }

    /// References media by public URL.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// assert!(MediaRef::link("https://example.com/a.png").as_link().is_some());
    /// ```
    #[must_use]
    pub fn link(url: impl Into<String>) -> Self {
        Self::Link(url.into())
    }

    /// The id, when this reference is an id.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// assert_eq!(MediaRef::id("42").as_id(), Some("42"));
    /// ```
    #[must_use]
    pub fn as_id(&self) -> Option<&str> {
        match self {
            Self::Id(id) => Some(id),
            Self::Link(_) => None,
        }
    }

    /// The link, when this reference is a link.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// assert_eq!(MediaRef::link("https://x.test/a").as_link(), Some("https://x.test/a"));
    /// ```
    #[must_use]
    pub fn as_link(&self) -> Option<&str> {
        match self {
            Self::Link(url) => Some(url),
            Self::Id(_) => None,
        }
    }

    /// The JSON key this reference occupies: `id` or `link`.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// assert_eq!(MediaRef::id("1").field_name(), "id");
    /// assert_eq!(MediaRef::link("https://x.test/a").field_name(), "link");
    /// ```
    #[must_use]
    pub fn field_name(&self) -> &'static str {
        match self {
            Self::Id(_) => "id",
            Self::Link(_) => "link",
        }
    }

    /// Checks the id charset, or that the link is an absolute https URL of
    /// sane length with no whitespace.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    ///
    /// assert!(MediaRef::link("http://insecure.test/a.png").validate().is_err());
    /// assert!(MediaRef::link("https://x.test/a b.png").validate().is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for empty/oversized/unsafe values.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Id(id) => validate_media_id(id),
            Self::Link(url) => {
                if !url.starts_with("https://") {
                    return Err(Error::validation(
                        "media.link",
                        "must be an absolute https:// URL",
                    ));
                }
                if url.len() > MAX_LINK_LEN {
                    return Err(Error::validation(
                        "media.link",
                        format!("must be at most {MAX_LINK_LEN} bytes"),
                    ));
                }
                if url.chars().any(char::is_whitespace) {
                    return Err(Error::validation(
                        "media.link",
                        "must not contain whitespace",
                    ));
                }
                if url.len() == "https://".len() {
                    return Err(Error::validation("media.link", "must contain a host"));
                }
                Ok(())
            }
        }
    }

    /// Renders `{"id": "..."}` or `{"link": "..."}`.
    ///
    /// ```
    /// use meta_whatsapp::MediaRef;
    /// use serde_json::json;
    ///
    /// assert_eq!(MediaRef::id("7").to_json(), json!({"id": "7"}));
    /// ```
    #[must_use]
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ self.field_name(): self.value() })
    }

    fn value(&self) -> &str {
        match self {
            Self::Id(v) | Self::Link(v) => v,
        }
    }
}

/// Bytes of a media upload plus the metadata the multipart body needs.
///
/// Construction validates eagerly ([`MediaUpload::new`]), so an oversized or
/// unsupported file never reaches the network. `mime_type` must be one of the
/// types [`max_bytes_for_mime`] knows about.
///
/// ```
/// use meta_whatsapp::MediaUpload;
///
/// let upload = MediaUpload::new("cat.png", "image/png", vec![0x89, b'P', b'N', b'G'])?;
/// assert_eq!(upload.filename, "cat.png");
///
/// // 5 MB + 1 byte is rejected before any request is built.
/// let too_big = vec![0_u8; (5 * 1024 * 1024) + 1];
/// assert!(MediaUpload::new("big.jpg", "image/jpeg", too_big).is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaUpload {
    /// Raw file bytes.
    pub data: Vec<u8>,
    /// Filename Meta stores alongside the media.
    pub filename: String,
    /// MIME type, e.g. `image/png`.
    pub mime_type: String,
}

impl MediaUpload {
    /// Validates and wraps `data`.
    ///
    /// ```
    /// use meta_whatsapp::MediaUpload;
    ///
    /// assert!(MediaUpload::new("a.pdf", "application/pdf", b"x".to_vec()).is_ok());
    /// assert!(MediaUpload::new("a.exe", "application/x-msdownload", b"x".to_vec()).is_err());
    /// assert!(MediaUpload::new("", "image/png", b"x".to_vec()).is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty/oversized payload, an empty or
    /// header-unsafe filename, or an unsupported MIME type.
    pub fn new(
        filename: impl Into<String>,
        mime_type: impl Into<String>,
        data: impl Into<Vec<u8>>,
    ) -> Result<Self> {
        let upload = Self {
            data: data.into(),
            filename: filename.into(),
            mime_type: mime_type.into(),
        };
        upload.validate()?;
        Ok(upload)
    }

    /// Re-checks the invariants enforced by [`MediaUpload::new`].
    ///
    /// ```
    /// use meta_whatsapp::MediaUpload;
    ///
    /// let mut upload = MediaUpload::new("a.png", "image/png", vec![1])?;
    /// upload.data.clear();
    /// assert!(upload.validate().is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// See [`MediaUpload::new`].
    pub fn validate(&self) -> Result<()> {
        if self.filename.is_empty() {
            return Err(Error::validation("media.filename", "must not be empty"));
        }
        if self.filename.len() > MAX_FILENAME_LEN {
            return Err(Error::validation(
                "media.filename",
                format!("must be at most {MAX_FILENAME_LEN} bytes"),
            ));
        }
        if self
            .filename
            .chars()
            .any(|c| c.is_control() || c == '"' || c == '/' || c == '\\')
        {
            return Err(Error::validation(
                "media.filename",
                "must not contain control characters, quotes or path separators",
            ));
        }
        if self.data.is_empty() {
            return Err(Error::validation("media.data", "must not be empty"));
        }
        let limit = max_bytes_for_mime(&self.mime_type).ok_or_else(|| {
            Error::validation(
                "media.mime_type",
                format!(
                    "unsupported type {:?}; see media::max_bytes_for_mime",
                    base_mime(&self.mime_type)
                ),
            )
        })?;
        let size = self.data.len() as u64;
        if size > limit {
            return Err(Error::validation(
                "media.data",
                format!("{size} bytes exceeds the {limit} byte limit for this type"),
            ));
        }
        Ok(())
    }

    /// Length of the payload in bytes.
    ///
    /// ```
    /// use meta_whatsapp::MediaUpload;
    /// assert_eq!(MediaUpload::new("a.png", "image/png", b"123".to_vec())?.len(), 3);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Whether the payload is empty. Always `false` for a validated upload —
    /// kept so clippy's `len_without_is_empty` stays quiet.
    ///
    /// ```
    /// use meta_whatsapp::MediaUpload;
    /// assert!(!MediaUpload::new("a.png", "image/png", b"1".to_vec())?.is_empty());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Encodes the `multipart/form-data` body: `messaging_product` followed by
    /// the `file` part. The returned string is the full `Content-Type` header
    /// value including the boundary.
    ///
    /// Custom [`crate::Transport`] implementations use this so the encoding
    /// stays identical to the bundled `reqwest` one.
    ///
    /// ```
    /// use meta_whatsapp::MediaUpload;
    ///
    /// let upload = MediaUpload::new("a.txt", "text/plain", b"hi".to_vec())?;
    /// let (content_type, body) = upload.encode();
    /// assert!(content_type.starts_with("multipart/form-data; boundary="));
    /// assert!(body.starts_with(b"--"));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn encode(&self) -> (String, Vec<u8>) {
        let boundary = next_boundary(&self.data);
        let content_type = format!("multipart/form-data; boundary={boundary}");
        (content_type, encode_multipart(&boundary, self))
    }
}

/// Renders the multipart body. Pure, so tests can assert the exact bytes.
pub(crate) fn encode_multipart(boundary: &str, upload: &MediaUpload) -> Vec<u8> {
    let mut body = Vec::with_capacity(upload.data.len() + 512);

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        b"Content-Disposition: form-data; name=\"messaging_product\"\r\n\r\nwhatsapp\r\n",
    );
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n",
            upload.filename
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", upload.mime_type).as_bytes());
    body.extend_from_slice(&upload.data);
    body.extend_from_slice(b"\r\n");
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// A boundary that cannot appear in the payload: it hashes the payload *and* a
/// process-wide counter, so identical uploads still get distinct boundaries.
fn next_boundary(data: &[u8]) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.update(n.to_le_bytes());
    format!("meta-whatsapp-{}", hex::encode(&hasher.finalize()[..16]))
}

/// A downloaded media object: the bytes as received plus the content type the
/// CDN served.
///
/// ```
/// use meta_whatsapp::MediaBytes;
///
/// let media = MediaBytes::new(b"\x89PNG".to_vec(), Some("image/png".into()));
/// assert_eq!(media.len(), 4);
/// assert_eq!(media.content_type(), Some("image/png"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBytes {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

impl MediaBytes {
    /// Wraps downloaded bytes.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// assert!(MediaBytes::new(Vec::new(), None).is_empty());
    /// ```
    #[must_use]
    pub fn new(bytes: Vec<u8>, content_type: Option<String>) -> Self {
        Self {
            bytes,
            content_type,
        }
    }

    /// The raw bytes.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// assert_eq!(MediaBytes::new(b"abc".to_vec(), None).bytes(), b"abc");
    /// ```
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consumes the value, returning the bytes.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// assert_eq!(MediaBytes::new(b"abc".to_vec(), None).into_bytes(), b"abc");
    /// ```
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The `Content-Type` the CDN sent, lower-cased MIME part only.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// let media = MediaBytes::new(Vec::new(), Some("image/png; charset=binary".into()));
    /// assert_eq!(media.content_type(), Some("image/png"));
    /// ```
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref().map(|raw| {
            // `base_mime` allocates; map on the fly and keep the borrow.
            let end = raw.find(';').unwrap_or(raw.len());
            raw[..end].trim()
        })
    }

    /// Number of bytes.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// assert_eq!(MediaBytes::new(vec![0; 7], None).len(), 7);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Whether the body was empty.
    ///
    /// ```
    /// use meta_whatsapp::MediaBytes;
    /// assert!(MediaBytes::new(Vec::new(), None).is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// Metadata for an uploaded media id: `GET /{version}/{media_id}`.
///
/// ```
/// use meta_whatsapp::MediaInfo;
///
/// let info: MediaInfo = serde_json::from_str(
///     r#"{"id":"1","url":"https://lookaside.fbsbx.com/whatsapp/1","mime_type":"image/png","sha256":"abc","file_size":12}"#,
/// )?;
/// assert_eq!(info.mime_type.as_deref(), Some("image/png"));
/// assert_eq!(info.file_size, Some(12));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MediaInfo {
    /// Media id.
    #[serde(default)]
    pub id: String,
    /// Short-lived CDN URL; fetch it with the access token.
    #[serde(default, alias = "url")]
    pub url: Option<String>,
    /// MIME type as stored by Meta.
    #[serde(default, alias = "mimeType")]
    pub mime_type: Option<String>,
    /// SHA-256 of the file, hex encoded.
    #[serde(default, alias = "sha256")]
    pub sha256: Option<String>,
    /// Size in bytes.
    #[serde(default, alias = "fileSize")]
    pub file_size: Option<u64>,
}

/// Parses a `Retry-After` header value: integer seconds only.
///
/// HTTP-date values are ignored (returns `None`) rather than guessed at —
/// the retry policy then falls back to exponential backoff.
///
/// ```
/// use std::time::Duration;
/// use meta_whatsapp::media::parse_retry_after;
///
/// assert_eq!(parse_retry_after("7"), Some(Duration::from_secs(7)));
/// assert_eq!(parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"), None);
/// assert_eq!(parse_retry_after("-1"), None);
/// ```
#[must_use]
pub fn parse_retry_after(value: &str) -> Option<std::time::Duration> {
    value
        .trim()
        .parse::<u64>()
        .ok()
        .map(std::time::Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_body_contains_both_parts_and_a_closing_boundary() {
        let upload = MediaUpload::new("a.txt", "text/plain", b"hello".to_vec()).unwrap();
        let body = encode_multipart("BOUND", &upload);
        let text = String::from_utf8(body).unwrap();

        assert_eq!(text.matches("--BOUND").count(), 3, "{text}");
        assert!(text.contains("name=\"messaging_product\"\r\n\r\nwhatsapp\r\n"));
        assert!(text.contains("filename=\"a.txt\""));
        assert!(text.contains("Content-Type: text/plain"));
        assert!(text.contains("\r\n\r\nhello\r\n"));
        assert!(text.ends_with("--BOUND--\r\n"));
    }

    #[test]
    fn boundaries_differ_even_for_identical_payloads() {
        let a = next_boundary(b"same");
        let b = next_boundary(b"same");
        assert_ne!(a, b);
        assert!(a.starts_with("meta-whatsapp-"));
    }

    #[test]
    fn content_type_is_truncated_at_parameters() {
        let media = MediaBytes::new(Vec::new(), Some("IMAGE/PNG; charset=x".into()));
        assert_eq!(media.content_type(), Some("IMAGE/PNG"));
    }

    #[test]
    fn retry_after_rejects_non_numeric_values() {
        assert_eq!(
            parse_retry_after(" 12 "),
            Some(std::time::Duration::from_secs(12))
        );
        assert_eq!(parse_retry_after("1.5"), None);
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("18446744073709551616"), None);
    }

    #[test]
    fn filename_header_injection_is_rejected() {
        let err = MediaUpload::new("a\r\nX: y", "text/plain", b"1".to_vec()).unwrap_err();
        assert!(matches!(err, Error::Validation { .. }));
    }

    #[test]
    fn media_ids_stop_at_the_boundary() {
        assert!(validate_media_id(&"a".repeat(MAX_MEDIA_ID_LEN)).is_ok());
        assert!(validate_media_id(&"a".repeat(MAX_MEDIA_ID_LEN + 1)).is_err());
    }
}
