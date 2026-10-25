//! Business account surfaces: profile, phone-number quality and messaging
//! limits, and the blocklist.
//!
//! These are account administration calls, not messaging, but they belong to
//! the same credential and the same client, and "did my number get restricted?"
//! is a question every sender eventually asks.
//!
//! Contract source: the published OpenAPI description of the Cloud API
//! (`business-messaging-api_v23.0.yaml`, Meta). Every shape here is derived
//! from it; the parts it does not pin down are marked *unverified* in the
//! README rather than guessed at.
//!
//! ```rust
//! use meta_whatsapp::business::{BusinessProfile, QualityRating};
//!
//! let profile = BusinessProfile::parse(&serde_json::json!({
//!     "data": [{ "business_profile": { "about": "Kaffee", "vertical": "RESTAURANT" } }]
//! }))?;
//! assert_eq!(profile.about.as_deref(), Some("Kaffee"));
//!
//! let rating: QualityRating = serde_json::from_str("\"green\"").unwrap();
//! assert_eq!(rating, QualityRating::Green);
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::template::Paging;

/// Maximum length of the profile `about` text, in characters.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_ABOUT_MAX_CHARS, 139);
/// ```
pub const PROFILE_ABOUT_MAX_CHARS: usize = 139;

/// Maximum length of the profile address, in characters.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_ADDRESS_MAX_CHARS, 256);
/// ```
pub const PROFILE_ADDRESS_MAX_CHARS: usize = 256;

/// Maximum length of the profile description, in characters.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_DESCRIPTION_MAX_CHARS, 256);
/// ```
pub const PROFILE_DESCRIPTION_MAX_CHARS: usize = 256;

/// Maximum length of the profile e-mail address, in characters.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_EMAIL_MAX_CHARS, 128);
/// ```
pub const PROFILE_EMAIL_MAX_CHARS: usize = 128;

/// How many websites a profile may list.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_WEBSITES_MAX, 2);
/// ```
pub const PROFILE_WEBSITES_MAX: usize = 2;

/// Maximum length of a single profile website URL.
///
/// ```
/// assert_eq!(meta_whatsapp::business::PROFILE_WEBSITE_MAX_CHARS, 256);
/// ```
pub const PROFILE_WEBSITE_MAX_CHARS: usize = 256;

/// Shortest accepted blocklist number, in digits.
///
/// ```
/// assert_eq!(meta_whatsapp::business::BLOCK_USER_MIN_DIGITS, 7);
/// ```
pub const BLOCK_USER_MIN_DIGITS: usize = 7;

/// Longest accepted blocklist number, in digits (E.164 ceiling).
///
/// ```
/// assert_eq!(meta_whatsapp::business::BLOCK_USER_MAX_DIGITS, 15);
/// ```
pub const BLOCK_USER_MAX_DIGITS: usize = 15;

/// Fields requested by default when reading the phone-number node.
///
/// The Adapter default of the Graph API is only `id`, `display_phone_number`,
/// `verified_name` and `quality_rating`; asking for the rest explicitly is what
/// makes quality *and* limits available in one call.
///
/// ```
/// assert!(meta_whatsapp::business::PHONE_NUMBER_FIELDS.contains("messaging_limit_tier"));
/// ```
pub const PHONE_NUMBER_FIELDS: &str = "id,display_phone_number,verified_name,quality_rating,code_verification_status,name_status,messaging_limit_tier,account_mode,is_official_business_account";

/// Quality rating Meta assigns to a phone number.
///
/// [`QualityRating::Unknown`] carries the raw string, so a rating Meta adds
/// later still parses instead of failing the whole response.
///
/// ```
/// use meta_whatsapp::business::QualityRating;
///
/// assert_eq!(QualityRating::parse("GREEN"), QualityRating::Green);
/// assert_eq!(QualityRating::parse("red"), QualityRating::Red);
/// assert_eq!(QualityRating::parse("NA"), QualityRating::Na);
/// assert_eq!(
///     QualityRating::parse("SILVER"),
///     QualityRating::Unknown("SILVER".into())
/// );
/// assert_eq!(QualityRating::Green.as_str(), "GREEN");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QualityRating {
    /// High quality: messages are delivered and engaged with well.
    Green,
    /// Medium quality: some delivery or engagement issues.
    Yellow,
    /// Low quality: significant delivery or engagement problems.
    Red,
    /// Not yet determined (new numbers).
    Na,
    /// A rating this crate does not know; the raw string is kept.
    Unknown(String),
}

