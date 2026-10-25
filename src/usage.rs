//! Typed view of Meta's rate-limit budget header.
//!
//! Every Graph API response may carry `X-Business-Use-Case-Usage`, a JSON
//! object keyed by business id whose values are the per-use-case counters Meta
//! uses for throttling. Parsing it is optional everywhere: the header is
//! absent on most responses, and a caller that ignores it loses nothing but
//! visibility.
//!
//! ```rust
//! use meta_whatsapp::usage::BusinessUseCaseUsage;
//!
//! let header = r#"{"123456":[{"type":"whatsapp","call_count":42,"total_cputime":7,"total_time":12,"estimated_time_to_regain_access":0}]}"#;
//! let usage = BusinessUseCaseUsage::from_header(header).unwrap();
//!
//! let whatsapp = usage.use_case("123456", "whatsapp").unwrap();
//! assert_eq!(whatsapp.call_count, Some(42));
//! assert_eq!(whatsapp.total_cputime, Some(7));
//! assert!(!usage.is_throttled("123456"));
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// One use case's counters for one business.
///
/// Percentages are as Meta reports them: integers, `0..=100` for the time
/// budgets, an absolute count for `call_count`.
///
/// ```rust
/// use meta_whatsapp::usage::UseCaseUsage;
///
/// let entry: UseCaseUsage = serde_json::from_str(
///     r#"{"type":"whatsapp","call_count":9,"total_cputime":80,"total_time":95,"estimated_time_to_regain_access":30}"#,
/// )?;
/// assert_eq!(entry.use_case.as_deref(), Some("whatsapp"));
/// assert_eq!(entry.total_time, Some(95));
/// assert_eq!(entry.estimated_time_to_regain_access, Some(30));
/// # Ok::<(), serde_json::Error>(())
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UseCaseUsage {
    /// Use case name, e.g. `whatsapp` or `whatsapp_business_management`.
    ///
    /// Kept as a string: Meta adds use cases without notice.
    #[serde(default, rename = "type", alias = "use_case")]
    pub use_case: Option<String>,
    /// Calls made in the current window.
    #[serde(default)]
    pub call_count: Option<u32>,
    /// Percentage of the CPU-time budget consumed.
    #[serde(default)]
    pub total_cputime: Option<u32>,
    /// Percentage of the wall-clock budget consumed.
    #[serde(default)]
    pub total_time: Option<u32>,
    /// Minutes Meta expects the throttle to last; `0` when not throttled.
    #[serde(default)]
    pub estimated_time_to_regain_access: Option<u32>,
}

impl UseCaseUsage {
    /// Whether Meta reports this use case as currently throttled.
    ///
    /// `true` when `estimated_time_to_regain_access` is greater than zero, or
    /// when either time budget has reached 100 %.
    ///
    /// ```rust
    /// use meta_whatsapp::usage::UseCaseUsage;
    ///
    /// let mut throttled = UseCaseUsage::default();
    /// throttled.estimated_time_to_regain_access = Some(5);
    /// assert!(throttled.is_throttled());
    /// assert!(!UseCaseUsage::default().is_throttled());
    /// ```
    #[must_use]
    pub fn is_throttled(&self) -> bool {
        let waiting = self
            .estimated_time_to_regain_access
            .is_some_and(|minutes| minutes > 0);
        let spent = |value: Option<u32>| value.is_some_and(|percent| percent >= 100);
        waiting || spent(self.total_cputime) || spent(self.total_time)
    }
}

/// The `X-Business-Use-Case-Usage` header, parsed and keyed by business id.
///
/// ```rust
/// use meta_whatsapp::usage::BusinessUseCaseUsage;
///
/// let header = r#"{"111":[{"type":"whatsapp","call_count":1}],"222":[]}"#;
/// let usage = BusinessUseCaseUsage::from_header(header).unwrap();
/// assert_eq!(usage.businesses().collect::<Vec<_>>(), ["111", "222"]);
/// assert_eq!(usage.for_business("111").len(), 1);
/// assert!(usage.for_business("333").is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BusinessUseCaseUsage {
    entries: BTreeMap<String, Vec<UseCaseUsage>>,
}

impl BusinessUseCaseUsage {
    /// Parses the header value; `None` when it is not the documented shape.
    ///
    /// A malformed or unexpected header never fails a request — telemetry must
    /// not break messaging — so this returns `None` instead of an error.
    ///
    /// ```rust
    /// use meta_whatsapp::usage::BusinessUseCaseUsage;
    ///
    /// assert!(BusinessUseCaseUsage::from_header("not json").is_none());
    /// assert!(BusinessUseCaseUsage::from_header("").is_none());
    /// // A JSON array where the object belongs is not accepted either.
    /// assert!(BusinessUseCaseUsage::from_header("[]").is_none());
    /// ```
    #[must_use]
    pub fn from_header(value: &str) -> Option<Self> {
        let trimmed = value.trim();
        if !trimmed.starts_with('{') {
            return None;
        }
        serde_json::from_str(trimmed).ok()
    }

