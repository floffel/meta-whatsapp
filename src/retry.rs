//! Retry budget and retry classification.
//!
//! Two pure pieces, no timers: [`RetryPolicy::delay`] decides *how long* to
//! wait, [`RetryPolicy::decide`] decides *whether waiting is safe at all*. The
//! loop in [`crate::client`] only sleeps and re-sends, so the interesting
//! arithmetic and the delivery contract are testable — including property
//! tests that pin "no non-idempotent request is retried after a 5xx".
//!
//! # Why the curve is local
//!
//! The audit that produced this module's issues asked for the backoff
//! mechanism to be delegated (issue #5). The three candidate crates were
//! evaluated and each one loses something this crate needs:
//!
//! * `backoff` 0.4 — the crate the audit named — has had no release since
//!   2021-12, and `ExponentialBackoff::next_backoff` takes `&mut self`, so a
//!   pure `&self` decision function cannot hold one.
//! * `backon` 1.6 computes the curve in `f32` seconds: a 100 ms base yields
//!   `100 ms, 200.000_003 ms, 400.000_006 ms`, and asking for entry `n` walks
//!   the iterator `n` times, which a `u32::MAX` budget makes unbounded work.
//! * `retry-policies` 0.5 answers with a `SystemTime`, has no `Retry-After`
//!   input at all, and its exponential is the only thing it offers.
//!
//! What remains below is integer arithmetic on `Duration` (exact by
//! construction), the `Retry-After` precedence, and the idempotency gate — the
//! three things no retry crate models together. The parts that *are* generic —
//! the classifier and the budget — are pinned by unit and property tests
//! instead of by a dependency.

use std::time::Duration;

use crate::error::Error;
use crate::transport::Method;

/// Whether repeating a request could duplicate its effect.
///
/// ```
/// use meta_whatsapp::transport::Method;
/// use meta_whatsapp::retry::Repeatability;
///
/// assert_eq!(Repeatability::of_method(Method::Get), Repeatability::Idempotent);
/// assert_eq!(Repeatability::of_method(Method::Delete), Repeatability::Idempotent);
/// assert_eq!(Repeatability::of_method(Method::Post), Repeatability::NotIdempotent);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Repeatability {
    /// Repeating is harmless: `GET` and `DELETE` are idempotent by definition
    /// (deleting twice leaves the same state as deleting once).
    Idempotent,
    /// Repeating may duplicate an effect: sending a message, uploading media.
    /// WhatsApp has no idempotency key, so a repeat is visible to the
    /// recipient.
    NotIdempotent,
}

impl Repeatability {
    /// Classifies an HTTP verb.
    ///
    /// ```
    /// use meta_whatsapp::retry::Repeatability;
    /// use meta_whatsapp::transport::Method;
    ///
    /// assert!(Repeatability::of_method(Method::Get).is_idempotent());
    /// assert!(!Repeatability::of_method(Method::Post).is_idempotent());
    /// ```
    #[must_use]
    pub fn of_method(method: Method) -> Self {
        match method {
            Method::Get | Method::Delete => Self::Idempotent,
            Method::Post => Self::NotIdempotent,
        }
    }

    /// `true` for [`Repeatability::Idempotent`].
    ///
    /// ```
    /// use meta_whatsapp::retry::Repeatability;
    /// assert!(Repeatability::Idempotent.is_idempotent());
    /// assert!(!Repeatability::NotIdempotent.is_idempotent());
    /// ```
    #[must_use]
    pub fn is_idempotent(self) -> bool {
        matches!(self, Self::Idempotent)
    }
}

/// One failed attempt, reduced to the facts the retry decision needs.
///
/// ```
/// use meta_whatsapp::retry::Attempt;
/// use meta_whatsapp::Error;
///
/// assert_eq!(Attempt::classify(&Error::transport("dns")), Attempt::NotSent);
/// assert_eq!(
///     Attempt::classify(&Error::transport_after_write("timeout")),
///     Attempt::NoAnswer
/// );
/// assert_eq!(Attempt::classify(&Error::validation("to", "empty")), Attempt::Terminal);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Attempt {
    /// 429: the server refused the request before processing it. Repeating is
    /// always safe.
    RateLimited {
        /// `Retry-After`, when the server sent one.
        retry_after: Option<Duration>,
    },
    /// The connection failed before the request was written (DNS, connect,
    /// TLS). Nothing was processed.
    NotSent,
    /// 5xx: the server may have processed the request before failing.
    ServerError {
        /// `Retry-After`, when the server sent one.
        retry_after: Option<Duration>,
    },
    /// The request was written but no response arrived (timeout, reset,
    /// truncated body).
    NoAnswer,
    /// Terminal: 4xx other than 429, validation, signature or decoding
    /// failures. Repeating cannot help.
    Terminal,
}

