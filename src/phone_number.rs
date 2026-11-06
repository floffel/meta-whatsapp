//! WABA-level phone-number listing and onboarding.
//!
//! A Cloud API number is created under a WhatsApp Business Account, verified by
//! a code sent to the phone, and finally registered with the six-digit
//! two-step PIN. This module covers the WABA-scoped list/create and the code
//! request/verify edges; the final registration call is
//! [`crate::WhatsAppClient::register`] on a client bound to the new number id.
//!
//! Contract sources are the published v23 OpenAPI description and Meta's phone
//! number onboarding reference, recorded in `docs/contracts.md`.
//!
//! ```
//! use meta_whatsapp::phone_number::{PhoneNumberCreateRequest, PhoneNumberSort};
//!
//! let create = PhoneNumberCreateRequest::new("16315551000", "Jasper's Market")?
//!     .country_dial_code("1");
//! assert_eq!(create.to_json()?["phone_number"], "16315551000");
//!
//! let query = meta_whatsapp::phone_number::PhoneNumberQuery::new()
//!     .limit(100)
//!     .sort(PhoneNumberSort::LastOnboardedDesc);
//! query.validate()?;
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use subtle::ConstantTimeEq;

use crate::business::{MessagingLimitTier, QualityRating};
use crate::error::{Error, Result};
use crate::template::{encode_query_value, Cursors, Paging};

/// Shortest phone number accepted by the create endpoint, in digits.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::PHONE_NUMBER_MIN_DIGITS, 7);
/// ```
pub const PHONE_NUMBER_MIN_DIGITS: usize = 7;

/// Longest phone number accepted by the create endpoint, in digits.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::PHONE_NUMBER_MAX_DIGITS, 15);
/// ```
pub const PHONE_NUMBER_MAX_DIGITS: usize = 15;

/// Shortest verified business name accepted by the create endpoint, in chars.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::VERIFIED_NAME_MIN_CHARS, 2);
/// ```
pub const VERIFIED_NAME_MIN_CHARS: usize = 2;

/// Longest verified business name accepted by the create endpoint, in chars.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::VERIFIED_NAME_MAX_CHARS, 75);
/// ```
pub const VERIFIED_NAME_MAX_CHARS: usize = 75;

/// Minimum page size documented for the WABA phone-number list.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::PHONE_NUMBER_PAGE_MIN, 1);
/// ```
pub const PHONE_NUMBER_PAGE_MIN: u32 = 1;

/// Maximum page size documented for the WABA phone-number list.
///
/// ```
/// assert_eq!(meta_whatsapp::phone_number::PHONE_NUMBER_PAGE_MAX, 100);
/// ```
pub const PHONE_NUMBER_PAGE_MAX: u32 = 100;

/// Validates a phone-number id before it becomes a URL path segment.
///
/// ```
/// use meta_whatsapp::phone_number::validate_phone_number_id;
///
/// assert!(validate_phone_number_id("1906385232743451").is_ok());
/// assert!(validate_phone_number_id("../me").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `phone_number_id` when it is empty, longer
/// than 32 characters, or not digits only.
pub fn validate_phone_number_id(phone_number_id: &str) -> Result<()> {
    if phone_number_id.is_empty() {
        return Err(Error::validation("phone_number_id", "must not be empty"));
    }
    if phone_number_id.len() > 32 || !phone_number_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::validation(
            "phone_number_id",
            "must be digits only (at most 32)",
        ));
    }
    Ok(())
}

/// How Meta delivers a phone-number verification code.
///
/// ```
/// use meta_whatsapp::phone_number::CodeMethod;
/// assert_eq!(CodeMethod::Sms.as_str(), "SMS");
/// assert_eq!(CodeMethod::Voice.as_str(), "VOICE");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CodeMethod {
    /// Send the code by SMS.
    #[default]
    Sms,
    /// Read the code out over a voice call.
    Voice,
}

impl CodeMethod {
    /// The uppercase spelling Meta expects.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::CodeMethod;
    /// assert_eq!(CodeMethod::Voice.as_str(), "VOICE");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sms => "SMS",
            Self::Voice => "VOICE",
        }
    }
}

impl fmt::Display for CodeMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Locale for the verification message, in Meta's `ll_CC` form.
///
/// ```
/// use meta_whatsapp::phone_number::CodeLanguage;
///
/// assert_eq!(CodeLanguage::new("en_US")?.as_str(), "en_US");
/// assert!(CodeLanguage::new("en-US").is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodeLanguage(String);

impl CodeLanguage {
    /// Validates and stores a locale.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `language` unless the input is exactly two
    /// lowercase ASCII letters, `_`, then two uppercase ASCII letters.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::CodeLanguage;
    ///
    /// assert!(CodeLanguage::new("de_DE").is_ok());
    /// assert!(CodeLanguage::new("de").is_err());
    /// assert!(CodeLanguage::new("DE_de").is_err());
    /// ```
    pub fn new(raw: impl Into<String>) -> Result<Self> {
        let raw = raw.into();
        let bytes = raw.as_bytes();
        let valid = bytes.len() == 5
            && bytes[0].is_ascii_lowercase()
            && bytes[1].is_ascii_lowercase()
            && bytes[2] == b'_'
            && bytes[3].is_ascii_uppercase()
            && bytes[4].is_ascii_uppercase();
        if valid {
            Ok(Self(raw))
        } else {
            Err(Error::validation(
                "language",
                "must be exactly two lowercase letters, '_', and two uppercase letters",
            ))
        }
    }

