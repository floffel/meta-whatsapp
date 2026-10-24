//! Graph API client: send messages, move media, list templates.
//!
//! The client owns no I/O of its own — everything goes through
//! [`Transport`], which makes it testable against a recording mock and lets you
//! plug in your own HTTP stack. The bundled `crate::ReqwestTransport` is used
//! when the `http` feature is on (the default).
//!
//! ```
//! use meta_whatsapp::{ImageMessage, MediaRef, MessageRequest, TextMessage, WhatsAppClient};
//! # use meta_whatsapp::{Result, Transport};
//! # struct Mock;
//! # impl Transport for Mock {
//! #     async fn execute(&self, _: meta_whatsapp::transport::Request<'_>) -> Result<meta_whatsapp::transport::Response> {
//! #         Ok(meta_whatsapp::transport::Response::new(200, std::iter::empty(), b"{}".to_vec()))
//! #     }
//! # }
//! # async fn demo() -> Result<()> {
//! let client = WhatsAppClient::new("106540352242922", "EAAG...", Mock);
//!
//! client.send_text("4915112345678", "Hallo!").await?;
//! client
//!     .send(&MessageRequest::new(
//!         "4915112345678",
//!         ImageMessage::new(MediaRef::id("987654321")).caption("Speisekarte"),
//!     ))
//!     .await?;
//! # Ok(())
//! # }
//! ```

use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::media::{parse_retry_after, validate_media_id, MediaBytes, MediaInfo, MediaUpload};
use crate::message::{MessageRequest, TextMessage};
use crate::retry::RetryPolicy;
use crate::template::{TemplateList, TemplateQuery};
use crate::transport::{Request, Response, Transport};

/// Default Graph API version used when none is given.
///
/// ```
/// assert_eq!(meta_whatsapp::DEFAULT_API_VERSION, "v21.0");
/// ```
pub const DEFAULT_API_VERSION: &str = "v21.0";

/// Base URL of the Graph API.
///
/// ```
/// assert_eq!(meta_whatsapp::client::DEFAULT_BASE_URL, "https://graph.facebook.com");
/// ```
pub const DEFAULT_BASE_URL: &str = "https://graph.facebook.com";

/// Client bound to one business phone number.
///
/// Cloning is not required: take `&WhatsAppClient` from any number of tasks —
/// it is `Send + Sync` whenever the transport is.
///
/// ```
/// use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
///
/// struct Mock;
/// impl Transport for Mock {
///     async fn execute(&self, _: Request<'_>) -> Result<Response> {
///         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
///     }
/// }
///
/// let client = WhatsAppClient::new("1", "token", Mock).with_api_version("v20.0");
/// assert!(client.messages_url().contains("/v20.0/1/messages"));
/// ```
pub struct WhatsAppClient<T: Transport> {
    phone_number_id: String,
    token: String,
    api_version: String,
    base_url: String,
    timeout: Option<Duration>,
    retry: RetryPolicy,
    transport: T,
}

impl<T: Transport> WhatsAppClient<T> {
    /// Creates a client for `phone_number_id` using `token`.
    ///
    /// The token is only ever sent in the `Authorization` header and is
    /// redacted from [`std::fmt::Debug`].
    ///
    /// ```
    /// use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    ///
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// let client = WhatsAppClient::new("106540352242922", "EAAG", Mock);
    /// assert_eq!(client.phone_number_id(), "106540352242922");
    /// ```
    pub fn new(phone_number_id: impl Into<String>, token: impl Into<String>, transport: T) -> Self {
        Self {
            phone_number_id: phone_number_id.into(),
            token: token.into(),
            api_version: DEFAULT_API_VERSION.to_string(),
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: None,
            retry: RetryPolicy::default(),
            transport,
        }
    }

    /// Overrides the API version (for example `v20.0`).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// let client = WhatsAppClient::new("1", "t", Mock).with_api_version("v20.0");
    /// assert_eq!(client.api_version(), "v20.0");
    /// ```
    #[must_use]
    pub fn with_api_version(mut self, version: impl Into<String>) -> Self {
        self.api_version = version.into();
        self
    }

