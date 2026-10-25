//! Send one text message: `cargo run --example send_text`.
//!
//! Needs the `http` feature (on by default) and three environment variables:
//!
//! ```text
//! WHATSAPP_PHONE_NUMBER_ID=106540352242922 \
//! WHATSAPP_TOKEN=EAAG... \
//! WHATSAPP_TO=4915112345678 \
//! cargo run --example send_text
//! ```
//!
//! Without the two credentials it prints the JSON body it would POST and exits,
//! so the example is runnable — and CI-buildable — on a machine with no
//! WhatsApp account.

// `WhatsApp`, `reqwest` and `JSON` are proper nouns, not identifiers - the
// same allowance `lib.rs` makes for the crate itself.
#![allow(clippy::doc_markdown)]

use std::env;

use meta_whatsapp::{MessageRequest, ReqwestTransport, TextMessage, WhatsAppClient};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let to = env::var("WHATSAPP_TO").unwrap_or_else(|_| "4915112345678".to_owned());
    let request = MessageRequest::new(
        &to,
        TextMessage::new("Hello from the meta-whatsapp example."),
    );

    let (Ok(phone_number_id), Ok(token)) = (
        env::var("WHATSAPP_PHONE_NUMBER_ID"),
        env::var("WHATSAPP_TOKEN"),
    ) else {
        // `to_json` validates before anything reaches the network: recipient
        // format, body length, the per-family limits. An invalid message is a
        // typed error here, not a 400 from the API.
        println!("WHATSAPP_PHONE_NUMBER_ID / WHATSAPP_TOKEN not set - request body instead:");
        println!("{}", serde_json::to_string_pretty(&request.to_json()?)?);
        return Ok(());
    };

    // The token is redacted in `Debug`; the client never prints it.
    let client = WhatsAppClient::new(phone_number_id, token, ReqwestTransport::new());

    match client.send(&request).await {
        Ok(response) => println!("accepted: {response}"),
        // The delivery contract in one arm: a 5xx or a response lost after the
        // request was written is *ambiguous* — the message may already be on
        // the recipient's phone. Retrying duplicates it, dropping it may lose
        // it, so the library refuses to guess and the caller decides.
        Err(err) if err.is_ambiguous() => {
            eprintln!("outcome unknown - check your ledger before retrying: {err}");
        }
        // 429 and pre-write connection failures were already retried by the
        // client; anything reaching this arm is terminal.
        Err(err) => return Err(err.into()),
    }

    Ok(())
}