    /// The validated locale.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::CodeLanguage;
    /// assert_eq!(CodeLanguage::new("fr_FR")?.as_str(), "fr_FR");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The six-digit code Meta sent to the phone number.
///
/// The code is a short-lived secret. It has no getter, no `Default`, redacts
/// itself in `Debug` and `Display`, and never appears in a validation error.
///
/// ```
/// use meta_whatsapp::phone_number::VerificationCode;
///
/// let code = VerificationCode::new("123456")?;
/// assert_eq!(format!("{code:?}"), "VerificationCode([redacted])");
/// assert!(VerificationCode::new("12345").is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Clone)]
pub struct VerificationCode(String);

impl VerificationCode {
    /// Validates and stores exactly six ASCII digits.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `code` when the value is not exactly six
    /// ASCII digits. The value is not echoed.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::VerificationCode;
    /// assert!(VerificationCode::new("000000").is_ok());
    /// assert!(VerificationCode::new("１２３４５６").is_err());
    /// ```
    pub fn new(raw: impl AsRef<str>) -> Result<Self> {
        let raw = raw.as_ref();
        let valid = raw.len() == 6 && raw.bytes().all(|byte| byte.is_ascii_digit());
        if valid {
            Ok(Self(raw.to_string()))
        } else {
            Err(Error::validation("code", "must be exactly 6 ASCII digits"))
        }
    }
}

impl fmt::Debug for VerificationCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VerificationCode([redacted])")
    }
}

impl fmt::Display for VerificationCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("6-digit verification code (redacted)")
    }
}

impl PartialEq for VerificationCode {
    fn eq(&self, other: &Self) -> bool {
        bool::from(self.0.as_bytes().ct_eq(other.0.as_bytes()))
    }
}

impl Eq for VerificationCode {}

/// Body of `POST /{phone-number-id}/request_code`.
///
/// ```
/// use meta_whatsapp::phone_number::{CodeLanguage, CodeMethod, RequestCodeRequest};
///
/// let request = RequestCodeRequest::new(CodeMethod::Sms, CodeLanguage::new("en_US")?);
/// assert_eq!(request.to_json(), serde_json::json!({
///     "code_method": "SMS",
///     "language": "en_US"
/// }));
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestCodeRequest {
    method: CodeMethod,
    language: CodeLanguage,
}

impl RequestCodeRequest {
    /// Builds a request for a validated delivery method and language.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{CodeLanguage, CodeMethod, RequestCodeRequest};
    ///
    /// let body = RequestCodeRequest::new(CodeMethod::Voice, CodeLanguage::new("de_DE")?).to_json();
    /// assert_eq!(body["code_method"], "VOICE");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn new(method: CodeMethod, language: CodeLanguage) -> Self {
        Self { method, language }
    }

    /// Renders exactly the two documented keys.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{CodeLanguage, CodeMethod, RequestCodeRequest};
    ///
    /// let body = RequestCodeRequest::new(CodeMethod::Sms, CodeLanguage::new("en_GB")?).to_json();
    /// assert_eq!(body.as_object().unwrap().len(), 2);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "code_method": self.method.as_str(),
            "language": self.language.as_str(),
        })
    }
}

/// Response to a successful verification-code request.
///
/// ```
/// use meta_whatsapp::phone_number::RequestCodeOutcome;
///
/// let outcome = RequestCodeOutcome::from_value(&serde_json::json!({"success": true}))?;
/// assert!(outcome.success);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RequestCodeOutcome {
    /// Whether Meta accepted the request.
    #[serde(default)]
    pub success: bool,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl RequestCodeOutcome {
    /// Reads the response body, requiring an object.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the body is not an object or `success` is not a
    /// boolean.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::RequestCodeOutcome;
    ///
    /// assert!(RequestCodeOutcome::from_value(&serde_json::json!("x")).is_err());
    /// assert!(RequestCodeOutcome::from_value(&serde_json::json!({"success": true}))?.success);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Payload(
                "expected a JSON object for the code-request response".to_string(),
            ));
        }
        serde_json::from_value(value.clone()).map_err(|error| {
            Error::Payload(format!("unexpected code-request response shape: {error}"))
        })
    }
}

/// Body of `POST /{phone-number-id}/verify_code`.
///
/// The debug representation redacts the code.
///
/// ```
/// use meta_whatsapp::phone_number::{VerificationCode, VerifyCodeRequest};
///
/// let request = VerifyCodeRequest::new(&VerificationCode::new("123456")?);
/// assert_eq!(request.to_json(), serde_json::json!({"code": "123456"}));
/// assert!(!format!("{request:?}").contains("123456"));
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Clone)]
pub struct VerifyCodeRequest {
    code: VerificationCode,
}

