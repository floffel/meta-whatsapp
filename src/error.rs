//! Error type shared by the webhook and client sides.

use std::time::Duration;

use thiserror::Error;

/// Where in a request's lifecycle a transport failure happened.
///
/// The distinction is what makes a retry *safe* rather than merely *plausible*:
/// a failure before the request was written cannot have produced an effect, one
/// after the write may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportPhase {
    /// The connection was never established — DNS, TCP connect or the TLS
    /// handshake failed. Nothing reached the server, so a retry is safe even
    /// for a non-idempotent request.
    BeforeWrite,
    /// The request was written but no usable answer arrived (timeout,
    /// connection reset, truncated body). The server may or may not have acted
    /// on it: the outcome is ambiguous.
    AfterWrite,
    /// Permanently broken for this request (unparseable URL, unsupported
    /// scheme, request body that could not be built). Retrying cannot help.
    Terminal,
}

impl TransportPhase {
    /// `true` only for [`TransportPhase::BeforeWrite`].
    ///
    /// ```
    /// use meta_whatsapp::error::TransportPhase;
    ///
    /// assert!(TransportPhase::BeforeWrite.is_before_write());
    /// assert!(!TransportPhase::AfterWrite.is_before_write());
    /// assert!(!TransportPhase::Terminal.is_before_write());
    /// ```
    #[must_use]
    pub fn is_before_write(self) -> bool {
        matches!(self, Self::BeforeWrite)
    }

    /// `true` only for [`TransportPhase::AfterWrite`] — the request may have
    /// been processed.
    ///
    /// ```
    /// use meta_whatsapp::error::TransportPhase;
    ///
    /// assert!(TransportPhase::AfterWrite.is_after_write());
    /// assert!(!TransportPhase::BeforeWrite.is_after_write());
    /// ```
    #[must_use]
    pub fn is_after_write(self) -> bool {
        matches!(self, Self::AfterWrite)
    }
}

/// Everything that can go wrong while verifying a webhook or calling the API.
///
/// The enum is `#[non_exhaustive]`: new failure modes may be added without a
/// breaking release. Match with a `_` arm.
///
/// ```rust
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

    /// A Flow data-exchange key, ciphertext or payload was invalid.
    ///
    /// The message never includes private-key material or plaintext. Treat it
    /// as terminal: a failed authentication cannot become valid on retry.
    #[error("flow data exchange error: {0}")]
    Flow(String),

    /// An argument would be rejected by the API, or is unsafe to interpolate
    /// into a URL. The request is never sent.
    ///
    /// ```rust
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
    /// timeout, connection reset). [`Error::transport_phase`] says whether
    /// anything can have reached the server.
    #[error("transport error ({phase:?}): {message}")]
    Transport {
        /// Transport supplied description.
        message: String,
        /// Where in the request lifecycle the failure happened.
        phase: TransportPhase,
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
        /// `true` for 429 always, and for 5xx only when the request is
        /// idempotent. A 5xx on a send turns into [`Error::Ambiguous`].
        retryable: bool,
    },

    /// The request was written, so the server may have processed it, but no
    /// usable answer came back — a 5xx on a non-idempotent request, or a
    /// timeout/connection reset after the write.
    ///
    /// Repeating the request could duplicate the effect, so
    /// [`Error::is_retryable`] is `false` and it is up to the caller to decide.
    /// WhatsApp has no idempotency key to fall back on.
    ///
    /// ```rust
    /// use meta_whatsapp::{error::TransportPhase, Error};
    ///
    /// let cause = Error::Transport {
    ///     message: "timeout".into(),
    ///     phase: TransportPhase::AfterWrite,
    /// };
    /// let err = Error::ambiguous("POST https://graph.facebook.com/v21.0/1/messages", None, cause);
    /// assert!(!err.is_retryable());
    /// assert!(err.is_ambiguous());
    /// ```
    #[error("ambiguous delivery of {operation}: {cause}")]
    Ambiguous {
        /// Method and URL of the request that may have taken effect.
        operation: String,
        /// HTTP status when the server did answer (`5xx`), `None` when the
        /// connection died after the request was written.
        status: Option<u16>,
        /// The failure that left the outcome unknown.
        #[source]
        cause: Box<Error>,
    },
}

impl Error {
    /// Builds a validation error. `field` is the dotted path, `message` the
    /// human readable detail.
    ///
    /// ```rust
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