impl QualityRating {
    /// Parses a rating, case-insensitively and without failing on new values.
    ///
    /// ```
    /// use meta_whatsapp::business::QualityRating;
    ///
    /// assert_eq!(QualityRating::parse(" yellow "), QualityRating::Yellow);
    /// assert_eq!(QualityRating::parse(""), QualityRating::Unknown(String::new()));
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "GREEN" => Self::Green,
            "YELLOW" => Self::Yellow,
            "RED" => Self::Red,
            "NA" => Self::Na,
            _ => Self::Unknown(raw.trim().to_string()),
        }
    }

    /// The wire spelling (`GREEN`, `YELLOW`, `RED`, `NA`, or the raw string for
    /// an unknown rating).
    ///
    /// ```
    /// use meta_whatsapp::business::QualityRating;
    /// assert_eq!(QualityRating::Na.as_str(), "NA");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Green => "GREEN",
            Self::Yellow => "YELLOW",
            Self::Red => "RED",
            Self::Na => "NA",
            Self::Unknown(raw) => raw,
        }
    }

    /// Conservative local helper: `false` only for [`QualityRating::Red`],
    /// Meta's flag that a number is at risk of being restricted.
    ///
    /// This is *our* policy, not an API field — `YELLOW` is a warning, `NA`
    /// means not yet rated, and an unknown rating is not assumed to be bad.
    ///
    /// ```
    /// use meta_whatsapp::business::QualityRating;
    ///
    /// assert!(QualityRating::Green.permits_sending());
    /// assert!(QualityRating::Yellow.permits_sending());
    /// assert!(QualityRating::Na.permits_sending());
    /// assert!(!QualityRating::Red.permits_sending());
    /// ```
    #[must_use]
    pub fn permits_sending(&self) -> bool {
        !matches!(self, Self::Red)
    }
}

impl std::fmt::Display for QualityRating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for QualityRating {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for QualityRating {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// Daily unique-recipient tier Meta grants a phone number.
///
/// [`MessagingLimitTier::Unknown`] keeps the raw string so a new tier does not
/// break parsing.
///
/// ```
/// use meta_whatsapp::business::MessagingLimitTier;
///
/// assert_eq!(MessagingLimitTier::parse("TIER_1K"), MessagingLimitTier::Tier1K);
/// assert_eq!(MessagingLimitTier::Tier1K.unique_contacts_per_24h(), Some(1000));
/// assert!(MessagingLimitTier::Unlimited.is_unlimited());
/// assert_eq!(MessagingLimitTier::parse("TIER_1M"), MessagingLimitTier::Unknown("TIER_1M".into()));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MessagingLimitTier {
    /// 50 unique recipients per 24 hours.
    Tier50,
    /// 250 unique recipients per 24 hours.
    Tier250,
    /// 1 000 unique recipients per 24 hours.
    Tier1K,
    /// 10 000 unique recipients per 24 hours.
    Tier10K,
    /// 100 000 unique recipients per 24 hours.
    Tier100K,
    /// No daily cap.
    Unlimited,
    /// A tier this crate does not know; the raw string is kept.
    Unknown(String),
}

impl MessagingLimitTier {
    /// Parses a tier name, case-insensitively, keeping unknown values.
    ///
    /// ```
    /// use meta_whatsapp::business::MessagingLimitTier;
    ///
    /// assert_eq!(MessagingLimitTier::parse("tier_100k"), MessagingLimitTier::Tier100K);
    /// assert_eq!(MessagingLimitTier::parse(""), MessagingLimitTier::Unknown(String::new()));
    /// ```
    #[must_use]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_uppercase().as_str() {
            "TIER_50" => Self::Tier50,
            "TIER_250" => Self::Tier250,
            "TIER_1K" => Self::Tier1K,
            "TIER_10K" => Self::Tier10K,
            "TIER_100K" => Self::Tier100K,
            "TIER_UNLIMITED" => Self::Unlimited,
            _ => Self::Unknown(raw.trim().to_string()),
        }
    }

    /// The wire spelling, or the raw string for an unknown tier.
    ///
    /// ```
    /// use meta_whatsapp::business::MessagingLimitTier;
    /// assert_eq!(MessagingLimitTier::Tier50.as_str(), "TIER_50");
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Tier50 => "TIER_50",
            Self::Tier250 => "TIER_250",
            Self::Tier1K => "TIER_1K",
            Self::Tier10K => "TIER_10K",
            Self::Tier100K => "TIER_100K",
            Self::Unlimited => "TIER_UNLIMITED",
            Self::Unknown(raw) => raw,
        }
    }

    /// Unique recipients allowed per 24 hours.
    ///
    /// `None` for an unlimited tier *and* for a tier this crate does not know —
    /// check [`MessagingLimitTier::is_unlimited`] to tell the two apart.
    ///
    /// ```
    /// use meta_whatsapp::business::MessagingLimitTier;
    ///
    /// assert_eq!(MessagingLimitTier::Tier50.unique_contacts_per_24h(), Some(50));
    /// assert_eq!(MessagingLimitTier::Unlimited.unique_contacts_per_24h(), None);
    /// assert_eq!(MessagingLimitTier::Unknown("TIER_X".into()).unique_contacts_per_24h(), None);
    /// ```
    #[must_use]
    pub fn unique_contacts_per_24h(&self) -> Option<u64> {
        match self {
            Self::Tier50 => Some(50),
            Self::Tier250 => Some(250),
            Self::Tier1K => Some(1_000),
            Self::Tier10K => Some(10_000),
            Self::Tier100K => Some(100_000),
            Self::Unlimited | Self::Unknown(_) => None,
        }
    }

    /// Whether the number has no daily cap.
    ///
    /// ```
    /// use meta_whatsapp::business::MessagingLimitTier;
    ///
    /// assert!(MessagingLimitTier::Unlimited.is_unlimited());
    /// assert!(!MessagingLimitTier::Tier250.is_unlimited());
    /// ```
    #[must_use]
    pub fn is_unlimited(&self) -> bool {
        matches!(self, Self::Unlimited)
    }
}

