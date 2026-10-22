//! Error type shared by the webhook and client sides.

use thiserror::Error;

/// Everything that can go wrong while verifying a webhook or calling the API.
#[derive(Debug, Error)]
pub enum Error {
    /// The signature header was missing or not parseable as `sha256=<hex>`.
    #[error("malformed signature header: {0}")]
    MalformedSignature(String),

    /// The computed HMAC did not match the header.
    #[error("signature mismatch")]
    SignatureMismatch,

    /// The request body was not valid UTF-8/JSON, or did not match the schema.
    #[error("invalid payload: {0}")]
    Payload(String),

    /// The transport reported a failure (network, timeout, non-2xx).
    #[error("transport error: {0}")]
    Transport(String),

    /// The Graph API answered with an error envelope.
    #[error("graph api error {code}: {message}")]
    Api {
        /// Numeric Graph API error code.
        code: i64,
        /// Human readable message from Meta.
        message: String,
    },
}

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, Error>;
