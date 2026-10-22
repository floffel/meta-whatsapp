//! Webhook verification and payload extraction.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::{Error, Result};
use crate::types::WebhookPayload;

type HmacSha256 = Hmac<Sha256>;

/// Verifies the `X-Hub-Signature-256` header Meta sends with every webhook call.
///
/// The header has the shape `sha256=<hex>`. Verification is constant time.
///
/// ```
/// use meta_whatsapp::WebhookVerifier;
///
/// let verifier = WebhookVerifier::new(b"app-secret")?;
/// let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
/// // In production this comes from the request header.
/// let header = verifier.sign(body);
/// assert!(verifier.verify(body, &header).is_ok());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Clone)]
pub struct WebhookVerifier {
    secret: Vec<u8>,
}

impl std::fmt::Debug for WebhookVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the app secret.
        f.debug_struct("WebhookVerifier").finish_non_exhaustive()
    }
}

impl WebhookVerifier {
    /// Creates a verifier for the given Meta app secret.
    ///
    /// # Errors
    ///
    /// Rejects an empty secret: HMAC with an empty key is well defined, so a
    /// missing environment variable would silently accept every request.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(Error::EmptySecret);
        }
        Ok(Self { secret })
    }

    /// Computes the header value for `body` — handy for tests and fixtures.
    ///
    /// # Panics
    ///
    /// Never in practice: HMAC accepts keys of any length. The internal
    /// `expect` only guards a contract that cannot fail.
    #[must_use]
    pub fn sign(&self, body: &[u8]) -> String {
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// Checks `header` against `body` in constant time.
    ///
    /// # Errors
    ///
    /// Returns [`Error::MalformedSignature`] when the header is not
    /// `sha256=<hex>` and [`Error::SignatureMismatch`] when it does not match.
    ///
    /// # Panics
    ///
    /// Never in practice: HMAC accepts keys of any length.
    pub fn verify(&self, body: &[u8], header: &str) -> Result<()> {
        let hex_part = header
            .strip_prefix("sha256=")
            .ok_or_else(|| Error::MalformedSignature(header.to_string()))?;
        let received =
            hex::decode(hex_part).map_err(|_| Error::MalformedSignature(header.to_string()))?;
        // A SHA-256 MAC is always 32 bytes. Anything else cannot be a valid
        // signature and is a malformed header, not a mismatch.
        if received.len() != 32 {
            return Err(Error::MalformedSignature(header.to_string()));
        }

        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC accepts any key length");
        mac.update(body);
        mac.verify_slice(&received)
            .map_err(|_| Error::SignatureMismatch)
    }

    /// Verifies and parses in one step.
    ///
    /// # Errors
    ///
    /// Propagates signature errors and maps JSON failures to [`Error::Payload`].
    pub fn verify_and_parse(&self, body: &[u8], header: &str) -> Result<WebhookPayload> {
        self.verify(body, header)?;
        serde_json::from_slice(body).map_err(|e| Error::Payload(e.to_string()))
    }

    /// Answers the GET handshake Meta performs when a webhook URL is registered.
    ///
    /// Returns the `hub.challenge` value when the mode is `subscribe` and the
    /// token matches, otherwise `None` (the caller should answer 403).
    #[must_use]
    pub fn verify_challenge<'a>(
        &self,
        mode: Option<&str>,
        token: Option<&'a str>,
        challenge: Option<&'a str>,
        expected_token: &str,
        expected_mode: &str,
    ) -> Option<&'a str> {
        if mode? != expected_mode || token? != expected_token {
            return None;
        }
        challenge
    }
}
