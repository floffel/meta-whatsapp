//! Pluggable HTTP transport so the client stays testable without a network.
//!
//! The trait is deliberately one method wide: [`Transport::execute`] takes a
//! [`Request`] and returns a [`Response`]. Everything above it — status
//! classification, error envelopes, `Retry-After`, JSON decoding — lives in
//! [`crate::client`], so a mock transport can exercise every branch without a
//! socket.

use std::collections::BTreeMap;
use std::time::Duration;

use serde_json::Value;

#[cfg(feature = "http")]
use crate::error::TransportPhase;
use crate::error::{Error, Result};
use crate::media::MediaUpload;

/// HTTP verb. Only the three the Cloud API needs.
///
/// ```
/// use meta_whatsapp::transport::Method;
///
/// assert_eq!(Method::Post.as_str(), "POST");
/// assert_eq!(Method::Get.as_str(), "GET");
/// assert_eq!(Method::Delete.as_str(), "DELETE");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Method {
    /// Read a resource (media metadata, template list).
    Get,
    /// Send a message or upload media.
    Post,
    /// Remove uploaded media.
    Delete,
}

impl Method {
    /// Upper-case name as used on the wire.
    ///
    /// ```
    /// use meta_whatsapp::transport::Method;
    /// assert_eq!(Method::Delete.as_str(), "DELETE");
    /// ```
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Delete => "DELETE",
        }
    }
}

/// What goes into the request body.
///
/// ```
/// use meta_whatsapp::transport::{Method, Request, RequestBody};
///
/// let payload = serde_json::json!({"hello": "world"});
/// let request =
///     Request::post_json("https://graph.facebook.com/v21.0/1/messages", "token", &payload);
/// assert_eq!(request.method, Method::Post);
/// assert!(matches!(request.body, RequestBody::Json(_)));
/// ```
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub enum RequestBody<'a> {
    /// No body (GET, DELETE).
    Empty,
    /// `application/json`, already rendered.
    Json(&'a Value),
    /// `multipart/form-data`, encoded by [`MediaUpload::encode`].
    Multipart(&'a MediaUpload),
}

/// A single HTTP call, transport agnostic.
///
/// The access token travels in the `Authorization: Bearer` header; it is never
/// part of the URL, so it cannot leak through proxy logs or error messages.
///
/// ```
/// use meta_whatsapp::transport::Request;
///
/// let request = Request::get("https://graph.facebook.com/v21.0/123", "EAAG")
///     .with_timeout(Some(std::time::Duration::from_secs(5)));
/// assert_eq!(request.token, "EAAG");
/// assert_eq!(request.timeout.unwrap().as_secs(), 5);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// Verb to use.
    pub method: Method,
    /// Absolute URL.
    pub url: &'a str,
    /// Bearer token (the access token).
    pub token: &'a str,
    /// Per-request timeout; `None` leaves the transport default in place.
    pub timeout: Option<Duration>,
    /// Request body.
    pub body: RequestBody<'a>,
}

impl<'a> Request<'a> {
    /// A GET with no body.
    ///
    /// ```
    /// use meta_whatsapp::transport::{Method, Request};
    /// assert_eq!(Request::get("https://x.test/a", "t").method, Method::Get);
    /// ```
    #[must_use]
    pub fn get(url: &'a str, token: &'a str) -> Self {
        Self {
            method: Method::Get,
            url,
            token,
            timeout: None,
            body: RequestBody::Empty,
        }
    }

    /// A POST with a JSON body.
    ///
    /// ```
    /// use meta_whatsapp::transport::Request;
    /// let body = serde_json::json!({"a": 1});
    /// assert!(matches!(
    ///     Request::post_json("https://x.test/a", "t", &body).body,
    ///     meta_whatsapp::transport::RequestBody::Json(_)
    /// ));
    /// ```
    #[must_use]
    pub fn post_json(url: &'a str, token: &'a str, body: &'a Value) -> Self {
        Self {
            method: Method::Post,
            url,
            token,
            timeout: None,
            body: RequestBody::Json(body),
        }
    }

