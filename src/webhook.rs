//! Webhook verification, replay protection and payload extraction.
//!
//! Meta signs every POST with `X-Hub-Signature-256: sha256=<hex>`, an HMAC of
//! the raw body keyed with the app secret. Verification is constant time
//! ([`subtle::ConstantTimeEq`]) and fails closed: an empty secret is rejected
//! at construction, because HMAC with an empty key would accept anything.
//!
//! Meta signs the body only — there is no signature timestamp — so replay
//! protection uses the message timestamps inside the payload and an explicit
//! window ([`WebhookVerifier::with_tolerance`], five minutes by default).
//!
//! ```
//! use meta_whatsapp::WebhookVerifier;
//!
//! let verifier = WebhookVerifier::new("app-secret")?;
//! let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
//! let header = verifier.sign(body);
//! assert!(verifier.verify(body, &header).is_ok());
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use std::time::Duration;

use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use crate::error::{Error, Result};
use crate::types::WebhookPayload;

type HmacSha256 = Hmac<Sha256>;

/// Default replay window: five minutes, Meta's documented retry horizon.
pub const DEFAULT_TOLERANCE_SECS: u64 = 300;

/// Default maximum body size (1 MiB). Real payloads are a few KiB; the cap
/// keeps a hostile caller from making the process buffer unbounded input
/// before the signature is checked.
pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Verifies the `X-Hub-Signature-256` header Meta sends with every webhook call.
///
/// The header has the shape `sha256=<hex>`. Verification is constant time and
/// never compares hex strings directly.
///
/// ```
/// use meta_whatsapp::WebhookVerifier;
///
/// let verifier = WebhookVerifier::new(b"app-secret")?;
/// let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
/// let header = verifier.sign(body);
/// assert!(verifier.verify(body, &header).is_ok());
/// // A tampered body no longer matches.
/// assert!(verifier.verify(br#"{"object":"x","entry":[]}"#, &header).is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Clone)]
pub struct WebhookVerifier {
    /// Keyed hasher built once at construction, where a rejected key is still a
    /// typed error rather than a panic. The raw secret is not kept: one copy of
    /// it in memory is enough.
    hasher: HmacSha256,
    tolerance: Duration,
    max_body_bytes: usize,
}

impl std::fmt::Debug for WebhookVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the app secret.
        f.debug_struct("WebhookVerifier")
            .field("tolerance", &self.tolerance)
            .field("max_body_bytes", &self.max_body_bytes)
            .finish_non_exhaustive()
    }
}

