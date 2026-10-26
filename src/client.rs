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

use crate::business::{
    block_users_payload, validate_field_list, BlockOutcome, BlockedUsersPage, BusinessProfile,
    BusinessProfileUpdate, PhoneNumberInfo, PHONE_NUMBER_FIELDS,
};
use crate::error::{Error, Result};
use crate::media::{parse_retry_after, validate_media_id, MediaBytes, MediaInfo, MediaUpload};
use crate::message::{MessageRequest, TextMessage};
use crate::retry::{Action, Attempt, Repeatability, RetryPolicy};
use crate::status::{validate_wamid, MessageStatus};
use crate::subscription::{
    validate_waba_id, SubscriptionConfig, SubscriptionState, WebhookSubscriptions,
};
use crate::template::{percent_encode, TemplateList, TemplateQuery};
use crate::transport::{Request, Response, Transport};
use crate::usage::BusinessUseCaseUsage;

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

    /// The *Read a Message* endpoint for one wamid: the phone-number node with
    /// the id in the query string.
    ///
    /// The wamid is percent-encoded, so the base64 payload (`+`, `/`, `=`)
    /// survives intact as a query value. Like [`WhatsAppClient::media_url`]
    /// this helper does not validate — [`WhatsAppClient::message_status`] and
    /// [`crate::status::validate_wamid`] do.
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
    ///     WhatsAppClient::new("1", "t", Mock).message_status_url("wamid.HBgLMTU="),
    ///     "https://graph.facebook.com/v21.0/1?id=wamid.HBgLMTU%3D&messaging_product=whatsapp"
    /// );
    /// ```
    #[must_use]
    pub fn message_status_url(&self, wamid: &str) -> String {
        format!(
            "{}?id={}&messaging_product=whatsapp",
            self.phone_number_url(),
            percent_encode(wamid)
        )
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
    /// Graph API errors. A 5xx arrives as [`Error::Ambiguous`], never as a
    /// silent retry.
    pub async fn send_text(&self, to: &str, body: &str) -> Result<Value> {
        self.send(&MessageRequest::new(to, TextMessage::new(body)))
            .await
    }

    /// Sends a message and returns Meta's rate-limit budget alongside the
    /// response body.
    ///
    /// The header is optional: `usage` is `None` when Meta did not send
    /// `X-Business-Use-Case-Usage` (or sent something unparsable). Use it to
    /// back off *before* hitting 429, not to change delivery semantics — a 5xx
    /// is still [`Error::Ambiguous`].
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, MessageRequest, TextMessage, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         let usage = r#"{"102290129340398":[{"type":"whatsapp","call_count":7,"total_cputime":40,"total_time":55,"estimated_time_to_regain_access":0}]}"#;
    /// #         Ok(Response::new(
    /// #             200,
    /// #             [("X-Business-Use-Case-Usage".to_string(), usage.to_string())],
    /// #             br#"{"messages":[{"id":"wamid.1"}]}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let outcome = client
    ///     .send_with_usage(&MessageRequest::new("2", TextMessage::new("hi")))
    ///     .await?;
    /// assert_eq!(outcome.response["messages"][0]["id"], "wamid.1");
    /// let usage = outcome.usage.unwrap();
    /// assert_eq!(usage.use_case("102290129340398", "whatsapp").unwrap().call_count, Some(7));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] before any request; then transport and Graph API
    /// errors, with a 5xx surfacing as [`Error::Ambiguous`].
    pub async fn send_with_usage(&self, message: &MessageRequest) -> Result<SendOutcome> {
        let payload = message.to_json()?;
        let url = self.messages_url();
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        Ok(SendOutcome {
            response: response.json()?,
            usage: response.business_use_case_usage(),
        })
    }

    /// Marks an inbound message as read (blue ticks).
    ///
    /// Marking read twice leaves the same state, so this POST is treated as
    /// idempotent: a 5xx is retried instead of surfacing as ambiguous.
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
            .execute_with(
                Request::post_json(&url, &self.token, &payload),
                Repeatability::Idempotent,
            )
            .await?;
        response.json()
    }

    /// Reads the current delivery status of a message you sent.
    ///
    /// This is the pull-side complement of the `statuses` webhook push: it
    /// answers `sent`, `delivered`, `read` or `failed` for one wamid, which is
    /// how you recover a lost webhook or run without a callback at all. The
    /// webhook still owns the timeline — poll this sparingly.
    ///
    /// It is a `GET`, so it is retried on 429 and 5xx like every other read
    /// ([`RetryPolicy::decide`]); asking twice cannot change anything.
    ///
    /// ```
    /// # use meta_whatsapp::{DeliveryStatus, Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert!(request.url.ends_with("?id=wamid.1&messaging_product=whatsapp"));
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"messages":[{"id":"wamid.1","status":"delivered",
    /// #                 "timestamp":"1729600000","recipient_id":"4915112345678"}]}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let status = client.message_status("wamid.1").await?;
    /// assert_eq!(status.status, DeliveryStatus::Delivered);
    /// assert_eq!(status.recipient_id, "4915112345678");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed wamid — empty, longer than
    /// [`MAX_WAMID_LEN`](crate::MAX_WAMID_LEN), or holding whitespace, NUL or
    /// query metacharacters — raised *before* a request is built. Then
    /// transport and Graph API errors, then [`Error::Payload`] when the body
    /// carries no message entry at all and [`Error::Json`] when the body is not
    /// JSON or an entry cannot be read.
    pub async fn message_status(&self, wamid: &str) -> Result<MessageStatus> {
        validate_wamid(wamid)?;
        let url = self.message_status_url(wamid);
        let response = self.execute(Request::get(&url, &self.token)).await?;
        MessageStatus::parse(&response.json()?)
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

    /// The business-profile endpoint for the bound phone number.
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
    ///     .business_profile_url()
    ///     .ends_with("/v21.0/1/whatsapp_business_profile"));
    /// ```
    #[must_use]
    pub fn business_profile_url(&self) -> String {
        self.url(&format!(
            "{}/whatsapp_business_profile",
            self.phone_number_id
        ))
    }

    /// The phone-number node (quality rating, limits, verification state).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("106540352242922", "t", Mock)
    ///     .phone_number_url()
    ///     .ends_with("/v21.0/106540352242922"));
    /// ```
    #[must_use]
    pub fn phone_number_url(&self) -> String {
        self.url(&self.phone_number_id)
    }

    /// The blocklist endpoint for the bound phone number.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
    /// #     }
    /// # }
    /// assert!(WhatsAppClient::new("1", "t", Mock).block_users_url().ends_with("/v21.0/1/block_users"));
    /// ```
    #[must_use]
    pub fn block_users_url(&self) -> String {
        self.url(&format!("{}/block_users", self.phone_number_id))
    }

    /// The webhook-subscription endpoint of a business account.
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
    ///     .subscribed_apps_url("102290129340398")
    ///     .ends_with("/v21.0/102290129340398/subscribed_apps"));
    /// ```
    #[must_use]
    pub fn subscribed_apps_url(&self, waba_id: &str) -> String {
        self.url(&format!("{waba_id}/subscribed_apps"))
    }

    /// Reads the business profile (about, address, description, e-mail,
    /// websites, vertical).
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert!(request.url.ends_with("/whatsapp_business_profile"));
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"data":[{"business_profile":{"about":"Kaffee","vertical":"RESTAURANT"}}]}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let profile = WhatsAppClient::new("1", "t", Mock).business_profile().await?;
    /// assert_eq!(profile.about.as_deref(), Some("Kaffee"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Transport and Graph API errors (429 is retried, this is a read), plus
    /// [`Error::Payload`] for an unexpected envelope.
    pub async fn business_profile(&self) -> Result<BusinessProfile> {
        let url = self.business_profile_url();
        let response = self.execute(Request::get(&url, &self.token)).await?;
        BusinessProfile::parse(&response.json()?)
    }

    /// Updates the business profile and returns Meta's `{"success": true}`.
    ///
    /// Only the fields set on the update are sent. This is a non-idempotent
    /// POST: a 5xx surfaces as [`Error::Ambiguous`] because the write may have
    /// happened.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # use meta_whatsapp::business::BusinessProfileUpdate;
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         let body = request.body_json().unwrap();
    /// #         assert_eq!(body["about"], "Kaffee");
    /// #         assert_eq!(body["messaging_product"], "whatsapp");
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"success":true}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let update = BusinessProfileUpdate::new().about("Kaffee");
    /// assert_eq!(client.update_business_profile(&update).await?["success"], true);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] before any request; then transport and Graph API
    /// errors, with a 5xx surfacing as [`Error::Ambiguous`].
    pub async fn update_business_profile(&self, update: &BusinessProfileUpdate) -> Result<Value> {
        let payload = update.to_json()?;
        let url = self.business_profile_url();
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        response.json()
    }

    /// Reads the bound number's quality rating, messaging-limit tier, account
    /// mode and verification state.
    ///
    /// Requests [`PHONE_NUMBER_FIELDS`], which is more than the Graph API
    /// returns by default — without it `quality_rating` is all you get.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # use meta_whatsapp::business::{MessagingLimitTier, QualityRating};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert!(request.url.contains("fields=id,display_phone_number"));
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"id":"1","quality_rating":"GREEN","messaging_limit_tier":"TIER_1K"}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let info = WhatsAppClient::new("1", "t", Mock).phone_number_info().await?;
    /// assert_eq!(info.quality_rating, Some(QualityRating::Green));
    /// assert_eq!(info.messaging_limit_tier, Some(MessagingLimitTier::Tier1K));
    /// assert_eq!(info.daily_contact_limit(), Some(1000));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Transport and Graph API errors (retried on 5xx: this is a read), plus
    /// [`Error::Payload`] for an unexpected envelope.
    pub async fn phone_number_info(&self) -> Result<PhoneNumberInfo> {
        let url = format!("{}?fields={PHONE_NUMBER_FIELDS}", self.phone_number_url());
        let response = self.execute(Request::get(&url, &self.token)).await?;
        PhoneNumberInfo::parse(&response.json()?)
    }

    /// [`WhatsAppClient::phone_number_info`] with an explicit field list.
    ///
    /// Field names must be lowercase identifiers (the same set the Graph API
    /// documents); anything else is rejected before the request so a value
    /// cannot smuggle a second query parameter into the URL.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert!(request.url.ends_with("?fields=id,quality_rating"));
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"id":"1","quality_rating":"YELLOW"}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let info = client.phone_number_info_with(&["id", "quality_rating"]).await?;
    /// assert_eq!(info.quality_rating.unwrap().as_str(), "YELLOW");
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty or unsafe field list; then transport,
    /// API and payload errors.
    pub async fn phone_number_info_with(&self, fields: &[&str]) -> Result<PhoneNumberInfo> {
        let joined = fields.join(",");
        validate_field_list(&joined)?;
        let url = format!("{}?fields={joined}", self.phone_number_url());
        let response = self.execute(Request::get(&url, &self.token)).await?;
        PhoneNumberInfo::parse(&response.json()?)
    }

    /// Lists the blocked users of the bound number, one page at a time.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"data":[{"wa_id":"16505551234"}]}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let page = WhatsAppClient::new("1", "t", Mock).blocked_users().await?;
    /// assert_eq!(page.data[0].wa_id.as_deref(), Some("16505551234"));
    /// assert!(!page.has_next_page());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Transport and Graph API errors; this is a read, so 5xx is retried.
    pub async fn blocked_users(&self) -> Result<BlockedUsersPage> {
        let url = self.block_users_url();
        let response = self.execute(Request::get(&url, &self.token)).await?;
        serde_json::from_slice(response.body()).map_err(Error::from)
    }

    /// Adds numbers to the blocklist.
    ///
    /// Numbers are validated first (optional `+`, 7–15 digits), so a typo is a
    /// [`Error::Validation`] and never reaches the network.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert_eq!(request.method.as_str(), "POST");
    /// #         assert_eq!(request.body_json().unwrap()["block_users"][0]["user"], "+16505551234");
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"block_users":{"added_users":[{"input":"+16505551234","wa_id":"16505551234"}]}}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let outcome = client.block_users(&["+16505551234"]).await?;
    /// assert_eq!(outcome.added[0].wa_id.as_deref(), Some("16505551234"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty list or a malformed number; then
    /// transport and Graph API errors, with a 5xx surfacing as
    /// [`Error::Ambiguous`].
    pub async fn block_users(&self, users: &[&str]) -> Result<BlockOutcome> {
        let payload = block_users_payload(users)?;
        let url = self.block_users_url();
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        BlockOutcome::from_value(&response.json()?)
    }

    /// Removes numbers from the blocklist.
    ///
    /// The ids travel in the body of a `DELETE`, as the reference requires.
    /// Deleting is idempotent, so a 5xx is retried.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert_eq!(request.method.as_str(), "DELETE");
    /// #         assert!(request.body_json().is_some());
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"block_users":{"removed_users":[{"input":"+16505551234","wa_id":"16505551234"}]}}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let outcome = client.unblock_users(&["+16505551234"]).await?;
    /// assert_eq!(outcome.removed.len(), 1);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for an empty list or a malformed number; then
    /// transport and Graph API errors.
    pub async fn unblock_users(&self, users: &[&str]) -> Result<BlockOutcome> {
        let payload = block_users_payload(users)?;
        let url = self.block_users_url();
        let response = self
            .execute(Request::delete_json(&url, &self.token, &payload))
            .await?;
        BlockOutcome::from_value(&response.json()?)
    }

    /// Lists the applications subscribed to a WABA's webhooks.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
    /// #         Ok(Response::new(
    /// #             200,
    /// #             std::iter::empty(),
    /// #             br#"{"data":[{"whatsapp_business_api_data":{"id":"1234","name":"Bot"}}]}"#.to_vec(),
    /// #         ))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let subs = WhatsAppClient::new("1", "t", Mock)
    ///     .webhook_subscriptions("102290129340398")
    ///     .await?;
    /// assert!(subs.has_app("1234"));
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed WABA id; then transport, API and
    /// payload errors.
    pub async fn webhook_subscriptions(&self, waba_id: &str) -> Result<WebhookSubscriptions> {
        validate_waba_id(waba_id)?;
        let url = self.subscribed_apps_url(waba_id);
        let response = self.execute(Request::get(&url, &self.token)).await?;
        WebhookSubscriptions::parse(&response.json()?)
    }

    /// Subscribes this app to a WABA's webhooks.
    ///
    /// `config` overrides the callback URL and sets the verification token; pass
    /// `None` to keep the app-level webhook configuration. Meta's answer is
    /// `{"success": true}`.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # use meta_whatsapp::subscription::SubscriptionConfig;
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert_eq!(request.method.as_str(), "POST");
    /// #         assert_eq!(request.body_json().unwrap()["verify_token"], "s3cret");
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"success":true}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let config = SubscriptionConfig::new().verify_token("s3cret");
    /// let value = client.subscribe_webhooks("102290129340398", Some(&config)).await?;
    /// assert_eq!(value["success"], true);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed WABA id or config; then transport
    /// and Graph API errors, with a 5xx surfacing as [`Error::Ambiguous`].
    pub async fn subscribe_webhooks(
        &self,
        waba_id: &str,
        config: Option<&SubscriptionConfig>,
    ) -> Result<Value> {
        validate_waba_id(waba_id)?;
        let payload = match config {
            Some(config) => config.to_json()?,
            None => json!({}),
        };
        let url = self.subscribed_apps_url(waba_id);
        let response = self
            .execute(Request::post_json(&url, &self.token, &payload))
            .await?;
        response.json()
    }

    /// Removes this app's subscription from a WABA.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         assert_eq!(request.method.as_str(), "DELETE");
    /// #         Ok(Response::new(200, std::iter::empty(), br#"{"success":true}"#.to_vec()))
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let value = WhatsAppClient::new("1", "t", Mock)
    ///     .unsubscribe_webhooks("102290129340398")
    ///     .await?;
    /// assert_eq!(value["success"], true);
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed WABA id; then transport and Graph
    /// API errors.
    pub async fn unsubscribe_webhooks(&self, waba_id: &str) -> Result<Value> {
        validate_waba_id(waba_id)?;
        let url = self.subscribed_apps_url(waba_id);
        let response = self.execute(Request::delete(&url, &self.token)).await?;
        response.json()
    }

    /// Makes sure `app_id` is subscribed to `waba_id`, subscribing only when it
    /// is missing.
    ///
    /// Safe to call on every start-up and again from an error path after a 401:
    /// a listed subscription is never re-POSTed, and a missing one is fixed.
    /// Meta does not document which failures drop a subscription (see
    /// *unverified* in the README), so this deliberately re-reads instead of
    /// re-subscribing blindly.
    ///
    /// ```
    /// # use meta_whatsapp::{Result, Transport, WhatsAppClient, transport::{Request, Response}};
    /// # use meta_whatsapp::subscription::SubscriptionState;
    /// # struct Mock;
    /// # impl Transport for Mock {
    /// #     async fn execute(&self, request: Request<'_>) -> Result<Response> {
    /// #         match request.method.as_str() {
    /// #             "GET" => Ok(Response::new(200, std::iter::empty(), br#"{"data":[]}"#.to_vec())),
    /// #             _ => Ok(Response::new(200, std::iter::empty(), br#"{"success":true}"#.to_vec())),
    /// #         }
    /// #     }
    /// # }
    /// # async fn demo() -> Result<()> {
    /// let client = WhatsAppClient::new("1", "t", Mock);
    /// let state = client.ensure_webhook_subscription("102290129340398", "1234").await?;
    /// assert_eq!(state, SubscriptionState::Subscribed);
    /// assert!(state.was_written());
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::Validation`] for a malformed WABA id; then transport, API and
    /// payload errors from the list or the subscribe call. A 401 from the list
    /// is returned as is — re-subscribing with a rejected token cannot help.
    pub async fn ensure_webhook_subscription(
        &self,
        waba_id: &str,
        app_id: &str,
    ) -> Result<SubscriptionState> {
        validate_waba_id(waba_id)?;
        if app_id.is_empty() {
            return Err(Error::validation("app_id", "must not be empty"));
        }
        let subscriptions = self.webhook_subscriptions(waba_id).await?;
        if subscriptions.has_app(app_id) {
            return Ok(SubscriptionState::AlreadySubscribed);
        }
        self.subscribe_webhooks(waba_id, None).await?;
        Ok(SubscriptionState::Subscribed)
    }

    /// Executes `request`, retrying only what it is safe to repeat.
    ///
    /// The decision itself lives in [`RetryPolicy::decide`] — a pure function —
    /// and this loop only sleeps and re-sends. A 429 or a connection failure
    /// before the write is retried; a 5xx (or a lost response) on a
    /// non-idempotent request is not, because the message may already have been
    /// delivered. That case surfaces as [`Error::Ambiguous`].
    ///
    /// The loop is bounded by `policy.max_attempts`; every wait is capped and
    /// `tokio::time::sleep` is cancellation safe, so dropping the future stops
    /// the retries immediately.
    async fn execute(&self, request: Request<'_>) -> Result<Response> {
        self.execute_with(request, Repeatability::of_method(request.method))
            .await
    }

    /// [`WhatsAppClient::execute`] with an explicit repeatability, for the POST
    /// endpoints that are idempotent anyway (marking a message read).
    async fn execute_with(
        &self,
        request: Request<'_>,
        repeatability: Repeatability,
    ) -> Result<Response> {
        let operation = format!("{} {}", request.method.as_str(), request.url);
        let mut attempts_failed: u32 = 0;
        loop {
            attempts_failed = attempts_failed.saturating_add(1);
            let result = self
                .transport
                .execute(request.with_timeout(self.timeout))
                .await;
            let error = match result {
                Ok(response) if response.is_success() => return Ok(response),
                Ok(response) => api_error(&response, repeatability),
                Err(error) => error,
            };

            match self
                .retry
                .decide(Attempt::classify(&error), repeatability, attempts_failed)
            {
                Action::Retry(delay) => sleep(delay).await,
                Action::Surface => return Err(error),
                Action::Ambiguous => {
                    // The request may have taken effect. Report it as such and
                    // never re-send: WhatsApp has no idempotency key that would
                    // let a duplicate be deduplicated.
                    if matches!(error, Error::Ambiguous { .. }) {
                        // A 5xx that `api_error` already classified.
                        return Err(error);
                    }
                    let status = error.status();
                    return Err(Error::ambiguous(operation, status, error));
                }
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

/// A send result plus Meta's rate-limit budget.
///
/// Returned by [`WhatsAppClient::send_with_usage`]; [`WhatsAppClient::send`]
/// and `send_text` stay as they are and return only the body.
///
/// ```
/// use meta_whatsapp::SendOutcome;
///
/// let mut outcome = SendOutcome::default();
/// outcome.response = serde_json::json!({"messages": [{"id": "wamid.1"}]});
/// assert_eq!(outcome.response["messages"][0]["id"], "wamid.1");
/// assert!(outcome.usage.is_none(), "the header is optional");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SendOutcome {
    /// The parsed Graph API response.
    pub response: Value,
    /// `X-Business-Use-Case-Usage`, when Meta sent a parseable value.
    pub usage: Option<BusinessUseCaseUsage>,
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
///
/// `repeatability` matters: a 5xx on a non-idempotent request may already have
/// been processed, so it is reported as not retryable — the retry loop then
/// wraps it in [`Error::Ambiguous`] rather than re-sending.
fn api_error(response: &Response, repeatability: Repeatability) -> Error {
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
    let server_error = (500..600).contains(&status);
    let retryable = status == 429 || (server_error && repeatability.is_idempotent());

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
        let error = api_error(&response, Repeatability::NotIdempotent);
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
    fn client_errors_are_terminal_and_5xx_depends_on_repeatability() {
        assert!(!api_error(
            &Response::new(400, std::iter::empty(), Vec::new()),
            Repeatability::Idempotent
        )
        .is_retryable());
        // A read can be repeated after a 5xx; a send cannot.
        assert!(api_error(
            &Response::new(503, std::iter::empty(), Vec::new()),
            Repeatability::Idempotent
        )
        .is_retryable());
        assert!(!api_error(
            &Response::new(503, std::iter::empty(), Vec::new()),
            Repeatability::NotIdempotent
        )
        .is_retryable());
    }

    #[test]
    fn non_json_error_bodies_are_kept_bounded() {
        let body = "x".repeat(2000).into_bytes();
        let error = api_error(
            &Response::new(502, std::iter::empty(), body),
            Repeatability::Idempotent,
        );
        match error {
            Error::Api { message, .. } => assert_eq!(message.chars().count(), 512),
            other => panic!("unexpected {other:?}"),
        }
    }
}