    /// A POST with a `multipart/form-data` body.
    ///
    /// ```
    /// use meta_whatsapp::transport::Request;
    /// use meta_whatsapp::MediaUpload;
    ///
    /// let upload = MediaUpload::new("a.txt", "text/plain", b"hi".to_vec())?;
    /// let request = Request::post_multipart("https://x.test/media", "t", &upload);
    /// assert!(matches!(
    ///     request.body,
    ///     meta_whatsapp::transport::RequestBody::Multipart(_)
    /// ));
    /// # Ok::<(), meta_whatsapp::Error>(())
    /// ```
    #[must_use]
    pub fn post_multipart(url: &'a str, token: &'a str, upload: &'a MediaUpload) -> Self {
        Self {
            method: Method::Post,
            url,
            token,
            timeout: None,
            body: RequestBody::Multipart(upload),
        }
    }

    /// A DELETE with no body.
    ///
    /// ```
    /// use meta_whatsapp::transport::{Method, Request};
    /// assert_eq!(Request::delete("https://x.test/a", "t").method, Method::Delete);
    /// ```
    #[must_use]
    pub fn delete(url: &'a str, token: &'a str) -> Self {
        Self {
            method: Method::Delete,
            url,
            token,
            timeout: None,
            body: RequestBody::Empty,
        }
    }

    /// A DELETE that carries a JSON body — the Graph API wants the ids to
    /// remove in the body, not the path.
    ///
    /// ```
    /// use meta_whatsapp::transport::{Method, Request};
    ///
    /// let body = serde_json::json!({"block_users": [{"user": "+16505551234"}]});
    /// let request = Request::delete_json("https://x.test/block_users", "t", &body);
    /// assert_eq!(request.method, Method::Delete);
    /// assert_eq!(request.body_json().unwrap()["block_users"][0]["user"], "+16505551234");
    /// ```
    #[must_use]
    pub fn delete_json(url: &'a str, token: &'a str, body: &'a Value) -> Self {
        Self {
            method: Method::Delete,
            url,
            token,
            timeout: None,
            body: RequestBody::Json(body),
        }
    }

    /// The JSON body, when this request carries one.
    ///
    /// ```
    /// use meta_whatsapp::transport::Request;
    ///
    /// let body = serde_json::json!({"a": 1});
    /// assert!(Request::post_json("https://x.test/a", "t", &body).body_json().is_some());
    /// assert!(Request::get("https://x.test/a", "t").body_json().is_none());
    /// ```
    #[must_use]
    pub fn body_json(&self) -> Option<&'a Value> {
        match self.body {
            RequestBody::Json(value) => Some(value),
            _ => None,
        }
    }

    /// Attaches a per-request timeout (no-op for `None`).
    ///
    /// ```
    /// use std::time::Duration;
    /// use meta_whatsapp::transport::Request;
    ///
    /// let request =
    ///     Request::get("https://x.test/a", "t").with_timeout(Some(Duration::from_secs(2)));
    /// assert_eq!(request.timeout, Some(Duration::from_secs(2)));
    /// ```
    #[must_use]
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }
}

