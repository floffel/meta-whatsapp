//! Retry budget: exponential backoff with an explicit ceiling.
//!
//! The policy is pure data plus one pure decision function, so the loop in
//! [`crate::client`] stays a thin shell and the arithmetic is testable —
//! including property tests over arbitrary delays and attempt counts.

use std::time::Duration;

/// How often, and how long, the client retries transient failures.
///
/// The default is three attempts (two retries) with 250 ms doubling to an
/// 8 s ceiling, honouring `Retry-After`. Every wait is capped by
/// [`RetryPolicy::max_delay`], so a hostile or broken proxy cannot park the
/// caller for an unbounded time.
///
/// ```
/// use std::time::Duration;
/// use meta_whatsapp::RetryPolicy;
///
/// let policy = RetryPolicy::default();
/// assert_eq!(policy.delay(1, None), Some(Duration::from_millis(250)));
/// assert_eq!(policy.delay(2, None), Some(Duration::from_millis(500)));
/// // Third attempt failed: the budget is spent.
/// assert_eq!(policy.delay(3, None), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total attempts, including the first one. `0` behaves like `1`.
    pub max_attempts: u32,
    /// Delay after the first failed attempt; doubles per further attempt.
    pub base_delay: Duration,
    /// Upper bound for a single wait, applied to backoff and `Retry-After`.
    pub max_delay: Duration,
    /// Use the server's `Retry-After` when present instead of the backoff.
    pub use_retry_after: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay: Duration::from_millis(250),
            max_delay: Duration::from_secs(8),
            use_retry_after: true,
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries (`max_attempts = 1`).
    ///
    /// ```
    /// use meta_whatsapp::RetryPolicy;
    ///
    /// assert_eq!(RetryPolicy::none().delay(1, None), None);
    /// ```
    #[must_use]
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            ..Self::default()
        }
    }

    /// A policy with `max_attempts` total attempts and the default delays.
    ///
    /// ```
    /// use meta_whatsapp::RetryPolicy;
    /// assert_eq!(RetryPolicy::new(5).max_attempts, 5);
    /// ```
    #[must_use]
    pub fn new(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            ..Self::default()
        }
    }

    /// Overrides the first backoff step.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::RetryPolicy;
    ///
    /// let policy = RetryPolicy::default().with_base_delay(Duration::from_secs(1));
    /// assert_eq!(policy.delay(1, None), Some(Duration::from_secs(1)));
    /// ```
    #[must_use]
    pub fn with_base_delay(mut self, delay: Duration) -> Self {
        self.base_delay = delay;
        self
    }

    /// Overrides the ceiling for a single wait.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::RetryPolicy;
    ///
    /// let policy = RetryPolicy::new(10).with_max_delay(Duration::from_secs(2));
    /// assert_eq!(policy.delay(9, None), Some(Duration::from_secs(2)));
    /// ```
    #[must_use]
    pub fn with_max_delay(mut self, delay: Duration) -> Self {
        self.max_delay = delay;
        self
    }

    /// Ignores `Retry-After` and always uses exponential backoff.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::RetryPolicy;
    ///
    /// let policy = RetryPolicy::default().without_retry_after();
    /// let hint = Duration::from_secs(30);
    /// assert_eq!(policy.delay(1, Some(hint)), Some(Duration::from_millis(250)));
    /// ```
    #[must_use]
    pub fn without_retry_after(mut self) -> Self {
        self.use_retry_after = false;
        self
    }

    /// The wait after `attempts_failed` failed attempts, or `None` when the
    /// budget is spent and the caller must surface the error.
    ///
    /// `attempts_failed` counts completed attempts: `1` means the first try
    /// just failed.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::RetryPolicy;
    ///
    /// let policy = RetryPolicy::default().with_max_delay(Duration::from_secs(3));
    /// // Retry-After is honoured but clamped to the ceiling.
    /// assert_eq!(
    ///     policy.delay(1, Some(Duration::from_secs(30))),
    ///     Some(Duration::from_secs(3))
    /// );
    /// ```
    #[must_use]
    pub fn delay(&self, attempts_failed: u32, retry_after: Option<Duration>) -> Option<Duration> {
        let budget = self.max_attempts.max(1);
        if attempts_failed >= budget {
            return None;
        }

        if self.use_retry_after {
            if let Some(hint) = retry_after {
                return Some(hint.min(self.max_delay));
            }
        }

        // 2^30 attempts is already far past any sane budget; shifting by more
        // would overflow, and `saturating_mul` keeps the value finite anyway.
        let exponent = attempts_failed.saturating_sub(1).min(30);
        let factor = 1_u32 << exponent;
        let backoff = self.base_delay.saturating_mul(factor);
        Some(backoff.min(self.max_delay))
    }
}