impl VerifyCodeRequest {
    /// Builds a request from an already validated code.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{VerificationCode, VerifyCodeRequest};
    ///
    /// let code = VerificationCode::new("654321")?;
    /// assert_eq!(VerifyCodeRequest::new(&code).to_json()["code"], "654321");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn new(code: &VerificationCode) -> Self {
        Self { code: code.clone() }
    }

    /// Renders the documented `{"code": "…"}` body.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{VerificationCode, VerifyCodeRequest};
    ///
    /// let body = VerifyCodeRequest::new(&VerificationCode::new("000001")?).to_json();
    /// assert_eq!(body.as_object().unwrap().len(), 1);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "code": self.code.0 })
    }
}

impl fmt::Debug for VerifyCodeRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerifyCodeRequest")
            .field("code", &self.code)
            .finish()
    }
}

/// Response from verifying a phone-number code.
///
/// ```
/// use meta_whatsapp::phone_number::VerifyCodeOutcome;
///
/// let outcome = VerifyCodeOutcome::from_value(&serde_json::json!({
///     "success": true,
///     "id": "1234567890123456"
/// }))?;
/// assert!(outcome.success);
/// assert_eq!(outcome.id.as_deref(), Some("1234567890123456"));
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VerifyCodeOutcome {
    /// Whether Meta accepted the code.
    #[serde(default)]
    pub success: bool,
    /// The verified phone-number id, when Meta returned it.
    #[serde(default)]
    pub id: Option<String>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl VerifyCodeOutcome {
    /// Reads the response body, requiring an object.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the body is not an object or its known fields
    /// have unexpected JSON types.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::VerifyCodeOutcome;
    ///
    /// assert!(VerifyCodeOutcome::from_value(&serde_json::json!([])).is_err());
    /// assert!(VerifyCodeOutcome::from_value(&serde_json::json!({"success": true}))?.success);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Payload(
                "expected a JSON object for the code-verification response".to_string(),
            ));
        }
        serde_json::from_value(value.clone()).map_err(|error| {
            Error::Payload(format!(
                "unexpected code-verification response shape: {error}"
            ))
        })
    }
}

/// Body of `POST /{waba-id}/phone_numbers`.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
///
/// let request = PhoneNumberCreateRequest::new("16315551000", "Jasper's Market")?
///     .country_dial_code("1");
/// assert_eq!(request.to_json()?["phone_number"], "16315551000");
/// assert_eq!(request.to_json()?["verified_name"], "Jasper's Market");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneNumberCreateRequest {
    phone_number: String,
    verified_name: String,
    country_dial_code: Option<String>,
    migrate: bool,
    preverified_id: Option<String>,
}

impl PhoneNumberCreateRequest {
    /// Creates a request, validating the number and business name locally.
    ///
    /// `phone_number` is E.164 digits without `+`.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `phone_number` unless it is 7–15 digits
    /// starting with 1–9, or `verified_name` unless it is 2–75 characters.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// assert!(PhoneNumberCreateRequest::new("16315551000", "My Business").is_ok());
    /// assert!(PhoneNumberCreateRequest::new("+16315551000", "My Business").is_err());
    /// assert!(PhoneNumberCreateRequest::new("16315551000", "A").is_err());
    /// ```
    pub fn new(phone_number: impl Into<String>, verified_name: impl Into<String>) -> Result<Self> {
        let request = Self {
            phone_number: phone_number.into(),
            verified_name: verified_name.into(),
            country_dial_code: None,
            migrate: false,
            preverified_id: None,
        };
        request.validate()?;
        Ok(request)
    }

