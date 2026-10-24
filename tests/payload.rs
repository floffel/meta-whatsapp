//! Inbound payload parsing: every message type Meta sends, plus the permissive
//! edge cases.

use meta_whatsapp::WebhookPayload;

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
          "recipient_id": "4915112345678",
          "biz_opaque_callback_data": "ticket-42",
          "conversation": { "id": "c1", "expiration_timestamp": "1729603600" },
          "pricing": { "billable": true, "pricing_model": "CBP" }
        }]
      }
    }]
  }]
}"#;

fn parse(json: &str) -> WebhookPayload {
    serde_json::from_str(json).expect("payload should parse")
}

#[test]
fn parses_a_realistic_payload() {
    let payload = parse(SAMPLE);
    assert_eq!(payload.object, "whatsapp_business_account");
    assert_eq!(payload.entry.len(), 1);

    let inbound: Vec<_> = payload.inbound_messages().collect();
    assert_eq!(inbound.len(), 1);
    let (phone_number_id, message) = inbound[0];
    assert_eq!(phone_number_id, "106540352242922");
    assert_eq!(message.from, "4915112345678");
    assert_eq!(message.text.as_ref().unwrap().body, "Hallo Welt");
    assert_eq!(message.message_type, "text");
    assert_eq!(message.timestamp_secs(), Some(1_729_600_000));

    let statuses: Vec<_> = payload.statuses().collect();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].status, "delivered");
    assert_eq!(
        statuses[0].biz_opaque_callback_data.as_deref(),
        Some("ticket-42")
    );
    assert_eq!(
        statuses[0].pricing.as_ref().unwrap()["pricing_model"],
        "CBP"
    );

    let contact = &payload.entry[0].changes[0].value.contacts[0];
    assert_eq!(contact.wa_id, "4915112345678");
    assert_eq!(contact.profile.name.as_deref(), Some("Florian"));
}

#[test]
fn every_inbound_media_type_exposes_its_block() {
    for (kind, block) in [
        (
            "image",
            r#"{"id":"1","mime_type":"image/jpeg","caption":"Katze","sha256":"abc"}"#,
        ),
        (
            "video",
            r#"{"id":"2","mime_type":"video/mp4","caption":"Clip"}"#,
        ),
        (
            "audio",
            r#"{"id":"3","mime_type":"audio/ogg","voice":true}"#,
        ),
        (
            "document",
            r#"{"id":"4","mime_type":"application/pdf","filename":"a.pdf","caption":"Rechnung"}"#,
        ),
        (
            "sticker",
            r#"{"id":"5","mime_type":"image/webp","animated":true}"#,
        ),
    ] {
        let json = format!(
            r#"{{"object":"whatsapp_business_account","entry":[{{"id":"1","changes":[{{"field":"messages","value":{{
                "metadata":{{"display_phone_number":"1","phone_number_id":"2"}},
                "messages":[{{"id":"wamid.{kind}","from":"3","type":"{kind}","timestamp":"10","{kind}":{block}}}]
            }}}}]}}]}}"#
        );
        let payload = parse(&json);
        let (_, message) = payload.inbound_messages().next().unwrap();
        let media = message
            .media()
            .unwrap_or_else(|| panic!("{kind} media missing"));
        assert!(!media.id.is_empty(), "{kind} id");
        assert_eq!(message.message_type, kind);
    }
}

#[test]
fn inbound_location_reaction_and_context_parse() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[
                {"id":"m1","from":"3","type":"location","timestamp":"10",
                 "location":{"latitude":48.7665,"longitude":11.4258,"name":"Bahnhof","address":"Str 1","url":"https://maps.example/1"}},
                {"id":"m2","from":"3","type":"reaction","timestamp":"11",
                 "reaction":{"message_id":"wamid.seen","emoji":"👍"}},
                {"id":"m3","from":"3","type":"text","timestamp":"12","text":{"body":"zitat"},
                 "context":{"from":"4","id":"wamid.quoted","forwarded":true,"frequently_forwarded":true}}
            ]
        }
    }]}]}"#;
    let payload = parse(json);
    let messages: Vec<_> = payload.inbound_messages().map(|(_, m)| m).collect();
    assert_eq!(messages.len(), 3);

    let location = messages[0].location.as_ref().unwrap();
    assert_eq!(location.name.as_deref(), Some("Bahnhof"));
    assert!((location.latitude - 48.7665).abs() < f64::EPSILON);

    let reaction = messages[1].reaction.as_ref().unwrap();
    assert_eq!(reaction.message_id, "wamid.seen");
    assert_eq!(reaction.emoji.as_deref(), Some("👍"));

    let context = messages[2].context.as_ref().unwrap();
    assert_eq!(context.id, "wamid.quoted");
    assert!(context.forwarded);
    assert!(context.frequently_forwarded);
}

#[test]
fn reaction_removal_parses_as_missing_emoji() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"id":"m1","from":"3","type":"reaction",
                         "reaction":{"message_id":"wamid.1","emoji":""}}]
        }
    }]}]}"#;
    let payload = parse(json);
    let reaction = payload
        .inbound_messages()
        .next()
        .unwrap()
        .1
        .reaction
        .clone()
        .unwrap();
    assert_eq!(reaction.emoji.as_deref(), Some(""));
}

