//! Shared recording transport for the integration tests.
//!
//! Every request is captured (method, URL, token, timeout, body) and replies
//! come from a scripted queue, so tests can assert both what the client sent
//! and how it reacted to a sequence of statuses.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use meta_whatsapp::transport::{Method, Request, RequestBody, Response};
use meta_whatsapp::{Error, Result, Transport, WhatsAppClient};
use serde_json::Value;

/// Body as the transport saw it, `multipart` already encoded.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// No body.
    Empty,
    /// `application/json` payload.
    Json(Value),
    /// Encoded multipart payload plus its content type.
    Multipart {
        /// Full `Content-Type` header value including the boundary.
        content_type: String,
        /// Encoded body.
        bytes: Vec<u8>,
    },
}

/// One recorded call.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// Verb.
    pub method: Method,
    /// Full URL.
    pub url: String,
    /// Bearer token.
    pub token: String,
    /// Per-request timeout.
    pub timeout: Option<Duration>,
    /// Request body.
    pub body: Body,
}

/// A scripted reply.
#[derive(Clone)]
pub enum Reply {
    /// Return this response.
    Response(Response),
    /// Fail in the transport layer.
    Failure {
        /// Error message.
        message: String,
        /// Whether a retry could help.
        retryable: bool,
    },
}

/// Records calls and replays scripted replies (default: `200 {}`).
#[derive(Clone, Default)]
pub struct MockTransport {
    state: Arc<Mutex<State>>,
}

#[derive(Default)]
struct State {
    calls: Vec<Call>,
    queue: VecDeque<Reply>,
}

impl MockTransport {
    /// A transport that answers `200 {}` forever.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A transport that answers a fixed JSON body.
    #[must_use]
    pub fn json(value: &Value) -> Self {
        let transport = Self::new();
        transport.push(Reply::Response(json_response(value)));
        transport
    }

    /// Queues a reply.
    pub fn push(&self, reply: Reply) {
        self.state.lock().unwrap().queue.push_back(reply);
    }

    /// Queues a response.
    pub fn push_response(&self, response: Response) {
        self.push(Reply::Response(response));
    }

    /// Every recorded call, in order.
    #[must_use]
    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Number of recorded calls.
    #[must_use]
    pub fn call_count(&self) -> usize {
        self.state.lock().unwrap().calls.len()
    }

    /// The single recorded call; panics when the count is not exactly one.
    #[must_use]
    pub fn only_call(&self) -> Call {
        let calls = self.calls();
        assert_eq!(calls.len(), 1, "expected exactly one call, got {calls:#?}");
        calls.into_iter().next().unwrap()
    }
}

impl Transport for MockTransport {
    async fn execute(&self, request: Request<'_>) -> Result<Response> {
        // Yield once: the trait method is async, so keep this a real coroutine.
        std::future::ready(()).await;
        let body = match request.body {
            RequestBody::Json(value) => Body::Json(value.clone()),
            RequestBody::Multipart(upload) => {
                let (content_type, bytes) = upload.encode();
                Body::Multipart {
                    content_type,
                    bytes,
                }
            }
            _ => Body::Empty,
        };

        let reply = {
            let mut state = self.state.lock().unwrap();
            state.calls.push(Call {
                method: request.method,
                url: request.url.to_string(),
                token: request.token.to_string(),
                timeout: request.timeout,
                body,
            });
            state.queue.pop_front()
        };

        match reply {
            Some(Reply::Response(response)) => Ok(response),
            Some(Reply::Failure { message, retryable }) => Err(if retryable {
                Error::transport(message)
            } else {
                Error::transport_terminal(message)
            }),
            None => Ok(json_response(&serde_json::json!({}))),
        }
    }
}

/// Builds a response from string slices.
#[must_use]
pub fn response(status: u16, headers: &[(&str, &str)], body: &[u8]) -> Response {
    Response::new(
        status,
        headers
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect::<Vec<_>>(),
        body.to_vec(),
    )
}

/// A 200 with `value` as JSON.
#[must_use]
pub fn json_response(value: &Value) -> Response {
    response(
        200,
        &[("content-type", "application/json")],
        value.to_string().as_bytes(),
    )
}

/// The response Meta sends for an accepted message.
#[must_use]
pub fn accepted(message_id: &str) -> Response {
    json_response(&serde_json::json!({
        "messaging_product": "whatsapp",
        "contacts": [{ "wa_id": "4915112345678" }],
        "messages": [{ "id": message_id }],
    }))
}

/// A client wired to `transport` with a predictable base URL.
#[must_use]
pub fn client(transport: MockTransport) -> WhatsAppClient<MockTransport> {
    WhatsAppClient::new("106540352242922", "EAAG-test-token", transport)
        .with_base_url("https://graph.facebook.com")
}

/// A retry policy that never sleeps, so retry tests stay fast.
#[must_use]
pub fn instant_retry(attempts: u32) -> meta_whatsapp::RetryPolicy {
    meta_whatsapp::RetryPolicy::new(attempts)
        .with_base_delay(Duration::ZERO)
        .with_max_delay(Duration::ZERO)
}