    /// Sets the optional 1–3 digit country dialing code.
    ///
    /// The value is checked by [`PhoneNumberCreateRequest::validate`] and
    /// [`PhoneNumberCreateRequest::to_json`].
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// let request = PhoneNumberCreateRequest::new("16315551000", "My Business")?
    ///     .country_dial_code("1");
    /// assert_eq!(request.to_json()?["cc"], "1");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn country_dial_code(mut self, code: impl Into<String>) -> Self {
        self.country_dial_code = Some(code.into());
        self
    }

    /// Marks this as a migration from the on-premises API.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// let request = PhoneNumberCreateRequest::new("16315551000", "My Business")?.migrate();
    /// assert_eq!(request.to_json()?["migrate_phone_number"], true);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn migrate(mut self) -> Self {
        self.migrate = true;
        self
    }

    /// Sets a pre-verified number id for BSP onboarding.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// let request = PhoneNumberCreateRequest::new("16315551000", "My Business")?
    ///     .preverified_id("preverified_12345");
    /// assert_eq!(request.to_json()?["preverified_id"], "preverified_12345");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn preverified_id(mut self, id: impl Into<String>) -> Self {
        self.preverified_id = Some(id.into());
        self
    }

    /// Re-checks every field before rendering.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the first invalid field.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// assert!(PhoneNumberCreateRequest::new("16315551000", "My Business")?
    ///     .country_dial_code("1234")
    ///     .validate()
    ///     .is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn validate(&self) -> Result<()> {
        let bytes = self.phone_number.as_bytes();
        let valid_number = bytes.len() >= PHONE_NUMBER_MIN_DIGITS
            && bytes.len() <= PHONE_NUMBER_MAX_DIGITS
            && bytes
                .first()
                .is_some_and(|byte| (b'1'..=b'9').contains(byte))
            && bytes.iter().all(u8::is_ascii_digit);
        if !valid_number {
            return Err(Error::validation(
                "phone_number",
                format!(
                    "must be {PHONE_NUMBER_MIN_DIGITS}..={PHONE_NUMBER_MAX_DIGITS} E.164 digits without '+' and start with 1-9"
                ),
            ));
        }
        let name_chars = self.verified_name.chars().count();
        if !(VERIFIED_NAME_MIN_CHARS..=VERIFIED_NAME_MAX_CHARS).contains(&name_chars) {
            return Err(Error::validation(
                "verified_name",
                format!("must be {VERIFIED_NAME_MIN_CHARS}..={VERIFIED_NAME_MAX_CHARS} characters"),
            ));
        }
        if self.verified_name.chars().any(char::is_control) {
            return Err(Error::validation(
                "verified_name",
                "must not contain control characters",
            ));
        }
        if let Some(code) = &self.country_dial_code {
            let valid = (1..=3).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit());
            if !valid {
                return Err(Error::validation("cc", "must be 1..=3 ASCII digits"));
            }
        }
        if let Some(id) = &self.preverified_id {
            if id.trim().is_empty() || id.chars().any(char::is_control) {
                return Err(Error::validation(
                    "preverified_id",
                    "must not be empty or contain control characters",
                ));
            }
        }
        Ok(())
    }

    /// Validates and renders the documented JSON body.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for any invalid field. No request is built.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateRequest;
    ///
    /// let body = PhoneNumberCreateRequest::new("16315551000", "My Business")?
    ///     .country_dial_code("1")
    ///     .to_json()?;
    /// assert_eq!(body.as_object().unwrap().len(), 3);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn to_json(&self) -> Result<Value> {
        self.validate()?;
        let mut body = Map::new();
        body.insert(
            "phone_number".to_string(),
            Value::String(self.phone_number.clone()),
        );
        body.insert(
            "verified_name".to_string(),
            Value::String(self.verified_name.clone()),
        );
        if let Some(code) = &self.country_dial_code {
            body.insert("cc".to_string(), Value::String(code.clone()));
        }
        if self.migrate {
            body.insert("migrate_phone_number".to_string(), Value::Bool(true));
        }
        if let Some(id) = &self.preverified_id {
            body.insert("preverified_id".to_string(), Value::String(id.clone()));
        }
        Ok(Value::Object(body))
    }
}

/// Result of creating a number under a WABA.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberCreateOutcome;
///
/// let outcome = PhoneNumberCreateOutcome::from_value(&serde_json::json!({
///     "id": "1906385232743451"
/// }))?;
/// assert_eq!(outcome.id, "1906385232743451");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PhoneNumberCreateOutcome {
    /// The new phone-number id used by later calls.
    pub id: String,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl PhoneNumberCreateOutcome {
    /// Reads and validates the create response.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the response is not an object, has no `id`, or
    /// the id is empty.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberCreateOutcome;
    ///
    /// assert!(PhoneNumberCreateOutcome::from_value(&serde_json::json!({})).is_err());
    /// assert!(PhoneNumberCreateOutcome::from_value(&serde_json::json!({"id": ""})).is_err());
    /// ```
    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Payload(
                "expected a JSON object for the phone-number create response".to_string(),
            ));
        }
        let outcome: Self = serde_json::from_value(value.clone()).map_err(|error| {
            Error::Payload(format!("unexpected phone-number create response: {error}"))
        })?;
        if outcome.id.is_empty() {
            return Err(Error::Payload(
                "phone-number create response carried an empty id".to_string(),
            ));
        }
        Ok(outcome)
    }
}