/// A response as the client needs it: status, headers, body.
///
/// Header names are normalised to lower case on construction, so
/// [`Response::header`] works regardless of the casing the transport saw.
///
/// ```
/// use meta_whatsapp::transport::Response;
///
/// let response = Response::new(
///     200,
///     [("Content-Type".to_string(), "application/json".to_string())],
///     br#"{"ok":true}"#.to_vec(),
/// );
/// assert!(response.is_success());
/// assert_eq!(response.header("content-type"), Some("application/json"));
/// assert_eq!(response.json().unwrap()["ok"], true);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Response {
    /// Builds a response, lower-casing header names.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    ///
    /// let response = Response::new(204, std::iter::empty(), Vec::new());
    /// assert_eq!(response.json().unwrap(), serde_json::json!({}));
    /// ```
    pub fn new(
        status: u16,
        headers: impl IntoIterator<Item = (String, String)>,
        body: Vec<u8>,
    ) -> Self {
        Self {
            status,
            headers: headers
                .into_iter()
                .map(|(name, value)| (name.to_ascii_lowercase(), value))
                .collect(),
            body,
        }
    }

    /// HTTP status code.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    /// assert_eq!(Response::new(429, std::iter::empty(), Vec::new()).status(), 429);
    /// ```
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Case-insensitive header lookup.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    /// let response = Response::new(429, [("Retry-After".into(), "3".into())], Vec::new());
    /// assert_eq!(response.header("retry-after"), Some("3"));
    /// ```
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// Raw body bytes.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    /// assert_eq!(Response::new(200, std::iter::empty(), b"hi".to_vec()).body(), b"hi");
    /// ```
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Whether the status is 2xx.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    /// assert!(Response::new(201, std::iter::empty(), Vec::new()).is_success());
    /// assert!(!Response::new(500, std::iter::empty(), Vec::new()).is_success());
    /// ```
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Parses the body as JSON; an empty body becomes `{}` so callers can
    /// index the result unconditionally.
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    ///
    /// let response = Response::new(200, std::iter::empty(), br#"{"messages":[]}"#.to_vec());
    /// assert_eq!(response.json().unwrap()["messages"], serde_json::json!([]));
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Json`] when the body is not valid JSON.
    pub fn json(&self) -> Result<Value> {
        if self.body.iter().all(u8::is_ascii_whitespace) {
            return Ok(serde_json::json!({}));
        }
        serde_json::from_slice(&self.body).map_err(Error::from)
    }

    /// The rate-limit budget Meta reported in `X-Business-Use-Case-Usage`.
    ///
    /// The header is optional and absent on most responses (and on every
    /// non-Graph transport); `None` means "no budget information", not
    /// "zero usage". See [`crate::usage::BusinessUseCaseUsage`].
    ///
    /// ```
    /// use meta_whatsapp::transport::Response;
    ///
    /// let header = r#"{"123456":[{"type":"whatsapp","call_count":2,"total_cputime":1,"total_time":1,"estimated_time_to_regain_access":0}]}"#;
    /// let response = Response::new(
    ///     200,
    ///     [("X-Business-Use-Case-Usage".to_string(), header.to_string())],
    ///     br#"{"messages":[{"id":"wamid.1"}]}"#.to_vec(),
    /// );
    /// let usage = response.business_use_case_usage().unwrap();
    /// assert_eq!(usage.for_business("123456")[0].call_count, Some(2));
    /// ```
    #[must_use]
    pub fn business_use_case_usage(&self) -> Option<crate::usage::BusinessUseCaseUsage> {
        self.header("x-business-use-case-usage")
            .and_then(crate::usage::BusinessUseCaseUsage::from_header)
    }
}