    /// Overrides the base URL — useful for tests or a proxy.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// let client = WhatsAppClient::new("1", "t", Mock).with_base_url("http://localhost:8080/");
    /// assert_eq!(client.messages_url(), "http://localhost:8080/v21.0/1/messages");
    /// ```
    #[must_use]
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Sets a per-request timeout. `None` (the default) leaves the transport's
    /// own timeout in place.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// let client = WhatsAppClient::new("1", "t", Mock)
    ///     .with_timeout(Some(std::time::Duration::from_secs(9)));
    /// assert_eq!(client.timeout().unwrap().as_secs(), 9);
    /// ```
    #[must_use]
    pub fn with_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.timeout = timeout;
        self
    }

    /// Replaces the retry policy (default: three attempts, 250 ms doubling to
    /// an 8 s ceiling, `Retry-After` honoured).
    ///
    /// ```
    /// use meta_whatsapp::RetryPolicy;
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// let client = WhatsAppClient::new("1", "t", Mock).with_retry(RetryPolicy::none());
    /// assert_eq!(client.retry().max_attempts, 1);
    /// ```
    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// The bound phone number id.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert_eq!(WhatsAppClient::new("7", "t", Mock).phone_number_id(), "7");
    /// ```
    #[must_use]
    pub fn phone_number_id(&self) -> &str {
        &self.phone_number_id
    }

    /// The configured Graph API version.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert_eq!(WhatsAppClient::new("7", "t", Mock).api_version(), "v21.0");
    /// ```
    #[must_use]
    pub fn api_version(&self) -> &str {
        &self.api_version
    }

    /// The per-request timeout, if set.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("7", "t", Mock).timeout().is_none());
    /// ```
    #[must_use]
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// The retry policy in force.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert_eq!(WhatsAppClient::new("7", "t", Mock).retry().max_attempts, 3);
    /// ```
    #[must_use]
    pub fn retry(&self) -> &RetryPolicy {
        &self.retry
    }

    /// The `messages` endpoint for the bound phone number.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert_eq!(
    ///     WhatsAppClient::new("1", "t", Mock).messages_url(),
    ///     "https://graph.facebook.com/v21.0/1/messages"
    /// );
    /// ```
    #[must_use]
    pub fn messages_url(&self) -> String {
        self.url(&format!("{}/messages", self.phone_number_id))
    }

    /// The media endpoint for one media id. Does not validate the id; the
    /// download and delete methods do.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("1", "t", Mock).media_url("42").ends_with("/v21.0/42"));
    /// ```
    #[must_use]
    pub fn media_url(&self, media_id: &str) -> String {
        self.url(media_id)
    }

    /// The upload endpoint for the bound phone number.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("1", "t", Mock).upload_url().ends_with("/v21.0/1/media"));
    /// ```
    #[must_use]
    pub fn upload_url(&self) -> String {
        self.url(&format!("{}/media", self.phone_number_id))
    }

    /// The template list endpoint for a business account.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("1", "t", Mock)
    ///     .templates_url("102290129340398")
    ///     .ends_with("/v21.0/102290129340398/message_templates"));
    /// ```
    #[must_use]
    pub fn templates_url(&self, waba_id: &str) -> String {
        self.url(&format!("{waba_id}/message_templates"))
    }

    /// Sends any typed message and returns the parsed Graph response
    /// (`{"messages": [{"id": "wamid..."}]}`).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, MessageRequest, TextMessage, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"messages":[{"id":"wamid.1"}]}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let response = client.send(&MessageRequest::new("2", TextMessage::new("hi"))).await?;
    /// assert_eq!(response["messages"][0]["id"], "wamid.1");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] before any request; then transport, API and
    /// decoding errors.
    pub async fn send(&self, message: &MessageRequest) -> Result<Value> {
        let payload = message.to_json()?;
        let url = self.messages_url();
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        response.json()
    }

    /// Sends a plain text message (link previews off).
    ///
    /// ```no_run
    /// # #[cfg(feature = "http")]
    /// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// use meta_whatsapp::{ReqwestTransport, WhatsAppClient};
    ///
    /// let client = WhatsAppClient::new("106540352242922", "EAAG...", ReqwestTransport::new());
    /// client.send_text("4915112345678", "Hallo!").await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or oversized text; then transport and
    /// Graph API errors.
    pub async fn send_text(&self, to: &str, body: &str) -> Result<Value> {
        self.send(&MessageRequest::new(to, TextMessage::new(body)))
            .await
    }

    /// Marks an inbound message as read (blue ticks).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{\"success\":true}".to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// assert_eq!(client.mark_read("wamid.1").await?["success"], true);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty message id; then transport and Graph
    /// API errors.
    pub async fn mark_read(&self, message_id: &str) -> Result<Value> {
        if message_id.is_empty() {
            return Err(Error::validation("message_id", "must not be empty"));
        }
        let payload = json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id,
        });
        let url = self.messages_url();
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        response.json()
    }

    /// Uploads media and returns the id to reference from messages.
    ///
    /// ```no_run
    /// # #[cfg(feature = "http")]
    /// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// use meta_whatsapp::{MediaUpload, ReqwestTransport, WhatsAppClient};
    ///
    /// let client = WhatsAppClient::new("106540352242922", "EAAG...", ReqwestTransport::new());
    /// let upload = MediaUpload::new("menu.pdf", "application/pdf", std::fs::read("menu.pdf")?)?;
    /// let media_id = client.upload_media(&upload).await?;
    /// println!("uploaded as {media_id}");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] from [`MediaUpload::new`] happens earlier; here
    /// transport, API errors and [`Error::Payload`] when the response has no
    /// `id`.
    pub async fn upload_media(&self, upload: &MediaUpload) -> Result<String> {
        upload.validate()?;
        let url = self.upload_url();
        let response = self
            .execute(Request::post_multipart(&url, &self.token, upload))
            .await?;
        let json = response.json()?;
        json.get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Payload(format!("upload response carried no id: {json}")))
    }

    /// Fetches metadata (including a short-lived URL) for a media id.
    ///
    /// ```no_run
    /// # #[cfg(feature = "http")]
    /// # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
    /// use meta_whatsapp::{ReqwestTransport, WhatsAppClient};
    ///
    /// let client = WhatsAppClient::new("1", "EAAG...", ReqwestTransport::new());
    /// let info = client.media_info("987654321").await?;
    /// println!("{:?} {:?}", info.mime_type, info.file_size);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media id; then transport, API and
    /// decoding errors.
    pub async fn media_info(&self, media_id: &str) -> Result<MediaInfo> {
        validate_media_id(media_id)?;
        let url = self.media_url(media_id);
        let response = self.execute(Request::get(&url, &self.token)).await?;
        serde_json::from_slice(response.body()).map_err(Error::from)
    }

    /// Downloads media by id and returns the bytes plus the served content type.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(
    /// #             200,
    /// #             [("Content-Type".to_string(), "image/png".to_string())],
    /// #             b"\x89PNG".to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let media = client.download_media("42").await?;
    /// assert_eq!(media.content_type(), Some("image/png"));
    /// assert_eq!(media.len(), 4);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media id; then transport and API
    /// errors.
    pub async fn download_media(&self, media_id: &str) -> Result<MediaBytes> {
        validate_media_id(media_id)?;
        let url = self.media_url(media_id);
        let response = self.execute(Request::get(&url, &self.token)).await?;
        Ok(MediaBytes::new(
            response.body().to_vec(),
            response.header("content-type").map(str::to_string),
        ))
    }

    /// Downloads from the CDN URL returned by [`WhatsAppClient::media_info`].
    ///
    /// The URL expires within minutes; the access token still has to be sent.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"abc".to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let bytes = client
    ///     .download_from_url("https://lookaside.fbsbx.com/whatsapp/1")
    ///     .await?;
    /// assert_eq!(bytes, b"abc");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a non-https URL; then transport and API errors.
    pub async fn download_from_url(&self, url: &str) -> Result<Vec<u8>> {
        if !url.starts_with("https://") {
            return Err(Error::validation("media.url", "must be an https:// URL"));
        }
        let response = self.execute(Request::get(url, &self.token)).await?;
        Ok(response.body().to_vec())
    }

    /// Deletes uploaded media (Meta keeps it 30 days otherwise).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"success":true}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// assert_eq!(client.delete_media("42").await?["success"], true);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an unsafe media id; then transport and API
    /// errors.
    pub async fn delete_media(&self, media_id: &str) -> Result<Value> {
        validate_media_id(media_id)?;
        let url = self.media_url(media_id);
        let response = self.execute(Request::delete(&url, &self.token)).await?;
        response.json()
    }

    /// Lists templates for a business account, one page at a time.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, TemplateQuery, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert!(request.url.contains("/message_templates?limit=2"));
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"data":[]}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let page = client.list_templates(&TemplateQuery::new("102290129340398").limit(2)).await?;
    /// assert!(page.data.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a bad WABA id or page size; then transport, API
    /// and decoding errors.
    pub async fn list_templates(&self, query: &TemplateQuery) -> Result<TemplateList> {
        query.validate()?;
        let base = self.templates_url(&query.waba_id);
        let query_string = query.query_string();
        let url = if query_string.is_empty() {
            base
        } else {
            format!("{base}?{query_string}")
        };
        let response = self.execute(Request::get(&url, &self.token)).await?;
        serde_json::from_slice(response.body()).map_err(Error::from)
    }

    /// Executes `request`, retrying transient failures per the policy.
    ///
    /// The loop is bounded by `policy.max_attempts`; every wait is capped and
    /// `tokio::time::sleep` is cancellation safe, so dropping the future stops
    /// the retries immediately.
    async fn execute(&self, request: Request<'_>) -> Result<Response> {
        let mut attempts_failed: u32 = 0;
        loop {
            attempts_failed = attempts_failed.saturating_add(1);
            let result = self
                .transport
                .execute(request.with_timeout(self.timeout))
                .await;
            let error = match result {
                Ok(response) if response.is_success() => return Ok(response),
                Ok(response) => api_error(&response),
                Err(error) => error,
            };

            if !error.is_retryable() {
                return Err(error);
            }
            match self.retry.delay(attempts_failed, error.retry_after()) {
                Some(delay) => sleep(delay).await,
                None => return Err(error),
            }
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.api_version,
            path
        )
    }
}