/// One phone number from the WABA list.
///
/// Unknown enum values are kept as strings, and unknown response fields stay
/// in [`WabaPhoneNumber::extra`].
///
/// ```
/// use meta_whatsapp::phone_number::WabaPhoneNumber;
///
/// let number: WabaPhoneNumber = serde_json::from_str(
///     r#"{"id":"1","display_phone_number":"+1 631-555-5555","status":"LINKED","quality_rating":"GREEN"}"#,
/// )?;
/// assert_eq!(number.status.as_deref(), Some("LINKED"));
/// assert_eq!(number.quality_rating, Some(meta_whatsapp::business::QualityRating::Green));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WabaPhoneNumber {
    /// Phone-number id used in subsequent Graph calls.
    #[serde(default)]
    pub id: Option<String>,
    /// International display form.
    #[serde(default)]
    pub display_phone_number: Option<String>,
    /// Verified business name.
    #[serde(default)]
    pub verified_name: Option<String>,
    /// Lifecycle status (`PENDING`, `LINKED`, …), kept as a string.
    #[serde(default)]
    pub status: Option<String>,
    /// Quality rating.
    #[serde(default)]
    pub quality_rating: Option<QualityRating>,
    /// ISO 3166-1 alpha-2 country code.
    #[serde(default)]
    pub country_code: Option<String>,
    /// Country dialing code without a leading plus.
    #[serde(default)]
    pub country_dial_code: Option<String>,
    /// `VERIFIED`, `NOT_VERIFIED`, or a value added later.
    #[serde(default)]
    pub code_verification_status: Option<String>,
    /// Unified certification status.
    #[serde(default)]
    pub unified_cert_status: Option<String>,
    /// `LIVE` or `SANDBOX`.
    #[serde(default)]
    pub account_mode: Option<String>,
    /// `CLOUD_API`, `ON_PREMISE`, or `NOT_APPLICABLE`.
    #[serde(default)]
    pub host_platform: Option<String>,
    /// Daily unique-recipient tier.
    #[serde(default)]
    pub messaging_limit_tier: Option<MessagingLimitTier>,
    /// Whether this is an official business account.
    #[serde(default)]
    pub is_official_business_account: Option<bool>,
    /// WhatsApp username, when one exists.
    #[serde(default)]
    pub username: Option<String>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

/// One page of WABA phone numbers.
///
/// ```
/// use meta_whatsapp::phone_number::WabaPhoneNumbersPage;
///
/// let page: WabaPhoneNumbersPage = serde_json::from_str(
///     r#"{"data":[{"id":"1"}],"paging":{"cursors":{"after":"QVFIU"}}}"#,
/// )?;
/// assert_eq!(page.next_cursor(), Some("QVFIU"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WabaPhoneNumbersPage {
    /// Phone numbers on this page.
    #[serde(default)]
    pub data: Vec<WabaPhoneNumber>,
    /// Cursor paging, when Meta sent it.
    #[serde(default)]
    pub paging: Option<Paging>,
}

impl WabaPhoneNumbersPage {
    /// Reads a list page and validates its envelope.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the body is not an object, `data` is not an
    /// array, or an entry has an unreadable known field.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::WabaPhoneNumbersPage;
    ///
    /// assert!(WabaPhoneNumbersPage::from_value(&serde_json::json!({"data": []})).is_ok());
    /// assert!(WabaPhoneNumbersPage::from_value(&serde_json::json!({"data": {}})).is_err());
    /// ```
    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Payload(
                "expected a JSON object for the WABA phone-number page".to_string(),
            ));
        }
        serde_json::from_value(value.clone()).map_err(|error| {
            Error::Payload(format!("unexpected WABA phone-number page shape: {error}"))
        })
    }

    /// Whether `paging.next` is present.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::WabaPhoneNumbersPage;
    /// assert!(!WabaPhoneNumbersPage::default().has_next_page());
    /// ```
    #[must_use]
    pub fn has_next_page(&self) -> bool {
        self.paging
            .as_ref()
            .and_then(|paging| paging.next.as_ref())
            .is_some()
    }

    /// The non-empty `after` cursor of this page.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::WabaPhoneNumbersPage;
    ///
    /// let page: WabaPhoneNumbersPage = serde_json::from_str(
    ///     r#"{"data":[],"paging":{"cursors":{"after":""}}}"#,
    /// )?;
    /// assert_eq!(page.next_cursor(), None);
    /// # Ok::<(), serde_json::Error>(())
    /// ```
    #[must_use]
    pub fn next_cursor(&self) -> Option<&str> {
        self.paging
            .as_ref()
            .and_then(|paging| paging.cursors.as_ref())
            .and_then(|cursors: &Cursors| cursors.after.as_deref())
            .filter(|cursor| !cursor.is_empty())
    }
}

/// Sort order accepted by `GET /{waba-id}/phone_numbers`.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberSort;
///
/// assert_eq!(PhoneNumberSort::CreationAsc.as_str(), "creation_time.asc");
/// assert_eq!(PhoneNumberSort::LastOnboardedDesc.as_str(), "last_onboarded_time.desc");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhoneNumberSort {
    /// Oldest created first.
    CreationAsc,
    /// Newest created first.
    CreationDesc,
    /// Least recently onboarded first.
    LastOnboardedAsc,
    /// Most recently onboarded first.
    LastOnboardedDesc,
}

impl PhoneNumberSort {
    /// The documented wire value.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberSort;
    /// assert_eq!(PhoneNumberSort::CreationDesc.as_str(), "creation_time.desc");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CreationAsc => "creation_time.asc",
            Self::CreationDesc => "creation_time.desc",
            Self::LastOnboardedAsc => "last_onboarded_time.asc",
            Self::LastOnboardedDesc => "last_onboarded_time.desc",
        }
    }
}