    /// All business ids in the header, in stable order.
    ///
    /// ```rust
    /// use meta_whatsapp::usage::BusinessUseCaseUsage;
    ///
    /// let usage = BusinessUseCaseUsage::from_header(r#"{"9":[{"type":"whatsapp"}]}"#).unwrap();
    /// assert_eq!(usage.businesses().count(), 1);
    /// ```
    pub fn businesses(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The use-case entries recorded for one business (empty when absent).
    ///
    /// ```rust
    /// use meta_whatsapp::usage::BusinessUseCaseUsage;
    ///
    /// let usage = BusinessUseCaseUsage::from_header(r#"{"9":[{"type":"whatsapp"}]}"#).unwrap();
    /// assert_eq!(usage.for_business("9")[0].use_case.as_deref(), Some("whatsapp"));
    /// ```
    #[must_use]
    pub fn for_business(&self, business_id: &str) -> &[UseCaseUsage] {
        self.entries
            .get(business_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// One use case's counters for one business, matched case-insensitively.
    ///
    /// ```rust
    /// use meta_whatsapp::usage::BusinessUseCaseUsage;
    ///
    /// let usage = BusinessUseCaseUsage::from_header(
    ///     r#"{"9":[{"type":"whatsapp_business_management","call_count":3}]}"#,
    /// )
    /// .unwrap();
    /// assert_eq!(
    ///     usage.use_case("9", "WHATSAPP_BUSINESS_MANAGEMENT").unwrap().call_count,
    ///     Some(3)
    /// );
    /// assert!(usage.use_case("9", "whatsapp").is_none());
    /// ```
    #[must_use]
    pub fn use_case(&self, business_id: &str, use_case: &str) -> Option<&UseCaseUsage> {
        self.for_business(business_id).iter().find(|entry| {
            entry
                .use_case
                .as_deref()
                .is_some_and(|name| name.eq_ignore_ascii_case(use_case))
        })
    }

    /// Whether any use case of `business_id` reports being throttled.
    ///
    /// ```rust
    /// use meta_whatsapp::usage::BusinessUseCaseUsage;
    ///
    /// let usage = BusinessUseCaseUsage::from_header(
    ///     r#"{"9":[{"type":"whatsapp","total_time":100}]}"#,
    /// )
    /// .unwrap();
    /// assert!(usage.is_throttled("9"));
    /// ```
    #[must_use]
    pub fn is_throttled(&self, business_id: &str) -> bool {
        self.for_business(business_id)
            .iter()
            .any(UseCaseUsage::is_throttled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_header_parses() {
        // Shape from the published OpenAPI header example
        // (docs/contracts.md, `x-business-use-case-usage`).
        let header = r#"{"102290129340398":[{"type":"whatsapp_business_management","call_count":1,"total_cputime":1,"total_time":1,"estimated_time_to_regain_access":0},{"type":"whatsapp","call_count":1,"total_cputime":1,"total_time":1,"estimated_time_to_regain_access":0}]}"#;
        let usage = BusinessUseCaseUsage::from_header(header).unwrap();
        assert_eq!(usage.for_business("102290129340398").len(), 2);
        assert!(usage.use_case("102290129340398", "whatsapp").is_some());
        assert!(!usage.is_throttled("102290129340398"));
    }

    #[test]
    fn unknown_counters_are_ignored_and_missing_ones_default() {
        let header = r#"{"1":[{"type":"whatsapp","unexpected":true}]}"#;
        let usage = BusinessUseCaseUsage::from_header(header).unwrap();
        let entry = &usage.for_business("1")[0];
        assert_eq!(entry.call_count, None);
        assert!(!entry.is_throttled());
    }

    #[test]
    fn wrong_types_are_rejected_instead_of_guessed() {
        assert!(BusinessUseCaseUsage::from_header(r#"{"1":"nope"}"#).is_none());
        assert!(BusinessUseCaseUsage::from_header(r#"{"1":[{"call_count":"many"}]}"#).is_none());
        assert!(BusinessUseCaseUsage::from_header("null").is_none());
        assert!(BusinessUseCaseUsage::from_header("{").is_none());
    }

    #[test]
    fn whitespace_around_the_header_is_tolerated() {
        let usage = BusinessUseCaseUsage::from_header("  {\"1\":[]}  ").unwrap();
        assert_eq!(usage.for_business("1").len(), 0);
    }

    #[test]
    fn serialising_round_trips() {
        let usage = BusinessUseCaseUsage::from_header(r#"{"1":[{"type":"whatsapp"}]}"#).unwrap();
        let rendered = serde_json::to_string(&usage).unwrap();
        let reparsed = BusinessUseCaseUsage::from_header(&rendered).unwrap();
        assert_eq!(usage, reparsed);
    }
}
