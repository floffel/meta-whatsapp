//! The bundled `ReqwestTransport`, exercised against a socket we control.
//!
//! Three paths matter and none needs the internet: a real HTTP response
//! (status, headers, body), a refused connection — which must be classified as
//! a `BeforeWrite` failure so a send can safely retry it — and a body that is
//! larger than the transport is allowed to buffer.
//!
//! The ceiling is tested from both sides (§2) for every way a body can arrive:
//! a declared `Content-Length`, a close-delimited body and a chunked body. The
//! declared length is only a hint — hyper never reads past the length it was
//! given — so the chunked and close-delimited cases are what exercise the
//! running total that a missing or unusable `Content-Length` cannot bypass.

#![cfg(feature = "http")]

use std::time::Duration;

use meta_whatsapp::error::TransportPhase;
use meta_whatsapp::transport::{Request, Transport};
use meta_whatsapp::{Error, ReqwestTransport, WhatsAppClient};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves one request with a fixed response and returns the base URL.
///
/// The request head is read to completion before answering: writing first (or
/// dropping the socket early) makes the client fail with a broken pipe on
/// Linux, which is a property of the harness, not of the transport.
async fn serve_once(response: &'static [u8]) -> String {
    serve_bytes(response.to_vec()).await
}

/// [`serve_once`] for a response that has to be built at run time.
async fn serve_bytes(response: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut head = Vec::new();
        let mut buffer = [0_u8; 512];
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    head.extend_from_slice(&buffer[..read]);
                    if head.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let _ = socket.write_all(&response).await;
        let _ = socket.flush().await;
        let _ = socket.shutdown().await;
    });
    format!("http://{address}")
}

/// Glues a response head (status line plus headers) to a body.
fn with_body(head: &str, body: &[u8]) -> Vec<u8> {
    let mut response = head.as_bytes().to_vec();
    response.extend_from_slice(body);
    response
}

#[tokio::test]
async fn a_real_response_is_mapped_to_status_headers_and_body() {
    let base = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
    )
    .await;

    let transport = ReqwestTransport::new();
    let response = transport
        .execute(Request::get(&format!("{base}/messages"), "EAAG-test"))
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    assert_eq!(response.header("content-type"), Some("application/json"));
    assert_eq!(response.json().unwrap()["ok"], true);
    assert!(response.is_success());
    assert!(response.business_use_case_usage().is_none());
}

#[tokio::test]
async fn a_non_2xx_status_is_returned_not_turned_into_an_error() {
    let base = serve_once(
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
    )
    .await;

    let response = ReqwestTransport::new()
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert!(!response.is_success());
}

#[tokio::test]
async fn a_refused_connection_is_a_before_write_failure() {
    // Port 1 on loopback is closed, so the connect fails immediately.
    let error = ReqwestTransport::new()
        .execute(
            Request::get("http://127.0.0.1:1/messages", "EAAG-test")
                .with_timeout(Some(Duration::from_secs(2))),
        )
        .await
        .unwrap_err();

    assert_eq!(
        error.transport_phase(),
        Some(TransportPhase::BeforeWrite),
        "nothing was written, so a send may retry: {error:?}"
    );
    assert!(error.is_retryable());
    assert!(!error.is_ambiguous());
    assert!(
        !error.to_string().contains("EAAG-test"),
        "the token must not leak: {error}"
    );
}

#[tokio::test]
async fn a_url_that_cannot_be_parsed_is_terminal() {
    let error = ReqwestTransport::new()
        .execute(Request::get("not-a-url::", "t"))
        .await
        .unwrap_err();
    assert_eq!(error.transport_phase(), Some(TransportPhase::Terminal));
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_declared_body_exactly_at_the_ceiling_is_accepted() {
    let base =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nabcd").await;

    let response = ReqwestTransport::new()
        .with_max_response_bytes(4)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap();
    assert_eq!(response.body(), b"abcd");
}

#[tokio::test]
async fn a_declared_body_one_byte_over_the_ceiling_is_rejected() {
    let base = serve_once(
        b"HTTP/1.1 200 OK\r\nContent-Length: 17\r\nConnection: close\r\n\r\nTOP-SECRET-BODY!",
    )
    .await;

    let error = ReqwestTransport::new()
        .with_max_response_bytes(16)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 16,
                status: 200
            }
        ),
        "{error:?}"
    );
    assert_eq!(error.transport_phase(), Some(TransportPhase::AfterWrite));
    assert_eq!(error.status(), Some(200), "the status is preserved");
    assert!(error.is_ambiguous(), "the server did answer");
    assert!(!error.is_retryable(), "the same body would come back");
    assert!(error.to_string().contains("16-byte limit"), "{error}");
    assert!(
        !error.to_string().contains("TOP-SECRET-BODY"),
        "an error must never echo the body: {error}"
    );
}

