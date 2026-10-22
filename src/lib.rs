//! Typed client and webhook toolkit for the
//! [WhatsApp Cloud API](https://developers.facebook.com/docs/whatsapp/cloud-api).
//!
//! Two independent halves:
//!
//! * [`webhook`] — verify `X-Hub-Signature-256` and parse the payload into types.
//! * [`client`] — send text messages and mark them read via the Graph API.
//!
//! The HTTP layer is behind the [`Transport`] trait, so tests need no network
//! and you can plug in your own client stack.
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

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod client;
pub mod error;
pub mod transport;
pub mod types;
pub mod webhook;

pub use client::{WhatsAppClient, DEFAULT_API_VERSION};
pub use error::{Error, Result};
#[cfg(feature = "http")]
pub use transport::ReqwestTransport;
pub use transport::Transport;
pub use types::{Message, Status, WebhookPayload};
pub use webhook::WebhookVerifier;