impl std::fmt::Display for MessagingLimitTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for MessagingLimitTier {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for MessagingLimitTier {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::parse(&raw))
    }
}

/// A business profile as returned by
/// `GET /{phone-number-id}/whatsapp_business_profile`.
///
/// Parsing accepts the flat object and both documented wrappings
/// (`{"data":[{"business_profile":{…}}]}` and `{"data":[{…}]}`), because the
/// published reference and the OpenAPI examples disagree on the envelope. See
/// *unverified* in the README.
///
/// ```
/// use meta_whatsapp::business::BusinessProfile;
///
/// let profile = BusinessProfile::parse(&serde_json::json!({
///     "messaging_product": "whatsapp",
///     "about": "Kaffee seit 1998",
///     "email": "hallo@example.test",
///     "websites": ["https://example.test"],
///     "vertical": "RESTAURANT",
///     "fields_added_later": 42
/// }))?;
/// assert_eq!(profile.about.as_deref(), Some("Kaffee seit 1998"));
/// assert_eq!(profile.websites, ["https://example.test"]);
/// assert_eq!(profile.extra["fields_added_later"], 42);
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BusinessProfile {
    /// Always `whatsapp` when present.
    #[serde(default)]
    pub messaging_product: Option<String>,
    /// About text (up to 139 characters).
    #[serde(default)]
    pub about: Option<String>,
    /// Street address (up to 256 characters).
    #[serde(default)]
    pub address: Option<String>,
    /// Description (up to 256 characters).
    #[serde(default)]
    pub description: Option<String>,
    /// Contact e-mail (up to 128 characters).
    #[serde(default)]
    pub email: Option<String>,
    /// URL of the profile picture Meta hosts.
    #[serde(default)]
    pub profile_picture_url: Option<String>,
    /// Up to two website URLs.
    #[serde(default)]
    pub websites: Vec<String>,
    /// Business vertical, e.g. `RESTAURANT`.
    ///
    /// Kept as a string: the vertical list grows with Meta's categories.
    #[serde(default)]
    pub vertical: Option<String>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl BusinessProfile {
    /// Reads a profile out of the JSON the API returned.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the value is an array, a scalar, or an object
    /// whose `data` entry holds no object.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfile;
    ///
    /// assert!(BusinessProfile::parse(&serde_json::json!({"about": "x"})).is_ok());
    /// assert!(BusinessProfile::parse(&serde_json::json!([{"about": "x"}])).is_err());
    /// assert!(BusinessProfile::parse(&serde_json::json!({"data": []})).is_err());
    /// ```
    pub fn parse(value: &Value) -> Result<Self> {
        let inner = unwrap_first_object(value, &["business_profile"])?;
        serde_json::from_value(inner.clone())
            .map_err(|error| Error::Payload(format!("unexpected business profile shape: {error}")))
    }
}

/// Peels the wrappings Graph API responses use, returning the object that
/// carries the fields.
///
/// Accepts `{…}`, `{"data":[{…}]}` and `{"data":[{"<inner>":{…}}]}`.
fn unwrap_first_object<'a>(value: &'a Value, inner_keys: &[&str]) -> Result<&'a Value> {
    let candidate = match value {
        Value::Object(map) => match map.get("data") {
            Some(Value::Array(entries)) => entries
                .first()
                .map_or_else(|| Err(Error::Payload("`data` was empty".to_string())), Ok)?,
            Some(_) => {
                return Err(Error::Payload(
                    "`data` was not an array of objects".to_string(),
                ))
            }
            None => value,
        },
        _ => {
            return Err(Error::Payload(
                "expected a JSON object for the Graph API response".to_string(),
            ))
        }
    };

    if let Value::Object(map) = candidate {
        for key in inner_keys {
            if let Some(nested @ Value::Object(_)) = map.get(*key) {
                return Ok(nested);
            }
        }
        Ok(candidate)
    } else {
        Err(Error::Payload(
            "expected a JSON object inside `data`".to_string(),
        ))
    }
}

