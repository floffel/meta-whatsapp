//! The bundled `ReqwestTransport`, exercised against a socket we control.
//!
//! Two paths matter and neither needs the internet: a real HTTP response (status,
//! headers, body) and a refused connection, which must be classified as a
//! `BeforeWrite` failure so a send can safely retry it.

#![cfg(feature = "http")]

use std::time::Duration;

use meta_whatsapp::error::TransportPhase;
use meta_whatsapp::transport::{Request, Transport};
use meta_whatsapp::ReqwestTransport;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves one request with a fixed response and returns the base URL.
///
/// The request head is read to completion before answering: writing first (or
/// dropping the socket early) makes the client fail with a broken pipe on
/// Linux, which is a property of the harness, not of the transport.
async fn serve_once(response: &'static [u8]) -> String {
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
        let _ = socket.write_all(response).await;
        let _ = socket.flush().await;
        let _ = socket.shutdown().await;
    });
    format!("http://{address}")
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