#[test]
fn interactive_button_and_list_replies_parse() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[
                {"id":"m1","from":"3","type":"interactive","timestamp":"1",
                 "interactive":{"type":"button_reply","button_reply":{"id":"yes","title":"Ja"}}},
                {"id":"m2","from":"3","type":"interactive","timestamp":"2",
                 "interactive":{"type":"list_reply","list_reply":{"id":"pizza","title":"Pizza","description":"Mit Salami"}}},
                {"id":"m3","from":"3","type":"button","timestamp":"3",
                 "button":{"text":"Details","payload":"details"}}
            ]
        }
    }]}]}"#;
    let payload = parse(json);
    let messages: Vec<_> = payload.inbound_messages().map(|(_, m)| m).collect();

    let button = messages[0].interactive.as_ref().unwrap();
    assert_eq!(button.reply_type, "button_reply");
    assert_eq!(button.button_reply.as_ref().unwrap().id, "yes");

    let list = messages[1].interactive.as_ref().unwrap();
    assert_eq!(
        list.list_reply.as_ref().unwrap().description.as_deref(),
        Some("Mit Salami")
    );

    assert_eq!(
        messages[2].button.as_ref().unwrap().payload.as_deref(),
        Some("details")
    );
}

#[test]
fn shared_contact_cards_parse_permissively() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"id":"m1","from":"3","type":"contacts",
                "contacts":[{"name":{"formatted_name":"Ada Lovelace","first_name":"Ada"},
                             "phones":[{"phone":"+44","type":"CELL"}]}]}]
        }
    }]}]}"#;
    let payload = parse(json);
    let message = payload.inbound_messages().next().unwrap().1;
    let card = &message.contacts[0];
    assert_eq!(card.name.formatted_name.as_deref(), Some("Ada Lovelace"));
    assert!(card.extra.contains_key("phones"));
}

#[test]
fn statuses_carry_failure_details() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{"statuses":[{
            "id":"wamid.1","status":"failed","timestamp":"10","recipient_id":"3",
            "errors":[{"code":131026,"title":"Message undeliverable","message":"Recipient not on WhatsApp",
                       "error_data":{"details":"not on whatsapp"}}]
        }]}}]}]}"#;
    let payload = parse(json);
    let status = payload.statuses().next().unwrap();
    assert_eq!(status.status, "failed");
    assert_eq!(status.errors[0].code, 131_026);
    assert_eq!(
        status.errors[0].error_data.as_ref().unwrap()["details"],
        "not on whatsapp"
    );
    assert_eq!(status.timestamp_secs(), Some(10));
}

#[test]
fn camel_case_aliases_are_accepted() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"id":"m1","from":"3","type":"image",
                         "image":{"id":"7","mimeType":"image/png","fileSize":12}}]
        }
    }]}]}"#;
    let payload = parse(json);
    let media = payload
        .inbound_messages()
        .next()
        .unwrap()
        .1
        .media()
        .unwrap();
    assert_eq!(media.mime_type.as_deref(), Some("image/png"));
}

#[test]
fn unknown_message_types_are_tolerated() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"id":"x","from":"3","type":"brand_new_type","brand_new":{"a":1}}]
        }
    }]}]}"#;
    let payload = parse(json);
    let (_, message) = payload.inbound_messages().next().unwrap();
    assert_eq!(message.message_type, "brand_new_type");
    assert!(message.text.is_none());
    assert!(message.media().is_none());
    assert_eq!(message.extra["brand_new"]["a"], 1);
}

#[test]
fn messages_without_metadata_are_skipped() {
    let json = r#"{"object":"whatsapp_business_account","entry":[{"id":"1","changes":[{
        "field":"messages","value":{"messages":[{"id":"x","from":"3","type":"text"}]}
    }]}]}"#;
    assert_eq!(parse(json).inbound_messages().count(), 0);
}

#[test]
fn empty_entry_list_is_valid() {
    let payload = parse(r#"{"object":"whatsapp_business_account","entry":[]}"#);
    assert_eq!(payload.inbound_messages().count(), 0);
    assert_eq!(payload.statuses().count(), 0);
    assert_eq!(payload.changes().count(), 0);
    assert_eq!(payload.timestamps().count(), 0);
    assert_eq!(payload.newest_timestamp(), None);
}

#[test]
fn missing_required_fields_default_instead_of_failing() {
    let payload = parse(
        r#"{"entry":[{"changes":[{"value":{
            "metadata":{"display_phone_number":"1","phone_number_id":"2"},
            "messages":[{"type":"text"}]
        }}]}]}"#,
    );
    assert_eq!(payload.object, "");
    let (_, message) = payload.inbound_messages().next().unwrap();
    assert_eq!(message.id, "");
    assert_eq!(message.from, "");
}

#[test]
fn serialisation_round_trips() {
    let payload = parse(SAMPLE);
    let rendered = serde_json::to_string(&payload).unwrap();
    let reparsed: WebhookPayload = serde_json::from_str(&rendered).unwrap();
    assert_eq!(payload, reparsed);
}

#[test]
fn a_thousand_messages_parse() {
    let messages = (0..1000)
        .map(|index| {
            format!(
                r#"{{"id":"wamid.{index}","from":"3","type":"text","timestamp":"10","text":{{"body":"msg {index}"}}}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let json = format!(
        r#"{{"object":"whatsapp_business_account","entry":[{{"id":"1","changes":[{{"field":"messages","value":{{"metadata":{{"display_phone_number":"1","phone_number_id":"2"}},"messages":[{messages}]}}}}]}}]}}"#
    );
    let payload = parse(&json);
    assert_eq!(payload.inbound_messages().count(), 1000);
}