impl Attempt {
    /// The server's `Retry-After` hint, when it sent one.
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::retry::Attempt;
    ///
    /// let attempt = Attempt::RateLimited { retry_after: Some(Duration::from_secs(2)) };
    /// assert_eq!(attempt.retry_after(), Some(Duration::from_secs(2)));
    /// assert_eq!(Attempt::NotSent.retry_after(), None);
    /// ```
    #[must_use]
    pub fn retry_after(self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } | Self::ServerError { retry_after } => retry_after,
            _ => None,
        }
    }

    /// Reduces an [`Error`] to the facts that drive the decision — purely,
    /// without I/O.
    ///
    /// ```
    /// use meta_whatsapp::retry::Attempt;
    /// use meta_whatsapp::Error;
    ///
    /// let rate_limited = Error::Api {
    ///     code: 4,
    ///     status: 429,
    ///     message: "slow down".into(),
    ///     retry_after: Some(std::time::Duration::from_secs(5)),
    ///     retryable: true,
    /// };
    /// assert!(matches!(
    ///     Attempt::classify(&rate_limited),
    ///     Attempt::RateLimited { retry_after: Some(_) }
    /// ));
    /// ```
    #[must_use]
    pub fn classify(error: &Error) -> Self {
        match error {
            Error::Api {
                status,
                retry_after,
                ..
            } => match *status {
                429 => Self::RateLimited {
                    retry_after: *retry_after,
                },
                500..=599 => Self::ServerError {
                    retry_after: *retry_after,
                },
                _ => Self::Terminal,
            },
            // A 5xx that was already wrapped is still a server error; the
            // wrapper exists so the caller sees the delivery risk.
            Error::Ambiguous { cause, .. } => Self::classify(cause),
            Error::Transport { phase, .. } => {
                if phase.is_after_write() {
                    Self::NoAnswer
                } else if phase.is_before_write() {
                    Self::NotSent
                } else {
                    Self::Terminal
                }
            }
            // Everything else — a 4xx, a validation failure, an invalid
            // signature, and `Error::ResponseTooLarge` (a repeat would return
            // the same oversized body) — is terminal. The typed error is more
            // useful to the caller than a retry loop that always fails.
            _ => Self::Terminal,
        }
    }
}