impl<T: Transport> std::fmt::Debug for WhatsAppClient<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the access token.
        f.debug_struct("WhatsAppClient")
            .field("phone_number_id", &self.phone_number_id)
            .field("api_version", &self.api_version)
            .field("base_url", &self.base_url)
            .field("timeout", &self.timeout)
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

/// Maps a non-2xx response to a typed error, extracting `Retry-After` and
/// classifying retryability.
fn api_error(response: &Response) -> Error {
    #[derive(serde::Deserialize, Default)]
    struct Envelope {
        error: Option<Body>,
    }
    #[derive(serde::Deserialize, Default)]
    struct Body {
        code: Option<i64>,
        message: Option<String>,
    }

    const MAX_CHARS: usize = 512;
    let status = response.status();
    let retry_after = response.header("retry-after").and_then(parse_retry_after);
    let retryable = status == 429 || (500..600).contains(&status);

    let decoded = serde_json::from_slice::<Envelope>(response.body())
        .ok()
        .and_then(|envelope| envelope.error);
    let (code, message) = match decoded {
        Some(body) => (body.code.unwrap_or(0), body.message.unwrap_or_default()),
        None => (0, String::new()),
    };

    let message = if message.is_empty() {
        // Not an envelope we recognise: keep the raw body, bounded, so the
        // caller can still see what went wrong.
        String::from_utf8_lossy(response.body())
            .chars()
            .take(MAX_CHARS)
            .collect()
    } else {
        message.chars().take(MAX_CHARS).collect()
    };

    Error::Api {
        code,
        status,
        message,
        retry_after,
        retryable,
    }
}

