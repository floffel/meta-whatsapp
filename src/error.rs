//! Error type shared by the webhook and client sides.

use std::time::Duration;

use thiserror::Error;

/// Everything that can go wrong while verifying a webhook or calling the API.
///
/// The enum is `#[non_exhaustive]`: new failure modes may be added without a
/// breaking release. Match with a `_` arm.
///
/// ```
/// use meta_whatsapp::{Error, WebhookVerifier};
///
/// let verifier = WebhookVerifier::new("secret")?;
/// let err = verifier.verify(b"{}", "sha1=deadbeef").unwrap_err();
/// match err {
///     Error::MalformedSignature(header) => assert_eq!(header, "sha1=deadbeef"),
///     other => panic!("unexpected: {other}"),
/// }
/// # Ok::<(), Error>(())
/// ```
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    /// The configured app secret is empty — every signature would verify.
    #[error("app secret must not be empty")]
    EmptySecret,

    /// The signature header was missing or not parseable as `sha256=<hex>`.
    #[error("malformed signature header: {0}")]
    MalformedSignature(String),

    /// The computed HMAC did not match the header.
    #[error("signature mismatch")]
    SignatureMismatch,

    /// A correctly signed payload carries a timestamp outside the replay window.
    ///
    /// Either the payload is old (a replay) or its clock is further ahead than
    /// the tolerance allows. Both fail closed.
    #[error(
        "payload timestamp {timestamp} is {skew_secs}s outside the {tolerance_secs}s replay window"
    )]
    Replay {
        /// The newest unix timestamp found in the payload, in seconds.
        timestamp: u64,
        /// Absolute distance between that timestamp and the caller's clock.
        skew_secs: u64,
        /// Configured tolerance, in seconds.
        tolerance_secs: u64,
    },

    /// JSON parsing failed. Keeps serde's line/column information as `source`.
    #[error("invalid payload: {source}")]
    Json {
        /// The underlying serde error.
        #[from]
        source: serde_json::Error,
    },

    /// The body was readable but not the expected shape (for example non-UTF-8
    /// bytes, or a JSON value where an object is required).
    #[error("invalid payload: {0}")]
    Payload(String),

    /// An argument would be rejected by the API, or is unsafe to interpolate
    /// into a URL. The request is never sent.
    ///
    /// ```
    /// use meta_whatsapp::{Error, MessageRequest, TextMessage};
    ///
    /// let err = MessageRequest::new("4915112345678", TextMessage::new(""))
    ///     .to_json()
    ///     .unwrap_err();
    /// assert!(matches!(err, Error::Validation { field: "text.body", .. }));
    /// ```
    #[error("invalid {field}: {message}")]
    Validation {
        /// Dotted path of the offending field, e.g. `text.body`.
        field: &'static str,
        /// What is wrong and what the accepted range is.
        message: String,
    },

    /// The transport failed before a usable response existed (DNS, TLS,
    /// timeout, connection reset). `retryable` says whether another attempt
    /// could plausibly succeed.
    #[error("transport error: {message}")]
    Transport {
        /// Transport supplied description.
        message: String,
        /// `true` for transient failures (I/O, timeouts), `false` otherwise.
        retryable: bool,
    },

    /// The Graph API answered with an error envelope, or with a non-2xx body
    /// that is not JSON.
    #[error("graph api error {code} (http {status}): {message}")]
    Api {
        /// Numeric Graph API error code (`0` when the body carried none).
        code: i64,
        /// HTTP status of the response.
        status: u16,
        /// Human readable message from Meta, truncated to 512 characters.
        message: String,
        /// Parsed `Retry-After` header, when present.
        retry_after: Option<Duration>,
        /// `true` for 429 and 5xx — the only statuses worth retrying.
        retryable: bool,
    },
}

impl Error {
    /// Builds a validation error. `field` is the dotted path, `message` the
    /// human readable detail.
    ///
    /// ```
    /// use meta_whatsapp::{Error, Result};
    ///
    /// fn check(n: usize) -> Result<()> {
    ///     if n == 0 {
    ///         return Err(Error::validation("buttons", "need at least one"));
    ///     }
    ///     Ok(())
    /// }
    /// assert!(check(0).is_err());
    /// ```
    #[must_use]
    pub fn validation(field: &'static str, message: impl Into<String>) -> Self {
        Self::Validation {
            field,
            message: message.into(),
        }
    }

    /// Builds a transient transport error (retryable).
    ///
    /// ```
    /// use meta_whatsapp::Error;
    /// let err = Error::transport("connection reset");
    /// assert!(err.is_retryable());
    /// ```
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
            retryable: true,
        }
    }

    /// Builds a terminal transport error (not retryable).
    ///
    /// ```
    /// use meta_whatsapp::Error;
    /// let err = Error::transport_terminal("unsupported scheme");
    /// assert!(!err.is_retryable());
    /// ```
    #[must_use]
    pub fn transport_terminal(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
            retryable: false,
        }
    }

    /// Whether another attempt could succeed. Signature, payload and
    /// validation errors are terminal by construction.
    ///
    /// ```
    /// use meta_whatsapp::{Error, RetryPolicy};
    ///
    /// let policy = RetryPolicy::default();
    /// assert!(policy.delay(1, None).is_some());
    /// assert!(Error::transport("dns").is_retryable());
    /// ```
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { retryable, .. } | Self::Api { retryable, .. } => *retryable,
            _ => false,
        }
    }

    /// The server requested wait, when it sent a `Retry-After` header.
    ///
    /// ```
    /// use meta_whatsapp::Error;
    /// let err = Error::Api {
    ///     code: 4,
    ///     status: 429,
    ///     message: "rate limited".into(),
    ///     retry_after: Some(std::time::Duration::from_secs(7)),
    ///     retryable: true,
    /// };
    /// assert_eq!(err.retry_after().unwrap().as_secs(), 7);
    /// ```
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::Api { retry_after, .. } => *retry_after,
            _ => None,
        }
    }
}

/// Convenience alias used across the crate.
///
/// ```
/// fn always_ok() -> meta_whatsapp::Result<u8> {
///     Ok(1)
/// }
/// assert_eq!(always_ok().unwrap(), 1);
/// ```
pub type Result<T> = std::result::Result<T, Error>;