/// A filter accepted by the WABA phone-number list.
///
/// Values are JSON-encoded into the `filtering` query parameter.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberFilter;
///
/// let filter = PhoneNumberFilter::equal("account_mode", "LIVE")?;
/// assert_eq!(filter.to_json()["field"], "account_mode");
/// assert_eq!(filter.to_json()["operator"], "EQUAL");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneNumberFilter {
    field: PhoneNumberFilterField,
    operator: PhoneNumberFilterOperator,
    value: PhoneNumberFilterValue,
}

impl PhoneNumberFilter {
    /// Creates a filter from a documented field, operator and value.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the field, operator or value is not a
    /// documented combination.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilter;
    ///
    /// assert!(PhoneNumberFilter::new("account_mode", "EQUAL", "LIVE").is_ok());
    /// assert!(PhoneNumberFilter::new("display_phone_number", "EQUAL", "x").is_err());
    /// ```
    pub fn new(
        field: impl AsRef<str>,
        operator: impl AsRef<str>,
        value: impl Into<String>,
    ) -> Result<Self> {
        let field = PhoneNumberFilterField::parse(field.as_ref())?;
        let operator = PhoneNumberFilterOperator::parse(operator.as_ref())?;
        let value = PhoneNumberFilterValue::new(field, &operator, value.into())?;
        Ok(Self {
            field,
            operator,
            value,
        })
    }

    /// Builds an equality filter for a documented field.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] when the field or value is not documented.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilter;
    ///
    /// assert!(PhoneNumberFilter::equal("messaging_limit_tier", "TIER_1K").is_ok());
    /// assert!(PhoneNumberFilter::equal("account_mode", "BETA").is_err());
    /// ```
    pub fn equal(field: impl AsRef<str>, value: impl Into<String>) -> Result<Self> {
        Self::new(field, "EQUAL", value)
    }

    /// Renders the filter object inside the JSON array.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilter;
    ///
    /// let filter = PhoneNumberFilter::equal("is_official_business_account", "true")?;
    /// assert_eq!(filter.to_json()["value"], "true");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "field": self.field.as_str(),
            "operator": self.operator.as_str(),
            "value": self.value.as_str(),
        })
    }
}

/// One documented filterable field.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberFilterField;
///
/// assert_eq!(PhoneNumberFilterField::parse("account_mode")?.as_str(), "account_mode");
/// assert!(PhoneNumberFilterField::parse("id").is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PhoneNumberFilterField {
    /// Environment mode.
    AccountMode,
    /// Daily unique-recipient tier.
    MessagingLimitTier,
    /// Official business account flag.
    IsOfficialBusinessAccount,
}

impl PhoneNumberFilterField {
    /// Parses one of the three documented field names.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `filtering.field` for any other name.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilterField;
    ///
    /// assert!(PhoneNumberFilterField::parse("messaging_limit_tier").is_ok());
    /// assert!(PhoneNumberFilterField::parse("owner").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "account_mode" => Ok(Self::AccountMode),
            "messaging_limit_tier" => Ok(Self::MessagingLimitTier),
            "is_official_business_account" => Ok(Self::IsOfficialBusinessAccount),
            _ => Err(Error::validation(
                "filtering.field",
                "supported fields are account_mode, messaging_limit_tier and is_official_business_account",
            )),
        }
    }

    /// The wire field name.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilterField;
    /// assert_eq!(PhoneNumberFilterField::AccountMode.as_str(), "account_mode");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccountMode => "account_mode",
            Self::MessagingLimitTier => "messaging_limit_tier",
            Self::IsOfficialBusinessAccount => "is_official_business_account",
        }
    }
}

/// Operator accepted inside a phone-number list filter.
///
/// ```
/// use meta_whatsapp::phone_number::PhoneNumberFilterOperator;
///
/// assert_eq!(PhoneNumberFilterOperator::Equal.as_str(), "EQUAL");
/// assert!(PhoneNumberFilterOperator::parse("LIKE").is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhoneNumberFilterOperator {
    /// Exact match.
    Equal,
}

impl PhoneNumberFilterOperator {
    /// Parses the documented operator (case-insensitive).
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `filtering.operator` for anything except
    /// `EQUAL`.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilterOperator;
    ///
    /// assert!(PhoneNumberFilterOperator::parse("equal").is_ok());
    /// assert!(PhoneNumberFilterOperator::parse("EQUAL_TO").is_err());
    /// ```
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.eq_ignore_ascii_case("EQUAL") {
            Ok(Self::Equal)
        } else {
            Err(Error::validation(
                "filtering.operator",
                "supported operator is EQUAL",
            ))
        }
    }

    /// The documented wire spelling.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberFilterOperator;
    /// assert_eq!(PhoneNumberFilterOperator::Equal.as_str(), "EQUAL");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        "EQUAL"
    }
}

