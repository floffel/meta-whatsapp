//! Client behaviour: URLs, error mapping, retries, timeouts, concurrency.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{
    accepted, client, instant_retry, json_response, response, Call, MockTransport, Reply,
};
use meta_whatsapp::{Error, RetryPolicy, WhatsAppClient};
use serde_json::json;

fn failure_after(calls: usize) -> MockTransport {
    let transport = MockTransport::new();
    for _ in 0..calls {
        transport.push_response(response(
            500,
            &[],
            b"{\"error\":{\"code\":2,\"message\":\"boom\"}}",
        ));
    }
    transport.push_response(accepted("wamid.after-retry"));
    transport
}

#[tokio::test]
async fn api_version_and_base_url_are_configurable() {
    let transport = MockTransport::new();
    let host = ["https:/", "/api.example.com/"].join("");
    let client = client(transport)
        .with_api_version("v20.0")
        .with_base_url(host.clone());
    assert_eq!(
        client.messages_url(),
        format!("{host}v20.0/106540352242922/messages")
    );
}

#[tokio::test]
async fn mark_read_sends_a_read_status() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({ "success": true })));
    let client = client(transport.clone());

    client.mark_read("wamid.abc123").await.unwrap();

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    match &call.body {
        common::Body::Json(body) => {
            assert_eq!(body["status"], "read");
            assert_eq!(body["message_id"], "wamid.abc123");
            assert_eq!(body["messaging_product"], "whatsapp");
        }
        other => panic!("expected JSON, got {other:?}"),
    }
}

#[tokio::test]
async fn mark_read_rejects_an_empty_message_id() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let err = client.mark_read("").await.unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "message_id",
            ..
        }
    ));
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn retry_after_is_honoured_on_429() {
    let transport = MockTransport::new();
    transport.push_response(response(
        429,
        &[("Retry-After", "1")],
        b"{\"error\":{\"code\":4,\"message\":\"rate limited\"}}",
    ));
    transport.push_response(accepted("wamid.1"));

    let client = client(transport.clone()).with_retry(
        RetryPolicy::new(2)
            .with_base_delay(Duration::ZERO)
            .with_max_delay(Duration::from_millis(1)),
    );

    let value = client.send_text("1", "hi").await.unwrap();
    assert_eq!(value["messages"][0]["id"], "wamid.1");
    assert_eq!(transport.call_count(), 2);
}

#[tokio::test]
async fn five_hundreds_are_retried_until_the_budget_is_spent() {
    let transport = failure_after(2);
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let value = client.send_text("1", "hi").await.unwrap();
    assert_eq!(value["messages"][0]["id"], "wamid.after-retry");
    assert_eq!(transport.call_count(), 3);
}

#[tokio::test]
async fn the_last_error_is_returned_once_the_budget_is_spent() {
    let transport = MockTransport::new();
    for _ in 0..5 {
        transport.push_response(response(
            503,
            &[],
            b"{\"error\":{\"code\":2,\"message\":\"down\"}}",
        ));
    }
    let client = client(transport.clone()).with_retry(instant_retry(2));

    let err = client.send_text("1", "hi").await.unwrap_err();
    match err {
        Error::Api {
            status, retryable, ..
        } => {
            assert_eq!(status, 503);
            assert!(retryable);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(transport.call_count(), 2, "bounded by max_attempts");
}

#[tokio::test]
async fn client_errors_are_terminal_and_not_retried() {
    let transport = MockTransport::new();
    transport.push_response(response(
        400,
        &[],
        b"{\"error\":{\"code\":131009,\"message\":\"bad parameter\"}}",
    ));
    transport.push_response(accepted("unused"));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.send_text("1", "hi").await.unwrap_err();
    match err {
        Error::Api {
            code,
            retryable,
            status,
            ..
        } => {
            assert_eq!(code, 131_009);
            assert_eq!(status, 400);
            assert!(!retryable);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn transport_failures_are_retried_only_when_retryable() {
    let retryable = MockTransport::new();
    retryable.push(Reply::Failure {
        message: "connection reset".into(),
        retryable: true,
    });
    retryable.push_response(accepted("wamid.1"));
    client(retryable.clone())
        .with_retry(instant_retry(2))
        .send_text("1", "hi")
        .await
        .unwrap();
    assert_eq!(retryable.call_count(), 2);

    let terminal = MockTransport::new();
    terminal.push(Reply::Failure {
        message: "unsupported scheme".into(),
        retryable: false,
    });
    let err = client(terminal.clone())
        .with_retry(instant_retry(3))
        .send_text("1", "hi")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Transport {
            retryable: false,
            ..
        }
    ));
    assert_eq!(terminal.call_count(), 1);
}

#[tokio::test]
async fn retries_can_be_disabled() {
    let transport = failure_after(0);
    let err = client(transport.clone())
        .with_retry(RetryPolicy::none())
        .send_text("1", "hi")
        .await;
    // `failure_after(0)` queues a 200, so this succeeds on the first try.
    assert!(err.is_ok());
    assert_eq!(transport.call_count(), 1);

    let failing = MockTransport::new();
    failing.push_response(response(500, &[], b"{}"));
    let err = client(failing.clone())
        .with_retry(RetryPolicy::none())
        .send_text("1", "hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Api { status: 500, .. }));
    assert_eq!(failing.call_count(), 1);
}

#[tokio::test]
async fn malformed_json_is_terminal() {
    let transport = MockTransport::new();
    transport.push_response(response(200, &[], b"{\"messages\": ["));
    let client = client(transport.clone()).with_retry(instant_retry(3));

    let err = client.send_text("1", "hi").await.unwrap_err();
    assert!(matches!(err, Error::Json { .. }));
    assert_eq!(transport.call_count(), 1, "a bad body is not retried");
}

#[tokio::test]
async fn the_configured_timeout_reaches_the_transport() {
    let transport = MockTransport::new();
    let client = client(transport.clone()).with_timeout(Some(Duration::from_millis(1500)));
    client.send_text("1", "hi").await.unwrap();

    let call: Call = transport.only_call();
    assert_eq!(call.timeout, Some(Duration::from_millis(1500)));
}

#[tokio::test]
async fn a_client_serves_many_tasks_concurrently() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}

    let transport = MockTransport::new();
    let client = Arc::new(client(transport.clone()));
    assert_send_sync(&*client);

    let mut handles = Vec::new();
    for index in 0..32 {
        let client = Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            client
                .send_text(&format!("4915{index:07}"), "hi")
                .await
                .unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }

    assert_eq!(transport.call_count(), 32);
}

#[test]
fn debug_does_not_leak_the_token() {
    let client = WhatsAppClient::new("1", "EAAG-super-secret", MockTransport::new());
    let rendered = format!("{client:?}");
    assert!(
        !rendered.contains("EAAG-super-secret"),
        "token leaked: {rendered}"
    );
    assert!(rendered.contains("phone_number_id"));
}

#[test]
fn retry_defaults_are_bounded() {
    let policy = RetryPolicy::default();
    assert_eq!(policy.max_attempts, 3);
    assert_eq!(policy.delay(1, None), Some(Duration::from_millis(250)));
    assert_eq!(policy.delay(2, None), Some(Duration::from_millis(500)));
    assert_eq!(policy.delay(3, None), None);
    // Retry-After is clamped to the ceiling.
    assert_eq!(
        policy.delay(1, Some(Duration::from_secs(600))),
        Some(Duration::from_secs(8))
    );
}