/// Performs one HTTP request. Implement it for your own stack, or use
/// `ReqwestTransport`.
///
/// Implementations must:
///
/// * send `Authorization: Bearer <token>` and never log the token,
/// * return the raw status, headers and body even for non-2xx (the client
///   decides what is retryable),
/// * classify connection failures as [`Error::transport`] (retryable).
///
/// ```
/// use meta_whatsapp::transport::{Request, Response, Transport};
/// use meta_whatsapp::Result;
///
/// struct AlwaysOk;
///
/// impl Transport for AlwaysOk {
///     async fn execute(&self, request: Request<'_>) -> Result<Response> {
///         assert!(request.url.starts_with("https://"));
///         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
///     }
/// }
/// ```
#[allow(async_fn_in_trait)]
pub trait Transport {
    /// Executes `request` and returns the response, whatever its status.
    ///
    /// # Errors
    ///
    /// [`Error::Transport`] when no response could be obtained.
    fn execute(&self, request: Request<'_>) -> impl std::future::Future<Output = Result<Response>>;
}

/// `reqwest`-based transport, enabled by default via the `http` feature.
///
/// The client is cheap to clone and safe to share across tasks.
///
/// ```no_run
/// # #[cfg(feature = "http")]
/// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
/// use meta_whatsapp::{ReqwestTransport, WhatsAppClient};
///
/// let client = WhatsAppClient::new("1234567890", "EAAG...", ReqwestTransport::new());
/// client.send_text("4915112345678", "Hallo!").await?;
/// # Ok(())
/// # }
/// ```
#[cfg(feature = "http")]
#[derive(Debug, Clone, Default)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

#[cfg(feature = "http")]
impl ReqwestTransport {
    /// Creates a transport with a default client (rustls, redirects followed).
    ///
    /// ```
    /// # #[cfg(feature = "http")]
    /// let transport = meta_whatsapp::ReqwestTransport::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(feature = "http")]
impl Transport for ReqwestTransport {
    async fn execute(&self, request: Request<'_>) -> Result<Response> {
        let mut builder = match request.method {
            Method::Get => self.client.get(request.url),
            Method::Post => self.client.post(request.url),
            Method::Delete => self.client.delete(request.url),
        };
        builder = builder.bearer_auth(request.token);
        if let Some(timeout) = request.timeout {
            builder = builder.timeout(timeout);
        }
        builder = match request.body {
            RequestBody::Empty => builder,
            RequestBody::Json(value) => builder.json(value),
            RequestBody::Multipart(upload) => {
                let (content_type, bytes) = upload.encode();
                builder
                    .header(reqwest::header::CONTENT_TYPE, content_type)
                    .body(bytes)
            }
        };

        let response = builder
            .send()
            .await
            .map_err(|error| describe_reqwest_error(&error))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_string(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>();
        let body = response
            .bytes()
            .await
            .map_err(|error| describe_reqwest_error(&error))?
            .to_vec();

        Ok(Response::new(status, headers, body))
    }
}

/// What kind of connection failure a `reqwest` error reports, independent of
/// the HTTP stack. Split out so the phase mapping stays a pure, tested
/// function.
#[cfg(feature = "http")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureKind {
    /// DNS, TCP connect or TLS handshake failed.
    Connect,
    /// Timed out, or the body/response could not be read.
    Response,
    /// Anything else: a URL or builder error the transport cannot fix.
    Other,
}

/// Maps a failure kind to the request phase it implies.
///
/// A connect failure means nothing was written, so a retry is safe. Everything
/// else (timeout, truncated body, decode error) happens after the request went
/// out and is therefore ambiguous for a non-idempotent request.
#[cfg(feature = "http")]
fn phase_of(kind: FailureKind) -> TransportPhase {
    match kind {
        FailureKind::Connect => TransportPhase::BeforeWrite,
        FailureKind::Response => TransportPhase::AfterWrite,
        FailureKind::Other => TransportPhase::Terminal,
    }
}

/// Maps a `reqwest` failure to a transport error carrying its lifecycle phase,
/// without echoing headers (which would contain the token).
#[cfg(feature = "http")]
fn describe_reqwest_error(error: &reqwest::Error) -> Error {
    let (kind, label) = if error.is_connect() {
        (FailureKind::Connect, "connect")
    } else if error.is_timeout() {
        (FailureKind::Response, "timeout")
    } else if error.is_body() || error.is_decode() {
        (FailureKind::Response, "body")
    } else {
        (FailureKind::Other, "request")
    };
    // `reqwest::Error` renders the URL but never the Authorization header.
    Error::Transport {
        message: format!("{label}: {error}"),
        phase: phase_of(kind),
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::*;

    #[test]
    fn only_connect_failures_count_as_before_the_write() {
        assert_eq!(phase_of(FailureKind::Connect), TransportPhase::BeforeWrite);
        assert_eq!(phase_of(FailureKind::Response), TransportPhase::AfterWrite);
        assert_eq!(phase_of(FailureKind::Other), TransportPhase::Terminal);
    }
}
