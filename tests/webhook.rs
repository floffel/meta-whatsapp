//! Verification of the `X-Hub-Signature-256` header and the GET handshake.

use meta_whatsapp::{Error, WebhookVerifier};

const BODY: &[u8] = br#"{"object":"whatsapp_business_account","entry":[]}"#;

#[test]
fn sign_then_verify_round_trips() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    let header = v.sign(BODY);
    assert!(header.starts_with("sha256="));
    assert_eq!(header.len(), "sha256=".len() + 64);
    assert!(v.verify(BODY, &header).is_ok());
}

#[test]
fn wrong_secret_is_rejected() {
    let signer = WebhookVerifier::new(b"secret".to_vec());
    let verifier = WebhookVerifier::new(b"other-secret".to_vec());
    let header = signer.sign(BODY);
    assert!(matches!(
        verifier.verify(BODY, &header),
        Err(Error::SignatureMismatch)
    ));
}

#[test]
fn tampered_body_is_rejected() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    let header = v.sign(BODY);
    let mut tampered = BODY.to_vec();
    tampered[1] = b'X';
    assert!(matches!(
        v.verify(&tampered, &header),
        Err(Error::SignatureMismatch)
    ));
}

#[test]
fn malformed_headers_are_rejected() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    for header in ["", "sha1=abc", "sha256=", "sha256=zz"] {
        assert!(
            matches!(v.verify(BODY, header), Err(Error::MalformedSignature(_))),
            "header {header:?} should be rejected"
        );
    }
}

#[test]
fn empty_secret_still_produces_a_verifiable_signature() {
    let v = WebhookVerifier::new(Vec::new());
    let header = v.sign(BODY);
    assert!(v.verify(BODY, &header).is_ok());
}

#[test]
fn verify_and_parse_returns_typed_payload() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    let header = v.sign(BODY);
    let payload = v.verify_and_parse(BODY, &header).unwrap();
    assert_eq!(payload.object, "whatsapp_business_account");
    assert!(payload.entry.is_empty());
}

#[test]
fn verify_and_parse_rejects_invalid_json() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    let body = b"not json";
    let header = v.sign(body);
    assert!(matches!(
        v.verify_and_parse(body, &header),
        Err(Error::Payload(_))
    ));
}

#[test]
fn challenge_handshake_accepts_matching_token() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    assert_eq!(
        v.verify_challenge(
            Some("subscribe"),
            Some("tok"),
            Some("1234"),
            "tok",
            "subscribe"
        ),
        Some("1234")
    );
}

#[test]
fn challenge_handshake_rejects_mismatches() {
    let v = WebhookVerifier::new(b"secret".to_vec());
    assert_eq!(
        v.verify_challenge(
            Some("subscribe"),
            Some("wrong"),
            Some("1234"),
            "tok",
            "subscribe"
        ),
        None
    );
    assert_eq!(
        v.verify_challenge(
            Some("unsubscribe"),
            Some("tok"),
            Some("1234"),
            "tok",
            "subscribe"
        ),
        None
    );
    assert_eq!(
        v.verify_challenge(None, Some("tok"), Some("1234"), "tok", "subscribe"),
        None
    );
}

#[test]
fn debug_impl_does_not_leak_the_secret() {
    let v = WebhookVerifier::new(b"super-secret-value".to_vec());
    let rendered = format!("{v:?}");
    assert!(!rendered.contains("super-secret-value"));
}