/// Validated value of a phone-number filter.
///
/// ```
/// use meta_whatsapp::phone_number::{PhoneNumberFilterField, PhoneNumberFilterOperator, PhoneNumberFilterValue};
///
/// let operator = PhoneNumberFilterOperator::Equal;
/// let value = PhoneNumberFilterValue::new(
///     PhoneNumberFilterField::AccountMode,
///     &operator,
///     "SANDBOX".into(),
/// )?;
/// assert_eq!(value.as_str(), "SANDBOX");
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PhoneNumberFilterValue(String);

impl PhoneNumberFilterValue {
    /// Validates the value against the field's documented domain.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming `filtering.value` for an unknown mode or
    /// tier, or a non-boolean official-business value.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{PhoneNumberFilterField, PhoneNumberFilterOperator, PhoneNumberFilterValue};
    ///
    /// let operator = PhoneNumberFilterOperator::Equal;
    /// assert!(PhoneNumberFilterValue::new(PhoneNumberFilterField::AccountMode, &operator, "LIVE".into()).is_ok());
    /// assert!(PhoneNumberFilterValue::new(PhoneNumberFilterField::AccountMode, &operator, "PROD".into()).is_err());
    /// ```
    pub fn new(
        field: PhoneNumberFilterField,
        _operator: &PhoneNumberFilterOperator,
        raw: String,
    ) -> Result<Self> {
        let valid = match field {
            PhoneNumberFilterField::AccountMode => {
                raw.eq_ignore_ascii_case("LIVE") || raw.eq_ignore_ascii_case("SANDBOX")
            }
            PhoneNumberFilterField::MessagingLimitTier => matches!(
                raw.as_str(),
                "TIER_50" | "TIER_250" | "TIER_1K" | "TIER_10K" | "TIER_100K" | "TIER_UNLIMITED"
            ),
            PhoneNumberFilterField::IsOfficialBusinessAccount => {
                raw.eq_ignore_ascii_case("true") || raw.eq_ignore_ascii_case("false")
            }
        };
        if valid {
            Ok(Self(raw))
        } else {
            Err(Error::validation(
                "filtering.value",
                format!("value is not documented for {}", field.as_str()),
            ))
        }
    }

    /// The validated value.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{PhoneNumberFilterField, PhoneNumberFilterOperator, PhoneNumberFilterValue};
    ///
    /// let value = PhoneNumberFilterValue::new(
    ///     PhoneNumberFilterField::MessagingLimitTier,
    ///     &PhoneNumberFilterOperator::Equal,
    ///     "TIER_UNLIMITED".into(),
    /// )?;
    /// assert_eq!(value.as_str(), "TIER_UNLIMITED");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Query for `GET /{waba-id}/phone_numbers`.
///
/// ```
/// use meta_whatsapp::phone_number::{PhoneNumberQuery, PhoneNumberSort};
///
/// let query = PhoneNumberQuery::new()
///     .limit(25)
///     .after("QVFIU")
///     .sort(PhoneNumberSort::CreationDesc)
///     .fields(["id", "display_phone_number"]);
/// assert!(query.query_string().contains("limit=25"));
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PhoneNumberQuery {
    /// Page size, 1–100 when set.
    pub limit: Option<u32>,
    /// Cursor for the next page.
    pub after: Option<String>,
    /// Cursor for the previous page.
    pub before: Option<String>,
    /// Fields to request.
    pub fields: Vec<String>,
    /// JSON-encoded filters.
    pub filters: Vec<PhoneNumberFilter>,
    /// Optional sort order.
    pub sort: Option<PhoneNumberSort>,
}

impl PhoneNumberQuery {
    /// An unfiltered first page at Meta's default size.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    /// assert_eq!(PhoneNumberQuery::new().query_string(), "");
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the page size.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    /// assert_eq!(PhoneNumberQuery::new().limit(100).limit, Some(100));
    /// ```
    #[must_use]
    pub fn limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Sets the next-page cursor.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    /// assert_eq!(PhoneNumberQuery::new().after("QVFIU").after.as_deref(), Some("QVFIU"));
    /// ```
    #[must_use]
    pub fn after(mut self, cursor: impl Into<String>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Sets the previous-page cursor.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    /// assert_eq!(PhoneNumberQuery::new().before("QVFIU").before.as_deref(), Some("QVFIU"));
    /// ```
    #[must_use]
    pub fn before(mut self, cursor: impl Into<String>) -> Self {
        self.before = Some(cursor.into());
        self
    }

    /// Selects response fields.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    /// assert_eq!(PhoneNumberQuery::new().fields(["id", "status"]).fields.len(), 2);
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

    /// Adds one documented filter. Repeating the same field is rejected by
    /// [`PhoneNumberQuery::validate`].
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{PhoneNumberFilter, PhoneNumberQuery};
    ///
    /// let query = PhoneNumberQuery::new().filter(PhoneNumberFilter::equal("account_mode", "LIVE")?);
    /// assert_eq!(query.filters.len(), 1);
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn filter(mut self, filter: PhoneNumberFilter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Sets the sort order.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{PhoneNumberQuery, PhoneNumberSort};
    /// assert_eq!(PhoneNumberQuery::new().sort(PhoneNumberSort::LastOnboardedDesc).sort.unwrap().as_str(), "last_onboarded_time.desc");
    /// ```
    #[must_use]
    pub fn sort(mut self, sort: PhoneNumberSort) -> Self {
        self.sort = Some(sort);
        self
    }

    /// Checks all query constraints before a URL is built.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a page size outside 1–100, both cursors set,
    /// blank/control cursors, malformed field names, duplicate filter fields,
    /// or unsafe filter values.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::PhoneNumberQuery;
    ///
    /// assert!(PhoneNumberQuery::new().limit(1).after("a").validate().is_ok());
    /// assert!(PhoneNumberQuery::new().limit(0).validate().is_err());
    /// assert!(PhoneNumberQuery::new().after("a").before("b").validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        if let Some(limit) = self.limit {
            if !(PHONE_NUMBER_PAGE_MIN..=PHONE_NUMBER_PAGE_MAX).contains(&limit) {
                return Err(Error::validation(
                    "limit",
                    format!(
                        "must be within {PHONE_NUMBER_PAGE_MIN}..={PHONE_NUMBER_PAGE_MAX}, got {limit}"
                    ),
                ));
            }
        }
        if self.after.is_some() && self.before.is_some() {
            return Err(Error::validation(
                "after",
                "after and before cannot be used together",
            ));
        }
        for (name, cursor) in [("after", &self.after), ("before", &self.before)] {
            if let Some(cursor) = cursor {
                if cursor.trim().is_empty() || cursor.chars().any(char::is_control) {
                    return Err(Error::validation(
                        name,
                        "must not be empty or contain control characters",
                    ));
                }
            }
        }
        if !self.fields.is_empty() {
            crate::business::validate_field_list(&self.fields.join(","))?;
        }
        for (index, filter) in self.filters.iter().enumerate() {
            if self.filters[..index]
                .iter()
                .any(|other| other.field == filter.field)
            {
                return Err(Error::validation(
                    "filtering",
                    format!("duplicate filter field {}", filter.field.as_str()),
                ));
            }
            if filter.value.as_str().chars().any(char::is_control) {
                return Err(Error::validation(
                    "filtering.value",
                    "must not contain control characters",
                ));
            }
        }
        Ok(())
    }

