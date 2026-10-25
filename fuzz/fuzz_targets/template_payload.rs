#![no_main]
//! Template list parsing and the template query string.
//!
//! The list comes back from the Graph API as JSON; the query string goes out
//! into a URL. Neither may panic, and a caller-supplied value must never escape
//! percent-encoding into the query.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::{TemplateList, TemplateQuery};

fuzz_target!(|data: &[u8]| {
    if let Ok(page) = serde_json::from_slice::<TemplateList>(data) {
        for template in &page.data {
            let _ = template.name.len();
            let _ = template.status;
        }
        if page.has_next_page() {
            assert!(
                page.paging
                    .as_ref()
                    .and_then(|paging| paging.next.as_ref())
                    .is_some(),
                "has_next_page() disagrees with the paging block"
            );
        }
    }

    // Every byte of the input becomes a query value; none may leave a character
    // that changes the meaning of the query.
    let text = String::from_utf8_lossy(data);
    let query = TemplateQuery::new("1")
        .name(text.as_ref())
        .language(text.as_ref())
        .status(text.as_ref())
        .after(text.as_ref())
        .fields([text.as_ref()])
        .query_string();

    for forbidden in [' ', '#', '\n', '\r', '\t'] {
        assert!(
            !query.contains(forbidden),
            "query {query:?} contains the unescaped {forbidden:?}"
        );
    }
    assert!(
        !query.starts_with('&') && !query.ends_with('&'),
        "{query:?}"
    );
    assert!(!query.contains("&&"), "{query:?}");
});
