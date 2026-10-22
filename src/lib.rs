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
//! use meta_whatsapp::{WebhookVerifier, WhatsAppClient, ReqwestTransport};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! let client = WhatsAppClient::new("1234567890", "EAAG...", ReqwestTransport::new());
//! client.send_text("4915112345678", "Hallo!").await?;
//!
//! let verifier = WebhookVerifier::new("app-secret");
//! let body = br#"{"object":"whatsapp_business_account","entry":[]}"#;
//! let header = verifier.sign(body);
//! let payload = verifier.verify_and_parse(body, &header)?;
//! assert!(payload.inbound_messages().next().is_none());
//! # Ok(())
//! # }
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