/// An update to the business profile (`POST
/// /{phone-number-id}/whatsapp_business_profile`).
///
/// Only the fields you set are sent; `messaging_product` is added
/// automatically. Every setter is chainable, and
/// [`BusinessProfileUpdate::validate`] enforces the documented limits before a
/// request is built.
///
/// ```
/// use meta_whatsapp::business::BusinessProfileUpdate;
///
/// let update = BusinessProfileUpdate::new()
///     .about("Kaffee seit 1998")
///     .email("hallo@example.test")
///     .website("https://example.test")
///     .vertical("RESTAURANT");
///
/// let payload = update.to_json()?;
/// assert_eq!(payload["messaging_product"], "whatsapp");
/// assert_eq!(payload["vertical"], "RESTAURANT");
/// assert!(payload.get("address").is_none());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct BusinessProfileUpdate {
    /// About text.
    pub about: Option<String>,
    /// Street address.
    pub address: Option<String>,
    /// Description.
    pub description: Option<String>,
    /// Contact e-mail.
    pub email: Option<String>,
    /// Handle from the Resumable Upload API, for a new profile picture.
    pub profile_picture_handle: Option<String>,
    /// Website URLs (at most two).
    pub websites: Vec<String>,
    /// Business vertical.
    pub vertical: Option<String>,
}

impl BusinessProfileUpdate {
    /// An empty update: nothing changes until a setter is called.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    ///
    /// assert_eq!(BusinessProfileUpdate::new().to_json().unwrap()["messaging_product"], "whatsapp");
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the about text.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert_eq!(BusinessProfileUpdate::new().about("Kaffee").about.unwrap(), "Kaffee");
    /// ```
    #[must_use]
    pub fn about(mut self, about: impl Into<String>) -> Self {
        self.about = Some(about.into());
        self
    }

    /// Sets the address.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert!(BusinessProfileUpdate::new().address("Hauptstr. 1").address.is_some());
    /// ```
    #[must_use]
    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    /// Sets the description.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert!(BusinessProfileUpdate::new().description("Rösterei").description.is_some());
    /// ```
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the contact e-mail.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert!(BusinessProfileUpdate::new().email("a@b.test").email.is_some());
    /// ```
    #[must_use]
    pub fn email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }

    /// Sets the profile-picture handle obtained from the Resumable Upload API.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert!(BusinessProfileUpdate::new()
    ///     .profile_picture_handle("4:abc:e:1")
    ///     .profile_picture_handle
    ///     .is_some());
    /// ```
    #[must_use]
    pub fn profile_picture_handle(mut self, handle: impl Into<String>) -> Self {
        self.profile_picture_handle = Some(handle.into());
        self
    }

    /// Appends a website URL.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert_eq!(BusinessProfileUpdate::new().website("https://a.test").websites.len(), 1);
    /// ```
    #[must_use]
    pub fn website(mut self, url: impl Into<String>) -> Self {
        self.websites.push(url.into());
        self
    }

    /// Sets the business vertical.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    /// assert_eq!(BusinessProfileUpdate::new().vertical("RETAIL").vertical.unwrap(), "RETAIL");
    /// ```
    #[must_use]
    pub fn vertical(mut self, vertical: impl Into<String>) -> Self {
        self.vertical = Some(vertical.into());
        self
    }

    /// Checks every field against the documented limits.
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] naming the offending field: `about` must be
    /// 1–139 characters, `address`/`description` at most 256, `email` at most
    /// 128 and shaped like an address, at most two `https://` websites of at
    /// most 256 characters, and a non-empty `vertical`.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    ///
    /// assert!(BusinessProfileUpdate::new().about("Kaffee").validate().is_ok());
    /// assert!(BusinessProfileUpdate::new().about("").validate().is_err());
    /// assert!(BusinessProfileUpdate::new()
    ///     .website("http://insecure.test")
    ///     .validate()
    ///     .is_err());
    /// ```
    pub fn validate(&self) -> Result<()> {
        if let Some(about) = &self.about {
            let chars = about.chars().count();
            if chars == 0 || chars > PROFILE_ABOUT_MAX_CHARS {
                return Err(Error::validation(
                    "profile.about",
                    format!("must be 1..={PROFILE_ABOUT_MAX_CHARS} characters, got {chars}"),
                ));
            }
        }
        for (field, value, max) in [
            (
                "profile.address",
                self.address.as_deref(),
                PROFILE_ADDRESS_MAX_CHARS,
            ),
            (
                "profile.description",
                self.description.as_deref(),
                PROFILE_DESCRIPTION_MAX_CHARS,
            ),
            (
                "profile.email",
                self.email.as_deref(),
                PROFILE_EMAIL_MAX_CHARS,
            ),
            (
                "profile.profile_picture_handle",
                self.profile_picture_handle.as_deref(),
                512,
            ),
            ("profile.vertical", self.vertical.as_deref(), 64),
        ] {
            if let Some(value) = value {
                let chars = value.chars().count();
                if chars > max {
                    return Err(Error::validation(
                        field,
                        format!("must be at most {max} characters, got {chars}"),
                    ));
                }
            }
        }

        if let Some(email) = &self.email {
            if !looks_like_email(email) {
                return Err(Error::validation(
                    "profile.email",
                    "must contain one '@' with text on both sides",
                ));
            }
        }
        if let Some(vertical) = &self.vertical {
            if vertical.is_empty() {
                return Err(Error::validation("profile.vertical", "must not be empty"));
            }
        }
        if self.websites.len() > PROFILE_WEBSITES_MAX {
            return Err(Error::validation(
                "profile.websites",
                format!(
                    "at most {PROFILE_WEBSITES_MAX} websites, got {}",
                    self.websites.len()
                ),
            ));
        }
        for url in &self.websites {
            if !url.starts_with("https://") {
                return Err(Error::validation(
                    "profile.websites",
                    "must be an https:// URL",
                ));
            }
            let chars = url.chars().count();
            if chars > PROFILE_WEBSITE_MAX_CHARS {
                return Err(Error::validation(
                    "profile.websites",
                    format!("must be at most {PROFILE_WEBSITE_MAX_CHARS} characters, got {chars}"),
                ));
            }
        }
        Ok(())
    }

    /// Validates and renders the request body.
    ///
    /// # Errors
    ///
    /// Whatever [`BusinessProfileUpdate::validate`] rejects.
    ///
    /// ```
    /// use meta_whatsapp::business::BusinessProfileUpdate;
    ///
    /// let payload = BusinessProfileUpdate::new().description("Rösterei").to_json()?;
    /// assert_eq!(payload["description"], "Rösterei");
    /// assert_eq!(payload.as_object().unwrap().len(), 2, "description + messaging_product");
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn to_json(&self) -> Result<Value> {
        self.validate()?;
        let mut map = Map::new();
        map.insert("messaging_product".into(), Value::String("whatsapp".into()));
        for (key, value) in [
            ("about", self.about.as_deref()),
            ("address", self.address.as_deref()),
            ("description", self.description.as_deref()),
            ("email", self.email.as_deref()),
            (
                "profile_picture_handle",
                self.profile_picture_handle.as_deref(),
            ),
            ("vertical", self.vertical.as_deref()),
        ] {
            if let Some(value) = value {
                map.insert(key.into(), Value::String(value.to_string()));
            }
        }
        if !self.websites.is_empty() {
            map.insert(
                "websites".into(),
                Value::Array(
                    self.websites
                        .iter()
                        .map(|url| Value::String(url.clone()))
                        .collect(),
                ),
            );
        }
        Ok(Value::Object(map))
    }
}

