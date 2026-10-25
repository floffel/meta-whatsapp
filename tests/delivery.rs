//! Delivery semantics (#4): a non-idempotent request is never re-sent after a
//! failure that may already have been processed.
//!
//! The contract under test:
//!
//! * 5xx on a send → exactly one HTTP call, `Error::Ambiguous`,
//!   `is_retryable() == false`, `is_ambiguous() == true`.
//! * 429 → two calls, the second one succeeds.
//! * Connection failure before the write → retried.
//! * Lost response after the write → one call, `Error::Ambiguous`.
//! * 5xx / lost response on a read (GET) → retried.
//! * 4xx (other than 429) → terminal, one call.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{accepted, client, instant_retry, json_response, response, MockTransport, Reply};
use meta_whatsapp::business::BusinessProfileUpdate;
use meta_whatsapp::retry::{Action, Attempt, Repeatability};
use meta_whatsapp::{Error, RetryPolicy, TemplateQuery};

const SERVER_ERROR: &[u8] = b"{\"error\":{\"code\":2,\"message\":\"internal\"}}";

#[tokio::test]
async fn a_5xx_on_a_send_is_one_call_and_ambiguous() {
    let transport = MockTransport::new();
    // Queue several so a wrong implementation would happily consume them.
    for _ in 0..4 {
        transport.push_response(response(500, &[], SERVER_ERROR));
    }
    let client = client(transport.clone()).with_retry(instant_retry(5));

    let err = client
        .send_text("4915112345678", "Hallo!")
        .await
        .unwrap_err();

    assert_eq!(transport.call_count(), 1, "never re-sent");
    match &err {
        Error::Ambiguous {
            operation,
            status,
            cause,
        } => {
            assert!(operation.contains("/messages"), "operation: {operation}");
            assert_eq!(*status, Some(500));
            assert!(
                matches!(**cause, Error::Api { code: 2, .. }),
                "the Graph envelope is preserved: {cause:?}"
            );
        }
        other => panic!("expected Ambiguous, got {other:?}"),
    }
    assert!(!err.is_retryable());
    assert!(err.is_ambiguous());
    assert!(err.to_string().contains("ambiguous"));
}

#[tokio::test]
async fn a_503_on_every_write_endpoint_stays_single_shot() {
    // Same rule for every non-idempotent call, not just `send`.
    let upload_transport = MockTransport::new();
    for _ in 0..3 {
        upload_transport.push_response(response(503, &[], SERVER_ERROR));
    }
    let upload_client = client(upload_transport.clone()).with_retry(instant_retry(3));
    let upload = meta_whatsapp::MediaUpload::new("a.txt", "text/plain", b"hi".to_vec()).unwrap();
    let err = upload_client.upload_media(&upload).await.unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(upload_transport.call_count(), 1);

    let block_transport = MockTransport::new();
    for _ in 0..3 {
        block_transport.push_response(response(503, &[], SERVER_ERROR));
    }
    let block_client = client(block_transport.clone()).with_retry(instant_retry(3));
    let err = block_client
        .block_users(&["+16505551234"])
        .await
        .unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(block_transport.call_count(), 1);

    let profile_transport = MockTransport::new();
    for _ in 0..3 {
        profile_transport.push_response(response(500, &[], SERVER_ERROR));
    }
    let profile_client = client(profile_transport.clone()).with_retry(instant_retry(3));
    let update = BusinessProfileUpdate::new().about("Kaffee");
    let err = profile_client
        .update_business_profile(&update)
        .await
        .unwrap_err();
    assert!(err.is_ambiguous(), "{err:?}");
    assert_eq!(profile_transport.call_count(), 1);
}