    /// Builds a transport error that happened *before* anything was written
    /// (DNS, connect, TLS handshake) — safe to retry.
    ///
    /// ```rust
    /// use meta_whatsapp::{error::TransportPhase, Error};
    /// let err = Error::transport("connection reset");
    /// assert!(err.is_retryable());
    /// assert_eq!(err.transport_phase(), Some(TransportPhase::BeforeWrite));
    /// ```
    #[must_use]
    pub fn transport(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
            phase: TransportPhase::BeforeWrite,
        }
    }

    /// Builds a transport error that happened *after* the request was written
    /// (timeout, reset, truncated body). Ambiguous for a non-idempotent
    /// request.
    ///
    /// ```rust
    /// use meta_whatsapp::Error;
    ///
    /// let err = Error::transport_after_write("timed out reading response");
    /// assert!(err.is_ambiguous());
    /// assert!(!err.is_retryable(), "a retry could duplicate the message");
    /// ```
    #[must_use]
    pub fn transport_after_write(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
            phase: TransportPhase::AfterWrite,
        }
    }

    /// Builds a terminal transport error (not retryable).
    ///
    /// ```rust
    /// use meta_whatsapp::Error;
    /// let err = Error::transport_terminal("unsupported scheme");
    /// assert!(!err.is_retryable());
    /// ```
    #[must_use]
    pub fn transport_terminal(message: impl Into<String>) -> Self {
        Self::Transport {
            message: message.into(),
            phase: TransportPhase::Terminal,
        }
    }

    /// Builds an [`Error::Ambiguous`] around the failure that left the outcome
    /// unknown.
    ///
    /// ```rust
    /// use meta_whatsapp::Error;
    ///
    /// let err = Error::ambiguous(
    ///     "POST /messages",
    ///     Some(503),
    ///     Error::Api {
    ///         code: 2,
    ///         status: 503,
    ///         message: "down".into(),
    ///         retry_after: None,
    ///         retryable: false,
    ///     },
    /// );
    /// assert_eq!(err.status(), Some(503));
    /// assert!(err.is_ambiguous());
    /// ```
    #[must_use]
    pub fn ambiguous(operation: impl Into<String>, status: Option<u16>, cause: Error) -> Self {
        Self::Ambiguous {
            operation: operation.into(),
            status,
            cause: Box::new(cause),
        }
    }

    /// Whether another attempt could succeed *and* is safe. Signature, payload
    /// and validation errors are terminal by construction;
    /// [`Error::Ambiguous`] and post-write transport failures are `false`
    /// because a retry could duplicate a message.
    ///
    /// ```rust
    /// use meta_whatsapp::{Error, RetryPolicy};
    ///
    /// let policy = RetryPolicy::default();
    /// assert!(policy.delay(1, None).is_some());
    /// assert!(Error::transport("dns").is_retryable());
    /// assert!(!Error::transport_after_write("timeout").is_retryable());
    /// ```
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Transport { phase, .. } => phase.is_before_write(),
            Self::Api { retryable, .. } => *retryable,
            _ => false,
        }
    }

    /// Whether the request may have taken effect even though it failed.
    ///
    /// `true` for [`Error::Ambiguous`] and for transport failures *after* the
    /// request was written. A caller that retries such a request risks a
    /// duplicate delivery; a caller that does not risks a lost one.
    ///
    /// ```rust
    /// use meta_whatsapp::{error::TransportPhase, Error};
    ///
    /// let lost = Error::Transport { message: "reset".into(), phase: TransportPhase::AfterWrite };
    /// assert!(lost.is_ambiguous());
    /// assert!(!Error::transport("dns").is_ambiguous());
    /// ```
    #[must_use]
    pub fn is_ambiguous(&self) -> bool {
        match self {
            Self::Ambiguous { .. } => true,
            Self::Transport { phase, .. } => phase.is_after_write(),
            _ => false,
        }
    }

    /// The transport phase, when this is a transport failure.
    ///
    /// ```rust
    /// use meta_whatsapp::{error::TransportPhase, Error};
    ///
    /// let err = Error::transport_terminal("bad scheme");
    /// assert_eq!(err.transport_phase(), Some(TransportPhase::Terminal));
    /// assert_eq!(Error::transport("dns").transport_phase(), Some(TransportPhase::BeforeWrite));
    /// ```
    #[must_use]
    pub fn transport_phase(&self) -> Option<TransportPhase> {
        match self {
            Self::Transport { phase, .. } => Some(*phase),
            _ => None,
        }
    }

    /// The HTTP status of the failed response, when one arrived.
    ///
    /// ```rust
    /// use meta_whatsapp::Error;
    ///
    /// assert_eq!(Error::transport("dns").status(), None);
    /// let api = Error::Api {
    ///     code: 4,
    ///     status: 429,
    ///     message: "slow down".into(),
    ///     retry_after: None,
    ///     retryable: true,
    /// };
    /// assert_eq!(api.status(), Some(429));
    /// ```
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            Self::Ambiguous { status, .. } => *status,
            _ => None,
        }
    }

    /// The server requested wait, when it sent a `Retry-After` header.
    ///
    /// This is the parsed header value unchanged. With `use_retry_after`
    /// enabled (the default), a value above [`crate::RetryPolicy::max_delay`]
    /// causes the client to surface the error instead of retrying early, so the
    /// caller still sees the full hint.
    ///
    /// ```rust
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
            Self::Ambiguous { cause, .. } => cause.retry_after(),
            _ => None,
        }
    }
}

/// Convenience alias used across the crate.
///
/// ```rust
/// fn always_ok() -> meta_whatsapp::Result<u8> {
///     Ok(1)
/// }
/// assert_eq!(always_ok().unwrap(), 1);
/// ```
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ambiguous_errors_say_so() {
        assert!(Error::transport_after_write("x").is_ambiguous());
        assert!(!Error::transport("x").is_ambiguous());
        assert!(!Error::transport_terminal("x").is_ambiguous());
        assert!(!Error::validation("a", "b").is_ambiguous());
    }

    #[test]
    fn a_wrapped_cause_keeps_its_retry_after() {
        let cause = Error::Api {
            code: 4,
            status: 429,
            message: "slow".into(),
            retry_after: Some(Duration::from_secs(3)),
            retryable: true,
        };
        let wrapped = Error::ambiguous("POST /messages", Some(429), cause);
        assert_eq!(wrapped.retry_after(), Some(Duration::from_secs(3)));
        assert_eq!(wrapped.status(), Some(429));
    }

    #[test]
    fn display_never_panics_for_any_variant() {
        let errors = [
            Error::EmptySecret,
            Error::MalformedSignature(String::new()),
            Error::SignatureMismatch,
            Error::Replay {
                timestamp: 0,
                skew_secs: 1,
                tolerance_secs: 2,
            },
            Error::Payload(String::new()),
            Error::Flow(String::new()),
            Error::transport("x"),
            Error::transport_after_write("x"),
            Error::transport_terminal("x"),
            Error::ambiguous("op", None, Error::transport("x")),
        ];
        for error in errors {
            assert_ne!(error.to_string(), "");
        }
    }
}