/// Whether `email` is shaped like an address (one `@`, text on both sides).
fn looks_like_email(email: &str) -> bool {
    let mut parts = email.split('@');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(local), Some(domain), None) => !local.is_empty() && !domain.is_empty(),
        _ => false,
    }
}

/// The phone-number node: quality rating, limits and verification state.
///
/// ```
/// use meta_whatsapp::business::{MessagingLimitTier, PhoneNumberInfo, QualityRating};
///
/// let info = PhoneNumberInfo::parse(&serde_json::json!({
///     "id": "106540352242922",
///     "display_phone_number": "+1 555-555-5555",
///     "verified_name": "Jasper's Market",
///     "quality_rating": "GREEN",
///     "messaging_limit_tier": "TIER_1K",
///     "account_mode": "LIVE"
/// }))?;
///
/// assert_eq!(info.quality_rating, Some(QualityRating::Green));
/// assert_eq!(info.messaging_limit_tier, Some(MessagingLimitTier::Tier1K));
/// assert!(info.permits_broadcast());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PhoneNumberInfo {
    /// Phone number id (the id every send uses).
    #[serde(default)]
    pub id: Option<String>,
    /// Number in international display form.
    #[serde(default)]
    pub display_phone_number: Option<String>,
    /// Verified business name.
    #[serde(default)]
    pub verified_name: Option<String>,
    /// Quality rating.
    #[serde(default)]
    pub quality_rating: Option<QualityRating>,
    /// `VERIFIED` or `UNVERIFIED` two-step verification.
    #[serde(default)]
    pub code_verification_status: Option<String>,
    /// Display-name review status (`APPROVED`, `PENDING_REVIEW`, …).
    #[serde(default)]
    pub name_status: Option<String>,
    /// Daily unique-recipient tier.
    #[serde(default)]
    pub messaging_limit_tier: Option<MessagingLimitTier>,
    /// `LIVE` or `SANDBOX`.
    #[serde(default)]
    pub account_mode: Option<String>,
    /// Whether the number carries an official-business badge.
    #[serde(default)]
    pub is_official_business_account: Option<bool>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl PhoneNumberInfo {
    /// Reads the node out of the JSON the API returned (flat or wrapped in
    /// `data`).
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the response is not an object or a non-empty
    /// `data` array of objects.
    ///
    /// ```
    /// use meta_whatsapp::business::PhoneNumberInfo;
    ///
    /// assert_eq!(PhoneNumberInfo::parse(&serde_json::json!({"id": "1"}))?.id.as_deref(), Some("1"));
    /// assert_eq!(
    ///     PhoneNumberInfo::parse(&serde_json::json!({"data": [{"id": "2"}]}))?.id.as_deref(),
    ///     Some("2")
    /// );
    /// assert!(PhoneNumberInfo::parse(&serde_json::json!("nope")).is_err());
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    pub fn parse(value: &Value) -> Result<Self> {
        let inner = unwrap_first_object(value, &[])?;
        serde_json::from_value(inner.clone())
            .map_err(|error| Error::Payload(format!("unexpected phone number shape: {error}")))
    }

    /// Conservative local helper: quality is not [`QualityRating::Red`] and the
    /// account is not in `SANDBOX` mode.
    ///
    /// Both rules are this crate's policy, derived from the documented meaning
    /// of the values — not a server-side flag.
    ///
    /// ```
    /// use meta_whatsapp::business::PhoneNumberInfo;
    ///
    /// let live: PhoneNumberInfo = serde_json::from_str(r#"{"quality_rating":"GREEN","account_mode":"LIVE"}"#).unwrap();
    /// assert!(live.permits_broadcast());
    ///
    /// let sandbox: PhoneNumberInfo = serde_json::from_str(r#"{"account_mode":"SANDBOX"}"#).unwrap();
    /// assert!(!sandbox.permits_broadcast());
    /// ```
    #[must_use]
    pub fn permits_broadcast(&self) -> bool {
        let quality_ok = self
            .quality_rating
            .as_ref()
            .is_none_or(QualityRating::permits_sending);
        let live = self
            .account_mode
            .as_deref()
            .is_none_or(|mode| mode.eq_ignore_ascii_case("LIVE"));
        quality_ok && live
    }

    /// Unique recipients this number may message in 24 hours, when the tier is
    /// known and bounded.
    ///
    /// ```
    /// use meta_whatsapp::business::PhoneNumberInfo;
    ///
    /// let info: PhoneNumberInfo = serde_json::from_str(r#"{"messaging_limit_tier":"TIER_250"}"#).unwrap();
    /// assert_eq!(info.daily_contact_limit(), Some(250));
    /// ```
    #[must_use]
    pub fn daily_contact_limit(&self) -> Option<u64> {
        self.messaging_limit_tier
            .as_ref()
            .and_then(MessagingLimitTier::unique_contacts_per_24h)
    }
}

