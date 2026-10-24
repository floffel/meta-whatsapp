//! Typed client and webhook toolkit for the
//! [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api).
//!
//! Two independent halves:
//!
//! * [`webhook`] — verify `X-Hub-Signature-256` in constant time, enforce a
//!   replay window, and parse the payload into types (message events, status
//!   events, and unknown future events).
//! * [`client`] — send every message family (text, media, location, contacts,
//!   reaction, interactive buttons/list/flow, templates), upload/download/delete
//!   media, and list templates — with retries on 429/5xx.
//!
//! The HTTP layer sits behind the [`Transport`] trait: tests need no network,
//! and you can plug in your own stack (or the bundled
//! `ReqwestTransport`).
//!
//! # Webhook
//!
//! ```
//! use meta_whatsapp::WebhookVerifier;
//!
//! let verifier = WebhookVerifier::new("app-secret")?;
//! let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
//! let header = verifier.sign(body);
//! let payload = verifier.verify_and_parse(body, &header)?;
//! assert!(payload.inbound_messages().next().is_none());
//! # Ok::<(), meta_whatsapp::Error>(())
//! ```
//!
//! # Client
//!
//! ```
//! use meta_whatsapp::{MessageRequest, TextMessage, WhatsAppClient};
//! # use meta_whatsapp::{Result, Transport, transport::{Request, Response}};
//! # struct Mock;
//! # impl Transport for Mock {
//! #     async fn execute(&self, _: Request<'_>) -> Result<Response> {
//! #         Ok(Response::new(200, std::iter::empty(), b"{}".to_vec()))
//! #     }
//! # }
//! # async fn demo() -> Result<()> {
//! let client = WhatsAppClient::new("106540352242922", "EAAG...", Mock);
//! client
//!     .send(&MessageRequest::new("4915112345678", TextMessage::new("Hallo!")))
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Feature flags
//!
//! * `http` (default) — pulls in `reqwest` and `tokio/time` and provides
//!   `ReqwestTransport`. Without it the crate is pure types + validation and
//!   compiles for `wasm32-unknown-unknown` style targets.
//!
//! # Security
//!
//! * `#![forbid(unsafe_code)]`.
//! * Secrets never reach `Debug`: [`WhatsAppClient`] and [`WebhookVerifier`]
//!   redact them.
//! * Signature comparison is constant time; an empty app secret is refused.
//! * Media ids and template query parameters are validated before they become
//!   URL path segments.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// `media::MediaUpload` reads better than `media::Upload`.
#![allow(clippy::module_name_repetitions)]

pub mod client;
pub mod error;
pub mod media;
pub mod message;
pub mod retry;
pub mod template;
pub mod transport;
pub mod types;
pub mod webhook;

pub use client::{WhatsAppClient, DEFAULT_API_VERSION};
pub use error::{Error, Result};
pub use media::{MediaBytes, MediaInfo, MediaRef, MediaUpload};
pub use message::{
    AudioMessage, ButtonMessage, ContactCard, ContactEmail, ContactName, ContactOrg, ContactPhone,
    ContactsMessage, DocumentMessage, EmailType, FlowAction, FlowMessage, FlowMode, HeaderText,
    ImageMessage, InteractiveMessage, ListMessage, ListRow, ListSection, LocationMessage,
    MessageBody, MessageRequest, PhoneType, ReactionMessage, RecipientType, ReplyButton,
    StickerMessage, TextMessage, UrlType, VideoMessage,
};
pub use retry::RetryPolicy;
pub use template::{
    Cursors, Paging, TemplateButtonType, TemplateComponent, TemplateLanguage, TemplateList,
    TemplateMediaKind, TemplateMessage, TemplateParameter, TemplateQuery, TemplateSummary,
};
#[cfg(feature = "http")]
pub use transport::ReqwestTransport;
pub use transport::Transport;
pub use types::{Change, ChangeValue, Entry, Message, Status, WebhookPayload};
pub use webhook::WebhookVerifier;
