#![no_main]
//! The account-administration parsers: business profile, phone number,
//! blocklist, block result and webhook subscriptions.
//!
//! Each one takes an arbitrary Graph API body. A shape change must surface as a
//! typed error or a permissive default, never as a panic or a `Display` that
//! itself panics.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::business::{BlockOutcome, BlockedUsersPage, BusinessProfile, PhoneNumberInfo};
use meta_whatsapp::subscription::WebhookSubscriptions;

fuzz_target!(|data: &[u8]| {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };

    // The profile envelope: three documented shapes accepted, everything else a
    // typed error whose `Display` must not panic on attacker-controlled text.
    if let Err(error) = BusinessProfile::parse(&value) {
        assert!(!error.to_string().is_empty());
    }

    if let Err(error) = PhoneNumberInfo::parse(&value) {
        assert!(!error.to_string().is_empty());
    }

    // The blocklist page is a plain `Deserialize` type: permissive defaults are
    // what keep an unexpected shape from failing the call.
    if let Ok(page) = serde_json::from_slice::<BlockedUsersPage>(data) {
        for user in &page.data {
            let _ = user.messaging_product.as_deref();
            let _ = user.wa_id.as_deref();
        }
        assert_eq!(
            page.has_next_page(),
            page.paging
                .as_ref()
                .and_then(|paging| paging.next.as_ref())
                .is_some(),
            "has_next_page() disagrees with the paging block"
        );
    }

    // A block result without the documented object is accepted with empty
    // lists, so a shape change cannot turn a successful block into an error.
    match BlockOutcome::from_value(&value) {
        Ok(outcome) => {
            let _ = outcome.added.len();
            let _ = outcome.removed.len();
            let _ = outcome.messaging_product.as_deref();
        }
        Err(error) => assert!(!error.to_string().is_empty()),
    }

    let _ = WebhookSubscriptions::parse(&value);
});
