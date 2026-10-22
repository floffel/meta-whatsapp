//! Client behaviour against a recording transport (no network).

use std::sync::{Arc, Mutex};

use meta_whatsapp::{Result, Transport, WhatsAppClient};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct RecordingTransport {
    calls: Arc<Mutex<Vec<(String, String, Value)>>>,
    response: Value,
}

impl RecordingTransport {
    fn new(response: Value) -> Self {
        Self {
            calls: Arc::default(),
            response,
        }
    }

    fn calls(&self) -> Vec<(String, String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Transport for RecordingTransport {
    fn post_json(
        &self,
        url: &str,
        token: &str,
        body: &Value,
    ) -> impl std::future::Future<Output = Result<Value>> {
        self.calls
            .lock()
            .unwrap()
            .push((url.to_string(), token.to_string(), body.clone()));
        std::future::ready(Ok(self.response.clone()))
    }
}

fn client(transport: RecordingTransport) -> WhatsAppClient<RecordingTransport> {
    WhatsAppClient::new("106540352242922", "EAAG-token", transport)
}

#[tokio::test]
async fn send_text_posts_to_the_messages_endpoint() {
    let transport = RecordingTransport::new(json!({"messages": [{"id": "wamid.1"}]}));
    let client = client(transport.clone());

    let response = client.send_text("4915112345678", "Hallo").await.unwrap();
    assert_eq!(response["messages"][0]["id"], "wamid.1");

    let calls = transport.calls();
    assert_eq!(calls.len(), 1);
    let (url, token, body) = &calls[0];
    assert!(
        url.ends_with("/v21.0/106540352242922/messages"),
        "url was {url}"
    );
    assert!(url.starts_with("https://"));
    assert_eq!(token, "EAAG-token");
    assert_eq!(body["messaging_product"], "whatsapp");
    assert_eq!(body["to"], "4915112345678");
    assert_eq!(body["type"], "text");
    assert_eq!(body["text"]["body"], "Hallo");
    assert_eq!(body["text"]["preview_url"], false);
}

#[tokio::test]
async fn mark_read_sends_a_read_status() {
    let transport = RecordingTransport::new(json!({"success": true}));
    let client = client(transport.clone());

    client.mark_read("wamid.abc123").await.unwrap();

    let calls = transport.calls();
    let (_, _, body) = &calls[0];
    assert_eq!(body["status"], "read");
    assert_eq!(body["message_id"], "wamid.abc123");
    assert_eq!(body["messaging_product"], "whatsapp");
}

#[test]
fn api_version_and_base_url_are_configurable() {
    let transport = RecordingTransport::new(json!({}));
    let host = ["https:/", "/api.example.com/"].join("");
    let client = client(transport)
        .with_api_version("v20.0")
        .with_base_url(host.clone());
    assert_eq!(
        client.messages_url(),
        format!("{host}v20.0/106540352242922/messages")
    );
}

#[tokio::test]
async fn transport_errors_are_propagated() {
    #[derive(Clone)]
    struct Failing;
    impl Transport for Failing {
        fn post_json(
            &self,
            _: &str,
            _: &str,
            _: &Value,
        ) -> impl std::future::Future<Output = Result<Value>> {
            std::future::ready(Err(meta_whatsapp::Error::Transport("boom".into())))
        }
    }

    let client = WhatsAppClient::new("1", "t", Failing);
    let err = client.send_text("2", "x").await.unwrap_err();
    assert!(matches!(err, meta_whatsapp::Error::Transport(_)));
}

#[test]
fn debug_does_not_leak_the_token() {
    let client = WhatsAppClient::new("1", "EAAG-super-secret", RecordingTransport::new(json!({})));
    let rendered = format!("{client:?}");
    assert!(
        !rendered.contains("EAAG-super-secret"),
        "token im Debug: {rendered}"
    );
}