/// One blocklist entry as returned by `GET /{phone-number-id}/block_users`.
///
/// ```
/// use meta_whatsapp::business::BlockedUser;
///
/// let user: BlockedUser = serde_json::from_str(r#"{"messaging_product":"whatsapp","wa_id":"16505551234"}"#)?;
/// assert_eq!(user.wa_id.as_deref(), Some("16505551234"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BlockedUser {
    /// Always `whatsapp` when present.
    #[serde(default)]
    pub messaging_product: Option<String>,
    /// WhatsApp id of the blocked user (digits, no `+`).
    #[serde(default)]
    pub wa_id: Option<String>,
}

/// One page of the blocklist.
///
/// ```
/// use meta_whatsapp::business::BlockedUsersPage;
///
/// let page: BlockedUsersPage = serde_json::from_str(
///     r#"{"data":[{"wa_id":"16505551234"}],"paging":{"next":"https://graph.facebook.com/next"}}"#,
/// )?;
/// assert_eq!(page.data[0].wa_id.as_deref(), Some("16505551234"));
/// assert!(page.has_next_page());
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BlockedUsersPage {
    /// Blocked users on this page.
    #[serde(default)]
    pub data: Vec<BlockedUser>,
    /// Paging block, when Meta sent one.
    #[serde(default)]
    pub paging: Option<Paging>,
}

impl BlockedUsersPage {
    /// Whether `paging.next` is present.
    ///
    /// ```
    /// use meta_whatsapp::business::BlockedUsersPage;
    /// assert!(!BlockedUsersPage::default().has_next_page());
    /// ```
    #[must_use]
    pub fn has_next_page(&self) -> bool {
        self.paging
            .as_ref()
            .and_then(|paging| paging.next.as_ref())
            .is_some()
    }
}

/// One entry of a block/unblock result: what was sent and what Meta resolved.
///
/// ```
/// use meta_whatsapp::business::BlockChange;
///
/// let change: BlockChange = serde_json::from_str(r#"{"input":"+16505551234","wa_id":"16505551234"}"#)?;
/// assert_eq!(change.input.as_deref(), Some("+16505551234"));
/// assert_eq!(change.wa_id.as_deref(), Some("16505551234"));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BlockChange {
    /// The number as it was sent.
    #[serde(default)]
    pub input: Option<String>,
    /// The WhatsApp id Meta resolved it to.
    #[serde(default)]
    pub wa_id: Option<String>,
}

/// Result of blocking or unblocking users.
///
/// ```
/// use meta_whatsapp::business::BlockOutcome;
///
/// let outcome = BlockOutcome::from_value(&serde_json::json!({
///     "block_users": { "added_users": [{ "input": "+16505551234", "wa_id": "16505551234" }] },
///     "messaging_product": "whatsapp"
/// }))?;
/// assert_eq!(outcome.added.len(), 1);
/// assert!(outcome.removed.is_empty());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BlockOutcome {
    /// Users added to the blocklist.
    #[serde(default)]
    pub added: Vec<BlockChange>,
    /// Users removed from the blocklist.
    #[serde(default)]
    pub removed: Vec<BlockChange>,
    /// Always `whatsapp` when present.
    #[serde(default)]
    pub messaging_product: Option<String>,
    /// Fields this crate does not model, preserved verbatim.
    #[serde(default, flatten)]
    pub extra: Map<String, Value>,
}

