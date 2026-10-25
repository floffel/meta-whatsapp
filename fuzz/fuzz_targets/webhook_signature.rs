#![no_main]
//! Signature verification must be exhaustive: no input is accepted without the
//! matching HMAC, and no header shape panics the verifier.
//!
//! The input is split at the first NUL into a body and a header candidate, so
//! the fuzzer can mutate both sides independently.

use libfuzzer_sys::fuzz_target;

use meta_whatsapp::WebhookVerifier;

const SECRET: &str = "fuzz-secret";

fuzz_target!(|data: &[u8]| {
    let split = data
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(data.len());
    let (body, rest) = data.split_at(split);
    let header_bytes = rest.get(1..).unwrap_or_default();
    let header = String::from_utf8_lossy(header_bytes);

    let Ok(verifier) = WebhookVerifier::new(SECRET) else {
        return;
    };

    // The genuine signature always verifies.
    let valid = verifier.sign(body);
    assert!(
        verifier.verify(body, &valid).is_ok(),
        "the crate rejected its own signature"
    );

    // Anything else verifies only if it is byte-identical to the genuine one.
    let accepted = verifier.verify(body, &header).is_ok();
    assert_eq!(
        accepted,
        header == valid,
        "verify() accepted a header that is not the genuine signature"
    );

    // `verify_and_parse` must not turn a signature failure into a parse attempt
    // that panics, and a genuine signature must survive to the parser.
    if accepted {
        let _ = verifier.verify_and_parse(body, &header);
        let _ = verifier.verify_fresh(body, &header, 1_700_000_000);
    } else {
        assert!(verifier.verify_and_parse(body, &header).is_err());
    }
});