#[tokio::test]
async fn a_429_on_a_send_is_retried_and_succeeds() {
    let transport = MockTransport::new();
    transport.push_response(response(
        429,
        &[("Retry-After", "0")],
        b"{\"error\":{\"code\":4,\"message\":\"rate limited\"}}",
    ));
    transport.push_response(accepted("wamid.after-retry"));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let value = client.send_text("4915112345678", "Hallo!").await.unwrap();

    assert_eq!(value["messages"][0]["id"], "wamid.after-retry");
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn the_retry_after_header_sets_the_wait() {
    let policy = RetryPolicy::new(3);
    assert_eq!(
        policy.decide(
            Attempt::RateLimited {
                retry_after: Some(Duration::from_secs(2))
            },
            Repeatability::NotIdempotent,
            1
        ),
        Action::Retry(Duration::from_secs(2))
    );
    // Clamped to the ceiling, so a hostile hint cannot park the caller.
    assert_eq!(
        policy.decide(
            Attempt::RateLimited {
                retry_after: Some(Duration::from_secs(3600))
            },
            Repeatability::NotIdempotent,
            1
        ),
        Action::Retry(policy.max_delay)
    );
}

#[tokio::test]
async fn a_lost_response_after_the_write_is_ambiguous_not_retried() {
    let transport = MockTransport::new();
    transport.push(Reply::after_write("operation timed out"));
    transport.push_response(accepted("never used"));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.send_text("1", "hi").await.unwrap_err();

    assert_eq!(transport.call_count(), 1);
    assert!(err.is_ambiguous());
    assert!(!err.is_retryable());
    assert_eq!(err.status(), None, "no response ever arrived");
    assert_eq!(
        err.transport_phase(),
        None,
        "the wrapper is what the caller sees"
    );
}

#[tokio::test]
async fn reads_and_deletes_stay_retryable_on_5xx() {
    let transport = MockTransport::new();
    transport.push_response(response(500, &[], SERVER_ERROR));
    transport.push_response(json_response(&serde_json::json!({ "data": [] })));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let page = client
        .list_templates(&TemplateQuery::new("102290129340398"))
        .await
        .unwrap();
    assert_eq!(page.data.len(), 0);
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn a_read_that_loses_the_connection_after_the_write_is_retried() {
    let transport = MockTransport::new();
    transport.push(Reply::after_write("timed out"));
    transport.push_response(json_response(&serde_json::json!({ "data": [] })));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    client
        .list_templates(&TemplateQuery::new("102290129340398"))
        .await
        .unwrap();
    assert_eq!(transport.call_count(), 2, "idempotent, so a retry is safe");
}

#[tokio::test]
async fn mark_read_is_treated_as_idempotent() {
    let transport = MockTransport::new();
    transport.push_response(response(500, &[], SERVER_ERROR));
    transport.push_response(json_response(&serde_json::json!({ "success": true })));
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let value = client.mark_read("wamid.1").await.unwrap();
    assert_eq!(value["success"], true);
    assert_eq!(
        transport.call_count(),
        2,
        "marking read twice leaves the same state"
    );
}

#[tokio::test]
async fn a_4xx_on_a_send_is_terminal_and_untouched() {
    let transport = MockTransport::new();
    transport.push_response(response(
        400,
        &[],
        b"{\"error\":{\"code\":131009,\"message\":\"bad parameter\"}}",
    ));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.send_text("1", "hi").await.unwrap_err();
    assert!(matches!(err, Error::Api { status: 400, .. }));
    assert!(!err.is_ambiguous());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn a_5xx_after_the_retry_budget_is_spent_is_still_ambiguous() {
    // Two failures, budget of two: the second result must still be Ambiguous,
    // not a plain Api error the caller might retry.
    let transport = MockTransport::new();
    for _ in 0..5 {
        transport.push_response(response(502, &[], SERVER_ERROR));
    }
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let err = client.send_text("1", "hi").await.unwrap_err();
    assert!(err.is_ambiguous());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn concurrent_sends_each_get_their_own_verdict() {
    let transport = MockTransport::new();
    // Every reply is a 500: 32 tasks must produce 32 calls and 32 ambiguous
    // errors, with no retries and no cross-talk.
    for _ in 0..64 {
        transport.push_response(response(500, &[], SERVER_ERROR));
    }
    let client = Arc::new(client(transport.clone()).with_retry(instant_retry(3)));

    let mut handles = Vec::new();
    for index in 0..32 {
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            let err = client
                .send_text(&format!("4915{index:07}"), "hi")
                .await
                .unwrap_err();
            assert!(err.is_ambiguous());
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(transport.call_count(), 32);
}

#[test]
fn the_documented_rule_table_holds() {
    let policy = RetryPolicy::new(3);
    let idempotent = Repeatability::Idempotent;
    let send = Repeatability::NotIdempotent;

    // 429 is safe for both.
    for repeatability in [idempotent, send] {
        assert!(matches!(
            policy.decide(Attempt::RateLimited { retry_after: None }, repeatability, 1),
            Action::Retry(_)
        ));
    }
    // Before the write is safe for both.
    for repeatability in [idempotent, send] {
        assert!(matches!(
            policy.decide(Attempt::NotSent, repeatability, 1),
            Action::Retry(_)
        ));
    }
    // 5xx and lost answers are only retried when idempotent.
    for attempt in [
        Attempt::ServerError { retry_after: None },
        Attempt::NoAnswer,
    ] {
        assert!(matches!(
            policy.decide(attempt, idempotent, 1),
            Action::Retry(_)
        ));
        assert_eq!(policy.decide(attempt, send, 1), Action::Ambiguous);
    }
}
