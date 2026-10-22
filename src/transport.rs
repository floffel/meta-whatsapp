//! Pluggable HTTP transport so the client stays testable without a network.

use crate::error::{Error, Result};

/// Minimal abstraction over "POST JSON with a bearer token".
///
/// Implement it for your own HTTP stack, or use [`ReqwestTransport`] when the
/// `http` feature is enabled.
#[allow(async_fn_in_trait)]
pub trait Transport {
    /// POSTs `body` to `url` with `Authorization: Bearer <token>` and returns
    /// the parsed JSON response (an empty object for 2xx without a body).
    fn post_json(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> impl std::future::Future<Output = Result<serde_json::Value>>;
}

/// `reqwest`-based transport, enabled by default via the `http` feature.
#[cfg(feature = "http")]
#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

#[cfg(feature = "http")]
impl ReqwestTransport {
    /// Creates a transport with a default client.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "http")]
impl Transport for ReqwestTransport {
    async fn post_json(
        &self,
        url: &str,
        token: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let response = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| Error::Transport(e.to_string()))?;

        if !status.is_success() {
            return Err(parse_graph_error(&text, status.as_u16()));
        }
        if text.trim().is_empty() {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_str(&text).map_err(|e| Error::Payload(e.to_string()))
    }
}

/// Turns a Graph API error envelope into [`Error::Api`], falling back to the
/// raw body when the shape is unexpected.
#[cfg(feature = "http")]
fn parse_graph_error(text: &str, status: u16) -> Error {
    #[derive(serde::Deserialize)]
    struct Envelope {
        error: Body,
    }
    #[derive(serde::Deserialize)]
    struct Body {
        code: i64,
        message: String,
    }

    match serde_json::from_str::<Envelope>(text) {
        Ok(Envelope { error }) => Error::Api {
            code: error.code,
            message: error.message,
        },
        Err(_) => Error::Transport(format!("http {status}: {text}")),
    }
}