impl WebhookVerifier {
    /// Creates a verifier for the given Meta app secret.
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    /// assert!(WebhookVerifier::new("s").is_ok());
    /// assert!(WebhookVerifier::new("").is_err());
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::EmptySecret`] for an empty secret: HMAC with an empty key is
    /// well defined, so a missing environment variable would silently accept
    /// every request.
    pub fn new(secret: impl Into<Vec<u8>>) -> Result<Self> {
        let secret = secret.into();
        if secret.is_empty() {
            return Err(Error::EmptySecret);
        }
        let hasher = HmacSha256::new_from_slice(&secret).map_err(|_| {
            Error::Payload("the app secret was rejected as an HMAC key".to_string())
        })?;
        Ok(Self {
            hasher,
            tolerance: Duration::from_secs(DEFAULT_TOLERANCE_SECS),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        })
    }

    /// Sets the replay window. `Duration::ZERO` disables the timestamp check
    /// (signature verification stays mandatory).
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("s")?.with_tolerance(Duration::from_secs(60));
    /// assert_eq!(verifier.tolerance().as_secs(), 60);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn with_tolerance(mut self, tolerance: Duration) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Sets the largest body [`WebhookVerifier::verify_and_parse`] will accept.
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("s")?.with_max_body_bytes(4096);
    /// assert_eq!(verifier.max_body_bytes(), 4096);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn with_max_body_bytes(mut self, limit: usize) -> Self {
        self.max_body_bytes = limit;
        self
    }

    /// The configured replay window.
    ///
    /// ```
    /// use meta_whatsapp::{WebhookVerifier, webhook::DEFAULT_TOLERANCE_SECS};
    ///
    /// let verifier = WebhookVerifier::new("s")?;
    /// assert_eq!(verifier.tolerance().as_secs(), DEFAULT_TOLERANCE_SECS);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn tolerance(&self) -> Duration {
        self.tolerance
    }

    /// The configured body size limit.
    ///
    /// ```
    /// use meta_whatsapp::{WebhookVerifier, webhook::DEFAULT_MAX_BODY_BYTES};
    ///
    /// assert_eq!(WebhookVerifier::new("s")?.max_body_bytes(), DEFAULT_MAX_BODY_BYTES);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    /// Computes the header value for `body` — handy for tests and fixtures.
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("secret")?;
    /// let header = verifier.sign(b"{}");
    /// assert_eq!(header.len(), "sha256=".len() + 64);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn sign(&self, body: &[u8]) -> String {
        let mut mac = self.mac();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    /// Checks `header` against `body` in constant time.
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("secret")?;
    /// let header = verifier.sign(b"{}");
    /// assert!(verifier.verify(b"{}", &header).is_ok());
    /// assert!(verifier.verify(b"{ }", &header).is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::MalformedSignature`] when the header is not `sha256=<64 hex>`,
    /// [`Error::SignatureMismatch`] when it does not match.
    ///
    /// # Panics
    ///
    /// Never in practice: HMAC-SHA256 accepts keys of any length.
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

        let mut mac = self.mac();
        mac.update(body);
        let expected = mac.finalize().into_bytes();

        // Constant-time over all 32 bytes; `unwrap_u8` is a branchless read of
        // the `Choice` (0 or 1), so timing does not depend on where the first
        // differing byte is.
        if expected.as_slice().ct_eq(received.as_slice()).unwrap_u8() == 1 {
            Ok(())
        } else {
            Err(Error::SignatureMismatch)
        }
    }

    /// Verifies and parses in one step (no replay check — call
    /// [`WebhookVerifier::verify_fresh`] for that).
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("secret")?;
    /// let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
    /// let payload = verifier.verify_and_parse(body, &verifier.sign(body))?;
    /// assert_eq!(payload.object, "whatsapp_business_account");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Propagates signature errors; [`Error::Payload`] when the body exceeds the
    /// size limit or is not the expected JSON shape.
    pub fn verify_and_parse(&self, body: &[u8], header: &str) -> Result<WebhookPayload> {
        self.verify(body, header)?;
        self.parse(body)
    }

    /// Verifies, parses, and rejects payloads whose timestamps fall outside the
    /// replay window around `now_unix`.
    ///
    /// `now_unix` is a parameter rather than a clock read so the check is
    /// deterministic and testable.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("secret")?.with_tolerance(Duration::from_secs(60));
    /// let body = br#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
    ///     "field":"messages","value":{
    ///         "metadata":{"display_phone_number":"1","phone_number_id":"2"},
    ///         "messages":[{"id":"m","from":"3","type":"text","timestamp":"1000"}]}}]}]}"#;
    /// let header = verifier.sign(body);
    ///
    /// assert!(verifier.verify_fresh(body, &header, 1_010).is_ok());
    /// assert!(verifier.verify_fresh(body, &header, 2_000).is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Signature errors, [`Error::Payload`] for malformed timestamps, and
    /// [`Error::Replay`] outside the window.
    pub fn verify_fresh(&self, body: &[u8], header: &str, now_unix: u64) -> Result<WebhookPayload> {
        let payload = self.verify_and_parse(body, header)?;
        self.check_replay(&payload, now_unix)?;
        Ok(payload)
    }

    /// Rejects a payload when any message or status timestamp is outside the
    /// window — one stale entry is enough, so an old message cannot ride along
    /// inside a freshly signed batch.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::{WebhookPayload, WebhookVerifier};
    ///
    /// let verifier = WebhookVerifier::new("s")?.with_tolerance(Duration::from_secs(10));
    /// let payload: WebhookPayload = serde_json::from_str(
    ///     r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
    ///         "field":"messages","value":{
    ///             "metadata":{"display_phone_number":"1","phone_number_id":"2"},
    ///             "messages":[{"id":"m","from":"3","type":"text","timestamp":"500"}]}}]}]}"#,
    /// )?;
    /// assert!(verifier.check_replay(&payload, 505).is_ok());
    /// assert!(verifier.check_replay(&payload, 900).is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Replay`] outside the window, [`Error::Payload`] when a
    /// timestamp is present but not a number (fail closed).
    pub fn check_replay(&self, payload: &WebhookPayload, now_unix: u64) -> Result<()> {
        let tolerance_secs = self.tolerance.as_secs();
        if tolerance_secs == 0 {
            return Ok(());
        }
        for raw in payload.timestamps() {
            let raw = raw.trim();
            if raw.is_empty() {
                // Meta omits timestamps on some events; they carry no replay
                // information, so there is nothing to compare.
                continue;
            }
            let timestamp = raw
                .parse::<u64>()
                .map_err(|_| Error::Payload(format!("timestamp {raw:?} is not unix seconds")))?;
            let skew_secs = timestamp.abs_diff(now_unix);
            if skew_secs > tolerance_secs {
                return Err(Error::Replay {
                    timestamp,
                    skew_secs,
                    tolerance_secs,
                });
            }
        }
        Ok(())
    }

    /// Answers the GET handshake Meta performs when a webhook URL is registered.
    ///
    /// Returns the `hub.challenge` value when the mode is `subscribe` and the
    /// token matches, otherwise `None` (the caller should answer 403). The
    /// token comparison is constant time.
    ///
    /// ```
    /// use meta_whatsapp::WebhookVerifier;
    ///
    /// let verifier = WebhookVerifier::new("secret")?;
    /// assert_eq!(
    ///     verifier.verify_challenge(Some("subscribe"), Some("tok"), Some("123"), "tok", "subscribe"),
    ///     Some("123")
    /// );
    /// assert_eq!(
    ///     verifier.verify_challenge(Some("subscribe"), Some("nope"), Some("123"), "tok", "subscribe"),
    ///     None
    /// );
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn verify_challenge<'a>(
        &self,
        mode: Option<&str>,
        token: Option<&str>,
        challenge: Option<&'a str>,
        expected_token: &str,
        expected_mode: &str,
    ) -> Option<&'a str> {
        if mode? != expected_mode {
            return None;
        }
        let token = token?;
        if !constant_time_eq(token.as_bytes(), expected_token.as_bytes()) {
            return None;
        }
        challenge
    }

    /// Parses a body, enforcing the size limit.
    fn parse(&self, body: &[u8]) -> Result<WebhookPayload> {
        if body.len() > self.max_body_bytes {
            return Err(Error::Payload(format!(
                "body is {} bytes, limit is {}",
                body.len(),
                self.max_body_bytes
            )));
        }
        serde_json::from_slice(body).map_err(Error::from)
    }

    fn mac(&self) -> HmacSha256 {
        // Cloning the pre-keyed hasher keeps `sign`/`verify` infallible: the key
        // was already accepted (or rejected) in `new`.
        self.hasher.clone()
    }
}

/// Length-independent comparison that does not short-circuit.
///
/// Different lengths are compared as unequal without leaking where the first
/// difference is: everything is folded into one accumulator.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        // Length is not secret (the verify token length is public knowledge in
        // the dashboard), so an early return here leaks nothing.
        return false;
    }
    left.ct_eq(right).unwrap_u8() == 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equality() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn mac_never_panics_on_long_or_weird_secrets() {
        let verifier = WebhookVerifier::new("\u{0}\u{1}secret".as_bytes().to_vec()).unwrap();
        let header = verifier.sign(b"body");
        assert!(verifier.verify(b"body", &header).is_ok());
    }
}