/// What to do with a failed attempt.
///
/// ```
/// use std::time::Duration;
/// use meta_whatsapp::RetryPolicy;
/// use meta_whatsapp::retry::{Action, Attempt, Repeatability};
///
/// let policy = RetryPolicy::default();
/// let action = policy.decide(
///     Attempt::RateLimited { retry_after: None },
///     Repeatability::NotIdempotent,
///     1,
/// );
/// assert!(matches!(action, Action::Retry(_)));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    /// Wait this long, then send again.
    Retry(Duration),
    /// Stop and return the error unchanged — the caller may retry later.
    Surface,
    /// Stop and return [`Error::Ambiguous`]: the message may have been
    /// delivered, and the caller has to decide (dedupe, alert, drop).
    Ambiguous,
}

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
    /// The backoff is exact: `base_delay` shifted left by `attempts_failed - 1`,
    /// capped at [`RetryPolicy::max_delay`]. There is no float math in the path,
    /// so a 100 ms base yields exactly 100, 200, 400 ms — not 200.000_003.
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

    /// Whether to retry a failed attempt, and how long to wait — the whole
    /// delivery contract in one pure function.
    ///
    /// The rules, and why:
    ///
    /// * **429** — the server refused the request *before* processing it, so a
    ///   repeat is safe even for a send.
    /// * **Connection failure before the write** — nothing was processed, so a
    ///   repeat is safe.
    /// * **5xx, or no answer after the write, on an idempotent request** —
    ///   repeating cannot change the outcome (reads, deletes).
    /// * **5xx, or no answer after the write, on a non-idempotent request** —
    ///   the message may already be delivered; a repeat could duplicate it, so
    ///   the answer is [`Action::Ambiguous`], never a retry.
    ///
    /// A spent budget always stops the loop; on a non-idempotent request with
    /// an unknown outcome it still reports [`Action::Ambiguous`].
    ///
    /// ```
    /// use meta_whatsapp::RetryPolicy;
    /// use meta_whatsapp::retry::{Action, Attempt, Repeatability};
    ///
    /// let policy = RetryPolicy::default();
    ///
    /// // A 5xx on a send is never retried: it might be delivered already.
    /// assert_eq!(
    ///     policy.decide(Attempt::ServerError { retry_after: None }, Repeatability::NotIdempotent, 1),
    ///     Action::Ambiguous
    /// );
    /// // The same 5xx on a list call is retried with backoff.
    /// assert!(matches!(
    ///     policy.decide(Attempt::ServerError { retry_after: None }, Repeatability::Idempotent, 1),
    ///     Action::Retry(_)
    /// ));
    /// // A 429 is safe to repeat either way.
    /// assert!(matches!(
    ///     policy.decide(Attempt::RateLimited { retry_after: None }, Repeatability::NotIdempotent, 1),
    ///     Action::Retry(_)
    /// ));
    /// ```
    #[must_use]
    pub fn decide(
        &self,
        attempt: Attempt,
        repeatability: Repeatability,
        attempts_failed: u32,
    ) -> Action {
        match attempt {
            Attempt::Terminal => Action::Surface,
            Attempt::RateLimited { .. } | Attempt::NotSent => {
                self.retry_or_surface(attempts_failed, attempt.retry_after())
            }
            Attempt::ServerError { .. } | Attempt::NoAnswer => {
                if repeatability.is_idempotent() {
                    self.retry_or_surface(attempts_failed, attempt.retry_after())
                } else {
                    // The request may have taken effect. Never repeat it
                    // automatically, whatever the budget still allows.
                    Action::Ambiguous
                }
            }
        }
    }

    fn retry_or_surface(&self, attempts_failed: u32, retry_after: Option<Duration>) -> Action {
        match self.delay(attempts_failed, retry_after) {
            Some(delay) => Action::Retry(delay),
            None => Action::Surface,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_5xx_on_a_send_is_ambiguous_even_with_budget_left() {
        let policy = RetryPolicy::new(10);
        assert_eq!(
            policy.decide(
                Attempt::ServerError { retry_after: None },
                Repeatability::NotIdempotent,
                1
            ),
            Action::Ambiguous
        );
    }

    #[test]
    fn a_timeout_after_the_write_is_ambiguous_for_a_send() {
        let policy = RetryPolicy::new(10);
        assert_eq!(
            policy.decide(Attempt::NoAnswer, Repeatability::NotIdempotent, 1),
            Action::Ambiguous
        );
        assert!(matches!(
            policy.decide(Attempt::NoAnswer, Repeatability::Idempotent, 1),
            Action::Retry(_)
        ));
    }

    #[test]
    fn a_failure_before_the_write_is_retried_until_the_budget_is_spent() {
        let policy = RetryPolicy::new(2);
        assert!(matches!(
            policy.decide(Attempt::NotSent, Repeatability::NotIdempotent, 1),
            Action::Retry(_)
        ));
        assert_eq!(
            policy.decide(Attempt::NotSent, Repeatability::NotIdempotent, 2),
            Action::Surface
        );
    }

    #[test]
    fn terminal_attempts_are_never_retried() {
        let policy = RetryPolicy::new(100);
        assert_eq!(
            policy.decide(Attempt::Terminal, Repeatability::Idempotent, 1),
            Action::Surface
        );
    }

    #[test]
    fn an_oversized_response_is_terminal_even_for_a_read() {
        let error = Error::ResponseTooLarge {
            limit: 1024,
            status: 200,
        };
        assert_eq!(Attempt::classify(&error), Attempt::Terminal);
        assert_eq!(
            RetryPolicy::new(3).decide(Attempt::classify(&error), Repeatability::Idempotent, 1),
            Action::Surface
        );
    }

    #[test]
    fn the_curve_doubles_exactly_from_an_arbitrary_base_and_saturates() {
        let policy = RetryPolicy::new(10)
            .with_base_delay(Duration::from_millis(100))
            .with_max_delay(Duration::from_millis(500));
        let waits: Vec<_> = (1..=5).map(|failed| policy.delay(failed, None)).collect();
        assert_eq!(
            waits,
            vec![
                Some(Duration::from_millis(100)),
                Some(Duration::from_millis(200)),
                Some(Duration::from_millis(400)),
                Some(Duration::from_millis(500)),
                Some(Duration::from_millis(500)),
            ]
        );
        // Exhaustion is the budget, not the curve: `max_attempts` attempts means
        // `max_attempts - 1` retries.
        assert_eq!(policy.delay(10, None), None);
        assert_eq!(policy.delay(11, None), None);
    }

    #[test]
    fn a_ceiling_below_the_base_delay_still_bounds_the_first_wait() {
        let policy = RetryPolicy::new(4)
            .with_base_delay(Duration::from_secs(5))
            .with_max_delay(Duration::from_millis(10));
        for failed in 1..4 {
            assert_eq!(policy.delay(failed, None), Some(Duration::from_millis(10)));
        }
    }

    #[test]
    fn a_zero_base_delay_yields_zero_waits() {
        let policy = RetryPolicy::new(3).with_base_delay(Duration::ZERO);
        assert_eq!(policy.delay(1, None), Some(Duration::ZERO));
        assert_eq!(policy.delay(2, None), Some(Duration::ZERO));
    }

    #[test]
    fn an_enormous_budget_stays_exact_and_cheap() {
        // The exponent is clamped, so a 4-billion-attempt budget costs one
        // shift: the wait saturates at the ceiling instead of overflowing.
        let policy = RetryPolicy::new(u32::MAX)
            .with_base_delay(Duration::from_millis(1))
            .with_max_delay(Duration::from_secs(1));
        assert_eq!(
            policy.delay(u32::MAX - 1, None),
            Some(Duration::from_secs(1))
        );
    }

    #[test]
    fn a_5xx_keeps_its_retry_after_only_for_idempotent_requests() {
        let policy = RetryPolicy::new(5);
        let attempt = Attempt::ServerError {
            retry_after: Some(Duration::from_secs(2)),
        };
        assert_eq!(
            policy.decide(attempt, Repeatability::Idempotent, 1),
            Action::Retry(Duration::from_secs(2))
        );
        assert_eq!(
            policy.decide(attempt, Repeatability::NotIdempotent, 1),
            Action::Ambiguous
        );
    }
}