impl BlockOutcome {
    /// Reads the result from the JSON body.
    ///
    /// A body without a `block_users` object is accepted (all lists empty) so
    /// a shape change cannot turn a successful block into an error.
    ///
    /// # Errors
    ///
    /// [`Error::Payload`] when the value is not an object, or when `added_users`
    /// / `removed_users` are not arrays of objects.
    ///
    /// ```
    /// use meta_whatsapp::business::BlockOutcome;
    ///
    /// assert!(BlockOutcome::from_value(&serde_json::json!({"success": true})).is_ok());
    /// assert!(BlockOutcome::from_value(&serde_json::json!([1, 2])).is_err());
    /// ```
    pub fn from_value(value: &Value) -> Result<Self> {
        if !value.is_object() {
            return Err(Error::Payload(
                "expected a JSON object for the blocklist result".to_string(),
            ));
        }
        let empty = Value::Array(Vec::new());
        let block_users = value
            .get("block_users")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let added = block_users.get("added_users").cloned().unwrap_or(empty);
        let removed = block_users
            .get("removed_users")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        let messaging_product = value
            .get("messaging_product")
            .cloned()
            .unwrap_or(Value::Null);
        let mut extra = value.as_object().cloned().unwrap_or_default();
        for known in ["block_users", "messaging_product"] {
            extra.remove(known);
        }
        serde_json::from_value::<Vec<BlockChange>>(added)
            .and_then(|added| {
                let removed = serde_json::from_value::<Vec<BlockChange>>(removed)?;
                let messaging_product =
                    serde_json::from_value::<Option<String>>(messaging_product)?;
                Ok(Self {
                    added,
                    removed,
                    messaging_product,
                    extra,
                })
            })
            .map_err(|error| Error::Payload(format!("unexpected blocklist result: {error}")))
    }
}

/// Validates a number for the blocklist: optional `+`, 7–15 digits.
///
/// ```
/// use meta_whatsapp::business::validate_block_user;
///
/// assert!(validate_block_user("+16505551234").is_ok());
/// assert!(validate_block_user("16505551234").is_ok());
/// assert!(validate_block_user("").is_err());
/// assert!(validate_block_user("+1 650 555 1234").is_err(), "spaces are not digits");
/// assert!(validate_block_user("+0123456789012345678").is_err(), "too long");
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `block_users[].user`.
pub fn validate_block_user(user: &str) -> Result<()> {
    let digits = user.strip_prefix('+').unwrap_or(user);
    let valid = !digits.is_empty()
        && digits.len() >= BLOCK_USER_MIN_DIGITS
        && digits.len() <= BLOCK_USER_MAX_DIGITS
        && digits.bytes().all(|byte| byte.is_ascii_digit());
    if valid {
        Ok(())
    } else {
        Err(Error::validation(
            "block_users[].user",
            format!(
                "must be {BLOCK_USER_MIN_DIGITS}..={BLOCK_USER_MAX_DIGITS} digits with an optional leading '+', got {user:?}"
            ),
        ))
    }
}

/// Validates and renders the body for blocking or unblocking users.
///
/// The same payload serves `POST` (block) and `DELETE` (unblock) on
/// `/{phone-number-id}/block_users`; Meta wants the ids in the body in both
/// cases.
///
/// ```
/// use meta_whatsapp::business::block_users_payload;
///
/// let body = block_users_payload(&["+16505551234"])?;
/// assert_eq!(body["messaging_product"], "whatsapp");
/// assert_eq!(body["block_users"][0]["user"], "+16505551234");
/// assert!(block_users_payload(&[]).is_err(), "an empty list is a no-op request");
/// assert!(block_users_payload(&["nope"]).is_err());
/// # Ok::<(), meta_whatsapp::Error>(())
/// ```
///
/// # Errors
///
/// [`Error::Validation`] for an empty list or an invalid number.
pub fn block_users_payload(users: &[&str]) -> Result<Value> {
    if users.is_empty() {
        return Err(Error::validation(
            "block_users",
            "need at least one user number",
        ));
    }
    let mut entries = Vec::with_capacity(users.len());
    for user in users {
        validate_block_user(user)?;
        entries.push(serde_json::json!({ "user": user }));
    }
    Ok(serde_json::json!({
        "messaging_product": "whatsapp",
        "block_users": entries,
    }))
}