/// Waits between retries. With the `http` feature this is `tokio`'s timer; in
/// `--no-default-features` builds the crate has no runtime to wait on, so the
/// wait is skipped (and the retry happens immediately). Bring your own loop in
/// that configuration if pacing matters.
#[cfg(feature = "http")]
async fn sleep(delay: Duration) {
    tokio::time::sleep(delay).await;
}

#[cfg(not(feature = "http"))]
#[allow(clippy::unused_async)] // kept `async` so the call site is feature-agnostic
async fn sleep(_delay: Duration) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_is_parsed_from_the_header() {
        let response = Response::new(
            429,
            [("Retry-After".to_string(), "4".to_string())],
            br#"{"error":{"code":4,"message":"too many"}}"#.to_vec(),
        );
        let error = api_error(&response);
        assert!(error.is_retryable());
        assert_eq!(error.retry_after().unwrap().as_secs(), 4);
        match error {
            Error::Api { code, status, .. } => {
                assert_eq!(code, 4);
                assert_eq!(status, 429);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn client_errors_are_terminal_and_5xx_retryable() {
        assert!(!api_error(&Response::new(400, std::iter::empty(), Vec::new())).is_retryable());
        assert!(api_error(&Response::new(503, std::iter::empty(), Vec::new())).is_retryable());
    }

    #[test]
    fn non_json_error_bodies_are_kept_bounded() {
        let body = "x".repeat(2000).into_bytes();
        let error = api_error(&Response::new(502, std::iter::empty(), body));
        match error {
            Error::Api { message, .. } => assert_eq!(message.chars().count(), 512),
            other => panic!("unexpected {other:?}"),
        }
    }
}
