#![no_main]
//! The header parsers: `Retry-After`, the usage-budget header, and the two
//! permissive string enums (`quality_rating`, `messaging_limit_tier`).
//!
//! These run on values a provider or a proxy sends; none of them may panic, and
//! the string enums must survive a round trip through their wire spelling.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::business::{MessagingLimitTier, QualityRating};
use meta_whatsapp::media::parse_retry_after;
use meta_whatsapp::usage::BusinessUseCaseUsage;

fuzz_target!(|data: &[u8]| {
    let raw = String::from_utf8_lossy(data);

    // Retry-After: integer seconds only. Anything else is `None`, never an
    // invented deadline.
    match parse_retry_after(&raw) {
        Some(delay) => assert_eq!(
            Ok(delay.as_secs()),
            raw.trim().parse::<u64>(),
            "Retry-After {raw:?}"
        ),
        None => assert!(
            raw.trim().parse::<u64>().is_err(),
            "a numeric Retry-After was rejected: {raw:?}"
        ),
    }

    // The usage budget comes as a JSON object in a header; anything that is not
    // an object is telemetry loss, not an error.
    if !raw.trim().starts_with('{') {
        assert!(
            BusinessUseCaseUsage::from_header(&raw).is_none(),
            "a non-object header was accepted as usage data: {raw:?}"
        );
    }
    if let Some(usage) = BusinessUseCaseUsage::from_header(&raw) {
        for business in usage.businesses() {
            for entry in usage.for_business(business) {
                let _ = entry.is_throttled();
            }
            let _ = usage.is_throttled(business);
            let _ = usage.use_case(business, "whatsapp");
        }
    }

    // Both enums keep unknown values and report them back verbatim.
    let rating = QualityRating::parse(&raw);
    assert_eq!(
        QualityRating::parse(rating.as_str()),
        rating,
        "quality rating {raw:?} does not survive the round trip"
    );
    assert_eq!(
        rating.permits_sending(),
        rating != QualityRating::Red,
        "only RED may block sending"
    );

    let tier = MessagingLimitTier::parse(&raw);
    assert_eq!(
        MessagingLimitTier::parse(tier.as_str()),
        tier,
        "messaging tier {raw:?} does not survive the round trip"
    );
});
