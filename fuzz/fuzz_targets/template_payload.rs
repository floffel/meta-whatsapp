#![no_main]
//! Template list parsing, the acknowledgement parsers and the template query
//! string.
//!
//! The list and the acks come back from the Graph API as JSON; the query string
//! goes out into a URL. Neither may panic, and a caller-supplied value must
//! never escape percent-encoding into the query.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::{
    TemplateAck, TemplateCategory, TemplateDefinition, TemplateDeleteAck, TemplateList,
    TemplateQuery,
};

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

    // The acknowledgements see whatever Meta sends. A body that is not an object
    // is an error, never a panic; `is_success()` is only false for an explicit
    // `success: false`.
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(data) {
        let create = TemplateAck::from_value(&value);
        assert_eq!(
            create.is_ok(),
            value.is_object(),
            "only an object is an acknowledgement"
        );
        if let Ok(ack) = create {
            assert_eq!(ack.is_success(), ack.success != Some(false));
        }
        let delete = TemplateDeleteAck::from_value(&value);
        assert_eq!(delete.is_ok(), value.is_object());
        if let Ok(ack) = delete {
            assert_eq!(ack.is_success(), ack.success != Some(false));
        }
    }

    // A definition built from arbitrary bytes validates into a payload that
    // keeps the name, language and category it was built with, or it refuses —
    // it never panics and never renders an unsupported component type.
    let definition = TemplateDefinition::new(text.as_ref(), "de", TemplateCategory::Utility)
        .body_text(text.as_ref());
    if let Ok(payload) = definition.render() {
        assert_eq!(payload["language"], "de");
        assert_eq!(payload["category"], "UTILITY");
        assert!(payload["components"].is_array());
        for component in payload["components"].as_array().into_iter().flatten() {
            assert!(
                matches!(
                    component["type"].as_str(),
                    Some("HEADER" | "BODY" | "FOOTER" | "BUTTONS")
                ),
                "unexpected component {component}"
            );
        }
    }
});
