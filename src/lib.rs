//! Typed client and webhook toolkit for the
//! [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api).
//!
//! Three independent parts:
//!
//! * [`webhook`] — verify `X-Hub-Signature-256` in constant time, enforce a
//!   replay window, and parse the payload into types (message events, status
//!   events, and unknown future events).
//! * [`client`] — send every message family (text, media, location, contacts,
//!   reaction, interactive buttons/list/flow, catalog/product/order/address
//!   commerce payloads, templates), upload/download/delete media, list
//!   templates, and administer the account (business profile, phone-number
//!   quality and limits, blocklist, webhook subscriptions).
//! * [`business`], [`subscription`], [`usage`] — the typed shapes behind that
//!   administration surface, including Meta's `X-Business-Use-Case-Usage`
//!   budget header.
//!
//! The HTTP layer sits behind the [`Transport`] trait: tests need no network,
//! and you can plug in your own stack (or the bundled
//! `ReqwestTransport`).
//!
//! # Delivery semantics
//!
//! Sending a message is not safely repeatable: WhatsApp has no idempotency key,
//! so a second POST is a second message on the recipient's phone. The client
//! therefore retries **only** what is safe — 429 (refused before processing)
//! and connection failures *before* the request was written (DNS, connect, TLS)
//! — and reports a 5xx or a lost response on a POST as [`Error::Ambiguous`].
//! Reads and deletes keep retrying on 5xx. See [`RetryPolicy::decide`] for the
//! whole rule set as one pure function.
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
//! * Media ids, WABA ids, field lists and blocklist numbers are validated before
//!   they become URL path segments or query values.

#![forbid(unsafe_code)]
#![deny(missing_docs)]
// `media::MediaUpload` reads better than `media::Upload`.
#![allow(clippy::module_name_repetitions)]
// `WhatsApp`, `OpenAPI` and `Graph API` are proper nouns, not identifiers.
#![allow(clippy::doc_markdown)]

pub mod business;
pub mod client;
pub mod commerce;
pub mod error;
pub mod media;
pub mod message;
pub mod retry;
pub mod subscription;
pub mod template;
pub mod transport;
pub mod types;
pub mod usage;
pub mod webhook;

pub use business::{
    BlockChange, BlockOutcome, BlockedUser, BlockedUsersPage, BusinessProfile,
    BusinessProfileUpdate, MessagingLimitTier, PhoneNumberInfo, QualityRating,
};
pub use client::{SendOutcome, WhatsAppClient, DEFAULT_API_VERSION};
pub use commerce::{
    AddressField, AddressFieldType, AddressMessage, AddressReply, AddressType, AddressValue,
    CatalogMessage, Order, OrderDetailsMessage, OrderItem, OrderMessage, OrderStatus,
    OrderStatusMessage, ProductItem, ProductListMessage, ProductMessage, ProductSection,
    SavedAddress,
};
pub use error::{Error, Result, TransportPhase};
pub use media::{MediaBytes, MediaInfo, MediaRef, MediaUpload};
pub use message::{
    AudioMessage, ButtonMessage, ContactCard, ContactEmail, ContactName, ContactOrg, ContactPhone,
    ContactsMessage, DocumentMessage, EmailType, FlowAction, FlowMessage, FlowMode, HeaderText,
    ImageMessage, InteractiveMessage, ListMessage, ListRow, ListSection, LocationMessage,
    MessageBody, MessageRequest, PhoneType, ReactionMessage, RecipientType, ReplyButton,
    StickerMessage, TextMessage, UrlType, VideoMessage,
};
pub use retry::{Action, Attempt, Repeatability, RetryPolicy};
pub use subscription::{
    SubscribedApp, SubscriptionConfig, SubscriptionState, WebhookSubscriptions,
};
pub use template::{
    Cursors, Paging, TemplateButtonType, TemplateCategory, TemplateComponent, TemplateLanguage,
    TemplateList, TemplateMediaKind, TemplateMessage, TemplateParameter, TemplateQuery,
    TemplateSummary,
};
pub use transport::Transport;
#[cfg(feature = "http")]
pub use transport::{ReqwestTransport, DEFAULT_MAX_RESPONSE_BYTES};
pub use types::{Change, ChangeValue, Entry, Message, Status, WebhookPayload};
pub use usage::{BusinessUseCaseUsage, UseCaseUsage};
pub use webhook::WebhookVerifier;
