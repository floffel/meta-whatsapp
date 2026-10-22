//! Parsing of real-shaped webhook payloads.

use meta_whatsapp::WebhookPayload;

/// Shaped like a real Meta delivery: one text message plus one status.
const SAMPLE: &str = r#"{
  "object": "whatsapp_business_account",
  "entry": [{
    "id": "102290129340398",
    "changes": [{
      "field": "messages",
      "value": {
        "messaging_product": "whatsapp",
        "metadata": {
          "display_phone_number": "15550001234",
          "phone_number_id": "106540352242922"
        },
        "contacts": [{ "profile": { "name": "Florian" }, "wa_id": "4915112345678" }],
        "messages": [{
          "from": "4915112345678",
          "id": "wamid.abc123",
          "timestamp": "1729600000",
          "type": "text",
          "text": { "body": "Hallo Welt" }
        }],
        "statuses": [{
          "id": "wamid.def456",
          "status": "delivered",
          "timestamp": "1729600005",
          "recipient_id": "4915112345678"
        }]
      }
    }]
  }]
}"#;

#[test]
fn parses_a_realistic_payload() {
    let payload: WebhookPayload = serde_json::from_str(SAMPLE).unwrap();
    assert_eq!(payload.object, "whatsapp_business_account");
    assert_eq!(payload.entry.len(), 1);

    let inbound: Vec<_> = payload.inbound_messages().collect();
    assert_eq!(inbound.len(), 1);
    let (phone_number_id, message) = inbound[0];
    assert_eq!(phone_number_id, "106540352242922");
    assert_eq!(message.from, "4915112345678");
    assert_eq!(message.text.as_ref().unwrap().body, "Hallo Welt");
    assert_eq!(message.message_type, "text");

    let statuses: Vec<_> = payload.statuses().collect();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].status, "delivered");
}

#[test]
fn tolerates_unknown_message_types() {
    let json = r#"{
      "object": "whatsapp_business_account",
      "entry": [{
        "id": "1",
        "changes": [{
          "field": "messages",
          "value": {
            "metadata": { "display_phone_number": "1", "phone_number_id": "2" },
            "messages": [{ "id": "x", "from": "3", "type": "sticker" }]
          }
        }]
      }]
    }"#;
    let payload: WebhookPayload = serde_json::from_str(json).unwrap();
    let inbound: Vec<_> = payload.inbound_messages().collect();
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].1.message_type, "sticker");
    assert!(inbound[0].1.text.is_none());
}

#[test]
fn messages_without_metadata_are_skipped() {
    let json = r#"{
      "object": "whatsapp_business_account",
      "entry": [{ "id": "1", "changes": [{ "field": "messages",
        "value": { "messages": [{ "id": "x", "from": "3", "type": "text" }] } }] }]
    }"#;
    let payload: WebhookPayload = serde_json::from_str(json).unwrap();
    assert_eq!(payload.inbound_messages().count(), 0);
}

#[test]
fn empty_entry_list_is_valid() {
    let payload: WebhookPayload =
        serde_json::from_str(r#"{"object":"whatsapp_business_account","entry":[]}"#).unwrap();
    assert_eq!(payload.inbound_messages().count(), 0);
    assert_eq!(payload.statuses().count(), 0);
}