#[tokio::test]
async fn a_close_delimited_body_exactly_at_the_ceiling_is_accepted() {
    let base = serve_once(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcd").await;

    let response = ReqwestTransport::new()
        .with_max_response_bytes(4)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap();
    assert_eq!(response.body(), b"abcd");
}

#[tokio::test]
async fn a_close_delimited_body_one_byte_over_the_ceiling_is_rejected() {
    let base = serve_once(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcde").await;

    let error = ReqwestTransport::new()
        .with_max_response_bytes(4)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 4,
                status: 200
            }
        ),
        "{error:?}"
    );
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn a_chunked_body_exactly_at_the_ceiling_is_accepted() {
    let base = serve_once(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nab\r\n2\r\ncd\r\n0\r\n\r\n",
    )
    .await;

    let response = ReqwestTransport::new()
        .with_max_response_bytes(4)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap();
    assert_eq!(response.body(), b"abcd");
}

#[tokio::test]
async fn a_chunked_body_one_byte_over_the_ceiling_is_rejected() {
    let base = serve_once(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nab\r\n3\r\ncde\r\n0\r\n\r\n",
    )
    .await;

    let error = ReqwestTransport::new()
        .with_max_response_bytes(4)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 4,
                status: 200
            }
        ),
        "{error:?}"
    );
    assert_eq!(error.transport_phase(), Some(TransportPhase::AfterWrite));
}

#[tokio::test]
async fn an_empty_body_at_a_zero_ceiling_is_accepted() {
    let base =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;

    let response = ReqwestTransport::new()
        .with_max_response_bytes(0)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap();
    assert_eq!(response.body(), b"");
}

#[tokio::test]
async fn a_single_byte_at_a_zero_ceiling_is_rejected() {
    let base =
        serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx").await;

    let error = ReqwestTransport::new()
        .with_max_response_bytes(0)
        .execute(Request::get(&format!("{base}/messages"), "t"))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 0,
                status: 200
            }
        ),
        "{error:?}"
    );
    assert!(error.to_string().contains("0-byte limit"), "{error}");
}

#[tokio::test]
async fn a_post_over_the_ceiling_surfaces_typed_instead_of_ambiguous() {
    let payload = vec![b'x'; 32];
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let base = serve_bytes(with_body(&head, &payload)).await;

    let body = serde_json::json!({"messaging_product": "whatsapp"});
    let error = ReqwestTransport::new()
        .with_max_response_bytes(8)
        .execute(Request::post_json(
            &format!("{base}/messages"),
            "EAAG-test",
            &body,
        ))
        .await
        .unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 8,
                status: 200
            }
        ),
        "a write must not lose the typed error: {error:?}"
    );
    assert!(
        error.is_ambiguous(),
        "a send whose answer was dropped may have landed"
    );
    assert!(!error.is_retryable());
}

#[tokio::test]
async fn download_media_still_succeeds_at_the_default_ceiling() {
    let payload = vec![0xAB_u8; 1024 * 1024];
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let base = serve_bytes(with_body(&head, &payload)).await;

    let client = WhatsAppClient::new("106540352242922", "EAAG-test", ReqwestTransport::new())
        .with_base_url(&base);
    let media = client.download_media("987654321").await.unwrap();

    assert_eq!(media.len(), payload.len(), "a megabyte fits the default");
    assert_eq!(media.bytes(), payload.as_slice());
    assert_eq!(media.content_type(), Some("image/png"));
}

#[tokio::test]
async fn a_media_download_over_a_lowered_ceiling_surfaces_the_typed_error() {
    let payload = vec![0xCD_u8; 4096];
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let base = serve_bytes(with_body(&head, &payload)).await;

    let transport = ReqwestTransport::new().with_max_response_bytes(1024);
    let client =
        WhatsAppClient::new("106540352242922", "EAAG-test", transport).with_base_url(&base);
    let error = client.download_media("987654321").await.unwrap_err();

    assert!(
        matches!(
            error,
            Error::ResponseTooLarge {
                limit: 1024,
                status: 200
            }
        ),
        "the client must not swallow the typed error: {error:?}"
    );
    assert!(
        !error.to_string().contains("0xCD"),
        "the message stays free of body content: {error}"
    );
}