/// Validates a `fields` query parameter: comma-separated lowercase identifiers.
///
/// ```
/// use meta_whatsapp::business::validate_field_list;
///
/// assert!(validate_field_list("id,quality_rating").is_ok());
/// assert!(validate_field_list("").is_err());
/// assert!(validate_field_list("id&access_token=x").is_err());
/// ```
///
/// # Errors
///
/// [`Error::Validation`] naming `fields` for an empty value, a stray comma, or
/// a character outside `[a-z0-9_,]`.
pub fn validate_field_list(fields: &str) -> Result<()> {
    let valid = !fields.is_empty()
        && !fields.starts_with(',')
        && !fields.ends_with(',')
        && !fields.contains(",,")
        && fields.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b','
        });
    if valid {
        Ok(())
    } else {
        Err(Error::validation(
            "fields",
            "must be a comma-separated list of lowercase field names",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_wrappings_are_all_accepted() {
        let flat = serde_json::json!({"about": "a"});
        let nested = serde_json::json!({"data": [{"business_profile": {"about": "a"}}]});
        let plain_data = serde_json::json!({"data": [{"about": "a"}]});
        for value in [flat, nested, plain_data] {
            assert_eq!(
                BusinessProfile::parse(&value).unwrap().about.as_deref(),
                Some("a")
            );
        }
    }

    #[test]
    fn a_non_object_profile_is_a_payload_error() {
        for value in [
            serde_json::json!([]),
            serde_json::json!("x"),
            serde_json::json!({"data": {}}),
        ] {
            assert!(matches!(
                BusinessProfile::parse(&value),
                Err(Error::Payload(_))
            ));
        }
    }

    #[test]
    fn profile_limits_are_enforced_on_both_sides() {
        let long = "a".repeat(PROFILE_ABOUT_MAX_CHARS);
        assert!(BusinessProfileUpdate::new().about(&long).validate().is_ok());
        assert!(BusinessProfileUpdate::new()
            .about(format!("{long}a"))
            .validate()
            .is_err());

        let mut update = BusinessProfileUpdate::new();
        for _ in 0..=PROFILE_WEBSITES_MAX {
            update = update.website("https://example.test");
        }
        assert!(update.validate().is_err());
    }

    #[test]
    fn email_shapes_are_checked() {
        for valid in ["a@b", "a.b@c.test"] {
            assert!(BusinessProfileUpdate::new().email(valid).validate().is_ok());
        }
        for invalid in ["", "a", "@b", "a@", "a@b@c"] {
            assert!(BusinessProfileUpdate::new()
                .email(invalid)
                .validate()
                .is_err());
        }
    }

    #[test]
    fn block_user_digits_stop_at_the_boundaries() {
        assert!(
            validate_block_user(&format!("1{}", "2".repeat(BLOCK_USER_MIN_DIGITS - 1))).is_ok()
        );
        assert!(validate_block_user(&"1".repeat(BLOCK_USER_MAX_DIGITS)).is_ok());
        assert!(validate_block_user(&"1".repeat(BLOCK_USER_MAX_DIGITS + 1)).is_err());
        assert!(validate_block_user(&format!("+{}", "1".repeat(BLOCK_USER_MAX_DIGITS))).is_ok());
        assert!(validate_block_user("++1234567").is_err());
        assert!(validate_block_user("1234567ä").is_err());
    }

    #[test]
    fn quality_ratings_round_trip_through_serde() {
        for raw in ["GREEN", "YELLOW", "RED", "NA", "SILVER"] {
            let parsed: QualityRating = serde_json::from_str(&format!("\"{raw}\"")).unwrap();
            assert_eq!(
                serde_json::to_string(&parsed).unwrap(),
                format!("\"{raw}\"")
            );
        }
    }

    #[test]
    fn unknown_tiers_keep_their_raw_string() {
        let tier: MessagingLimitTier = serde_json::from_str("\"TIER_1M\"").unwrap();
        assert_eq!(tier, MessagingLimitTier::Unknown("TIER_1M".into()));
        assert_eq!(tier.as_str(), "TIER_1M");
        assert!(!tier.is_unlimited());
        assert_eq!(tier.unique_contacts_per_24h(), None);
    }

    #[test]
    fn a_sandbox_account_does_not_permit_broadcasts() {
        let info: PhoneNumberInfo =
            serde_json::from_str(r#"{"quality_rating":"RED","account_mode":"LIVE"}"#).unwrap();
        assert!(!info.permits_broadcast());
    }

    #[test]
    fn block_outcomes_read_both_directions() {
        let blocked = BlockOutcome::from_value(&serde_json::json!({
            "block_users": {"added_users": [{"input": "+1", "wa_id": "1"}]},
            "messaging_product": "whatsapp"
        }))
        .unwrap();
        assert_eq!(blocked.added.len(), 1);
        assert_eq!(blocked.extra.len(), 0);

        let unblocked = BlockOutcome::from_value(&serde_json::json!({
            "block_users": {"removed_users": [{"input": "+1", "wa_id": "1"}]}
        }))
        .unwrap();
        assert_eq!(unblocked.removed.len(), 1);
        assert_eq!(unblocked.added.len(), 0);
    }

    #[test]
    fn field_lists_reject_injection_attempts() {
        for bad in ["", ",a", "a,", "a,,b", "A", "a b", "a&b=c", "a%2Cb"] {
            assert!(
                validate_field_list(bad).is_err(),
                "{bad:?} must be rejected"
            );
        }
    }
}