    /// Renders the query string without the leading `?`.
    ///
    /// ```
    /// use meta_whatsapp::phone_number::{PhoneNumberQuery, PhoneNumberSort};
    ///
    /// let query = PhoneNumberQuery::new().limit(25).sort(PhoneNumberSort::CreationDesc);
    /// assert_eq!(query.query_string(), "limit=25&sort=creation_time.desc");
    /// ```
    #[must_use]
    pub fn query_string(&self) -> String {
        let mut parts = Vec::new();
        if let Some(limit) = self.limit {
            parts.push(format!("limit={limit}"));
        }
        for (key, value) in [("after", &self.after), ("before", &self.before)] {
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
        if !self.filters.is_empty() {
            let filters = Value::Array(
                self.filters
                    .iter()
                    .map(PhoneNumberFilter::to_json)
                    .collect(),
            );
            parts.push(format!(
                "filtering={}",
                encode_query_value(&filters.to_string())
            ));
        }
        if let Some(sort) = self.sort {
            parts.push(format!("sort={}", sort.as_str()));
        }
        parts.join("&")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_boundaries_are_checked_on_both_sides() {
        let min = "1".repeat(PHONE_NUMBER_MIN_DIGITS);
        let max = "1".repeat(PHONE_NUMBER_MAX_DIGITS);
        assert!(PhoneNumberCreateRequest::new(min.clone(), "AB").is_ok());
        assert!(PhoneNumberCreateRequest::new(max.clone(), "AB").is_ok());
        assert!(PhoneNumberCreateRequest::new(min[1..].to_string(), "AB").is_err());
        assert!(PhoneNumberCreateRequest::new(format!("{max}1"), "AB").is_err());

        let name_min = "A".repeat(VERIFIED_NAME_MIN_CHARS);
        let name_max = "A".repeat(VERIFIED_NAME_MAX_CHARS);
        assert!(PhoneNumberCreateRequest::new("16315551000", name_min).is_ok());
        assert!(PhoneNumberCreateRequest::new("16315551000", format!("{name_max}A")).is_err());
        assert!(
            PhoneNumberCreateRequest::new("16315551000", format!("{name_max}😀")).is_err(),
            "the schema counts characters, and 76 is outside"
        );
    }

    #[test]
    fn code_redaction_covers_request_and_errors() {
        let code = VerificationCode::new("123456").unwrap();
        assert_eq!(format!("{code:?}"), "VerificationCode([redacted])");
        assert_eq!(format!("{code}"), "6-digit verification code (redacted)");
        let request = VerifyCodeRequest::new(&code);
        assert!(!format!("{request:?}").contains("123456"));
        let err = VerificationCode::new("12345").unwrap_err();
        assert!(!format!("{err}").contains("12345"));
    }
}
