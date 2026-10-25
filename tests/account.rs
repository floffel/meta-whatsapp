//! Account administration: business profile, phone-number quality and limits,
//! blocklist, webhook subscriptions and the rate-limit budget header.
//!
//! Every path here is taken from the published OpenAPI description of the Cloud
//! API (see `docs/contracts.md`); the shapes that description does not pin down
//! are marked *unverified* in the README.

#![allow(clippy::doc_markdown)] // WhatsApp / OpenAPI are proper nouns.

mod common;

use common::{client, json_response, response, MockTransport, Reply};
use meta_whatsapp::business::{BusinessProfileUpdate, MessagingLimitTier, QualityRating};
use meta_whatsapp::subscription::{SubscriptionConfig, SubscriptionState};
use meta_whatsapp::{Error, WhatsAppClient};

const WABA: &str = "102290129340398";
const APP: &str = "1234567890123456";

fn usage_header() -> String {
    format!(
        r#"{{"{WABA}":[{{"type":"whatsapp","call_count":12,"total_cputime":40,"total_time":55,"estimated_time_to_regain_access":0}}]}}"#
    )
}

#[tokio::test]
async fn the_business_profile_read_hits_the_phone_number_node() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "data": [{
            "business_profile": {
                "messaging_product": "whatsapp",
                "about": "Kaffee seit 1998",
                "email": "hallo@example.test",
                "websites": ["https://example.test"],
                "vertical": "RESTAURANT",
                "unknown_future_field": 7
            }
        }]
    })));
    let client = client(transport.clone());

    let profile = client.business_profile().await.unwrap();

    assert_eq!(
        transport.only_call().url,
        "https://graph.facebook.com/v21.0/106540352242922/whatsapp_business_profile"
    );
    assert_eq!(profile.about.as_deref(), Some("Kaffee seit 1998"));
    assert_eq!(profile.websites, ["https://example.test"]);
    assert_eq!(profile.vertical.as_deref(), Some("RESTAURANT"));
    assert_eq!(
        profile.extra["unknown_future_field"], 7,
        "unknown fields kept"
    );
}

#[tokio::test]
async fn a_flat_business_profile_envelope_is_accepted_too() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "about": "flat",
        "fields_added_later": true
    })));
    let profile = client(transport.clone()).business_profile().await.unwrap();
    assert_eq!(profile.about.as_deref(), Some("flat"));
}

#[tokio::test]
async fn updating_the_profile_posts_only_the_fields_that_were_set() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({ "success": true })));
    let client = client(transport.clone());

    let update = BusinessProfileUpdate::new()
        .about("Kaffee")
        .website("https://example.test");
    let value = client.update_business_profile(&update).await.unwrap();

    assert_eq!(value["success"], true);
    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    match &call.body {
        common::Body::Json(body) => {
            assert_eq!(body["messaging_product"], "whatsapp");
            assert_eq!(body["about"], "Kaffee");
            assert_eq!(body["websites"][0], "https://example.test");
            assert!(body.get("address").is_none(), "unset fields stay out");
        }
        other => panic!("expected JSON, got {other:?}"),
    }
}

#[tokio::test]
async fn an_invalid_profile_update_never_reaches_the_network() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    for update in [
        BusinessProfileUpdate::new().about(""),
        BusinessProfileUpdate::new().about("x".repeat(140)),
        BusinessProfileUpdate::new().website("http://plain.test"),
        BusinessProfileUpdate::new().email("nope"),
        BusinessProfileUpdate::new().vertical(""),
    ] {
        let err = client.update_business_profile(&update).await.unwrap_err();
        assert!(matches!(err, Error::Validation { .. }), "{err:?}");
    }
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn phone_number_info_asks_for_quality_and_limits() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "id": "106540352242922",
        "display_phone_number": "+1 555-555-5555",
        "verified_name": "Jasper's Market",
        "quality_rating": "YELLOW",
        "code_verification_status": "VERIFIED",
        "name_status": "APPROVED",
        "messaging_limit_tier": "TIER_10K",
        "account_mode": "LIVE",
        "is_official_business_account": false
    })));
    let client = client(transport.clone());

    let info = client.phone_number_info().await.unwrap();

    let url = transport.only_call().url;
    assert!(url.contains("/v21.0/106540352242922?fields="), "{url}");
    assert!(url.contains("quality_rating"));
    assert!(url.contains("messaging_limit_tier"));
    assert_eq!(info.quality_rating, Some(QualityRating::Yellow));
    assert_eq!(info.messaging_limit_tier, Some(MessagingLimitTier::Tier10K));
    assert_eq!(info.daily_contact_limit(), Some(10_000));
    assert_eq!(info.code_verification_status.as_deref(), Some("VERIFIED"));
    assert!(!info.is_official_business_account.unwrap());
    assert!(info.permits_broadcast());
}

#[tokio::test]
async fn a_raw_quality_string_the_crate_does_not_know_still_parses() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "quality_rating": "PLATINUM",
        "messaging_limit_tier": "TIER_1M"
    })));
    let info = client(transport.clone()).phone_number_info().await.unwrap();
    assert_eq!(
        info.quality_rating,
        Some(QualityRating::Unknown("PLATINUM".into()))
    );
    assert_eq!(
        info.messaging_limit_tier,
        Some(MessagingLimitTier::Unknown("TIER_1M".into()))
    );
    assert_eq!(info.daily_contact_limit(), None);
}

#[tokio::test]
async fn the_field_list_is_validated_before_it_becomes_a_query() {
    let transport = MockTransport::new();
    let client = client(transport.clone());

    for fields in [
        &[][..],
        &["id&access_token=x"][..],
        &["id,", "name"][..],
        &["ID"][..],
        &["id", ""][..],
    ] {
        let err = client.phone_number_info_with(fields).await.unwrap_err();
        assert!(
            matches!(err, Error::Validation { .. }),
            "{fields:?}: {err:?}"
        );
    }
    assert_eq!(transport.call_count(), 0);

    transport.push_response(json_response(&serde_json::json!({ "id": "1" })));
    client
        .phone_number_info_with(&["id", "messaging_limit_tier"])
        .await
        .unwrap();
    assert!(transport
        .only_call()
        .url
        .ends_with("?fields=id,messaging_limit_tier"));
}

#[tokio::test]
async fn the_blocklist_is_read_blocked_and_unblocked() {
    let read = MockTransport::new();
    read.push_response(json_response(&serde_json::json!({
        "data": [{"messaging_product": "whatsapp", "wa_id": "16505551234"}],
        "paging": {"next": "https://graph.facebook.com/next"}
    })));
    let page = client(read.clone()).blocked_users().await.unwrap();
    assert_eq!(read.only_call().method.as_str(), "GET");
    assert!(read.only_call().url.ends_with("/block_users"));
    assert_eq!(page.data[0].wa_id.as_deref(), Some("16505551234"));
    assert!(page.has_next_page());

    let blocked = MockTransport::new();
    blocked.push_response(json_response(&serde_json::json!({
        "block_users": {"added_users": [{"input": "+16505551234", "wa_id": "16505551234"}]},
        "messaging_product": "whatsapp"
    })));
    let outcome = client(blocked.clone())
        .block_users(&["+16505551234", "16505551235"])
        .await
        .unwrap();
    let call = blocked.only_call();
    assert_eq!(call.method.as_str(), "POST");
    match &call.body {
        common::Body::Json(body) => {
            assert_eq!(body["block_users"].as_array().unwrap().len(), 2);
            assert_eq!(body["block_users"][1]["user"], "16505551235");
        }
        other => panic!("expected JSON, got {other:?}"),
    }
    assert_eq!(outcome.added[0].wa_id.as_deref(), Some("16505551234"));

    let unblocked = MockTransport::new();
    unblocked.push_response(json_response(&serde_json::json!({
        "block_users": {"removed_users": [{"input": "+16505551234", "wa_id": "16505551234"}]}
    })));
    let outcome = client(unblocked.clone())
        .unblock_users(&["+16505551234"])
        .await
        .unwrap();
    let call = unblocked.only_call();
    assert_eq!(call.method.as_str(), "DELETE");
    assert!(
        matches!(call.body, common::Body::Json(_)),
        "ids go in the body"
    );
    assert_eq!(outcome.removed.len(), 1);
}

#[tokio::test]
async fn malformed_block_requests_never_reach_the_network() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    for users in [&[][..], &[""][..], &["+123456"][..], &["+1 650 555"][..]] {
        let err = client.block_users(users).await.unwrap_err();
        assert!(matches!(err, Error::Validation { .. }), "{users:?}");
        let err = client.unblock_users(users).await.unwrap_err();
        assert!(matches!(err, Error::Validation { .. }), "{users:?}");
    }
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn subscriptions_are_listed_subscribed_and_removed() {
    let list = MockTransport::new();
    list.push_response(json_response(&serde_json::json!({
        "data": [{"whatsapp_business_api_data": {"id": APP, "name": "Bot"}}]
    })));
    let subs = client(list.clone())
        .webhook_subscriptions(WABA)
        .await
        .unwrap();
    assert!(list
        .only_call()
        .url
        .ends_with(&format!("/{WABA}/subscribed_apps")));
    assert!(subs.has_app(APP));

    let subscribe = MockTransport::new();
    subscribe.push_response(json_response(&serde_json::json!({ "success": true })));
    let config = SubscriptionConfig::new()
        .callback_url("https://example.test/hooks/whatsapp")
        .verify_token("s3cret");
    client(subscribe.clone())
        .subscribe_webhooks(WABA, Some(&config))
        .await
        .unwrap();
    let call = subscribe.only_call();
    assert_eq!(call.method.as_str(), "POST");
    match &call.body {
        common::Body::Json(body) => {
            assert_eq!(
                body["override_callback_uri"],
                "https://example.test/hooks/whatsapp"
            );
            assert_eq!(body["verify_token"], "s3cret");
        }
        other => panic!("expected JSON, got {other:?}"),
    }

    // Without a config the body is empty, not absent: the endpoint accepts {}.
    let plain = MockTransport::new();
    plain.push_response(json_response(&serde_json::json!({ "success": true })));
    client(plain.clone())
        .subscribe_webhooks(WABA, None)
        .await
        .unwrap();
    match &plain.only_call().body {
        common::Body::Json(body) => assert_eq!(body, &serde_json::json!({})),
        other => panic!("expected JSON, got {other:?}"),
    }

    let remove = MockTransport::new();
    remove.push_response(json_response(&serde_json::json!({ "success": true })));
    client(remove.clone())
        .unsubscribe_webhooks(WABA)
        .await
        .unwrap();
    let call = remove.only_call();
    assert_eq!(call.method.as_str(), "DELETE");
    assert!(matches!(call.body, common::Body::Empty));
}

#[tokio::test]
async fn a_waba_id_that_is_not_numeric_is_rejected_before_the_request() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    for waba in ["", "../me", "12a", &"1".repeat(33)] {
        let err = client.webhook_subscriptions(waba).await.unwrap_err();
        assert!(matches!(
            err,
            Error::Validation {
                field: "waba_id",
                ..
            }
        ));
        let err = client.subscribe_webhooks(waba, None).await.unwrap_err();
        assert!(matches!(
            err,
            Error::Validation {
                field: "waba_id",
                ..
            }
        ));
        let err = client.unsubscribe_webhooks(waba).await.unwrap_err();
        assert!(matches!(
            err,
            Error::Validation {
                field: "waba_id",
                ..
            }
        ));
    }
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn ensure_subscription_does_not_re_post_an_existing_subscription() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({
        "data": [{"whatsapp_business_api_data": {"id": APP}}]
    })));
    let state = client(transport.clone())
        .ensure_webhook_subscription(WABA, APP)
        .await
        .unwrap();
    assert_eq!(state, SubscriptionState::AlreadySubscribed);
    assert!(!state.was_written());
    assert_eq!(transport.call_count(), 1, "one GET, no POST");
}

#[tokio::test]
async fn ensure_subscription_writes_exactly_once_when_the_app_is_missing() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&serde_json::json!({ "data": [] })));
    transport.push_response(json_response(&serde_json::json!({ "success": true })));
    let state = client(transport.clone())
        .ensure_webhook_subscription(WABA, APP)
        .await
        .unwrap();
    assert_eq!(state, SubscriptionState::Subscribed);
    assert!(state.was_written());
    let calls = transport.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].method.as_str(), "GET");
    assert_eq!(calls[1].method.as_str(), "POST");
}

#[tokio::test]
async fn a_401_on_the_subscription_list_does_not_trigger_a_blind_write() {
    // Meta does not document that a 401 means "subscription lost", so the
    // client reports the error and makes no write (see README: unverified).
    let transport = MockTransport::new();
    transport.push_response(response(
        401,
        &[],
        b"{\"error\":{\"code\":190,\"message\":\"Invalid OAuth access token\"}}",
    ));
    transport.push_response(json_response(&serde_json::json!({ "success": true })));

    let err = client(transport.clone())
        .ensure_webhook_subscription(WABA, APP)
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Api { status: 401, .. }));
    assert!(!err.is_retryable(), "a rejected token is terminal");
    assert_eq!(transport.call_count(), 1, "no subscribe attempt");
}

#[tokio::test]
async fn an_empty_app_id_is_rejected() {
    let transport = MockTransport::new();
    let err = client(transport.clone())
        .ensure_webhook_subscription(WABA, "")
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "app_id",
            ..
        }
    ));
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn the_usage_budget_is_surfaced_when_meta_sends_it() {
    let transport = MockTransport::new();
    transport.push_response(response(
        200,
        &[("X-Business-Use-Case-Usage", &usage_header())],
        br#"{"messages":[{"id":"wamid.1"}]}"#,
    ));
    let client = client(transport.clone());

    let outcome = client
        .send_with_usage(&meta_whatsapp::MessageRequest::new(
            "4915112345678",
            meta_whatsapp::TextMessage::new("hi"),
        ))
        .await
        .unwrap();

    assert_eq!(outcome.response["messages"][0]["id"], "wamid.1");
    let usage = outcome.usage.unwrap();
    let entry = usage.use_case(WABA, "whatsapp").unwrap();
    assert_eq!(entry.call_count, Some(12));
    assert_eq!(entry.total_cputime, Some(40));
    assert_eq!(entry.total_time, Some(55));
    assert!(!usage.is_throttled(WABA));
}

#[tokio::test]
async fn a_missing_or_broken_usage_header_never_fails_a_send() {
    let missing = MockTransport::new();
    missing.push_response(json_response(&serde_json::json!({
        "messages": [{"id": "wamid.1"}]
    })));
    let outcome = client(missing.clone())
        .send_with_usage(&meta_whatsapp::MessageRequest::new(
            "1",
            meta_whatsapp::TextMessage::new("hi"),
        ))
        .await
        .unwrap();
    assert!(outcome.usage.is_none());

    let broken = MockTransport::new();
    broken.push_response(response(
        200,
        &[("X-Business-Use-Case-Usage", "not json")],
        br#"{"messages":[{"id":"wamid.1"}]}"#,
    ));
    let outcome = client(broken.clone())
        .send_with_usage(&meta_whatsapp::MessageRequest::new(
            "1",
            meta_whatsapp::TextMessage::new("hi"),
        ))
        .await
        .unwrap();
    assert!(
        outcome.usage.is_none(),
        "telemetry must not break messaging"
    );
    assert_eq!(outcome.response["messages"][0]["id"], "wamid.1");
}

#[tokio::test]
async fn the_budget_header_is_readable_on_a_retried_read() {
    let transport = MockTransport::new();
    transport.push(Reply::not_sent("dns"));
    transport.push_response(response(
        200,
        &[("x-business-use-case-usage", &usage_header())],
        br#"{"id":"1","quality_rating":"GREEN"}"#,
    ));
    let client = client(transport.clone()).with_retry(
        meta_whatsapp::RetryPolicy::new(2)
            .with_base_delay(std::time::Duration::ZERO)
            .with_max_delay(std::time::Duration::ZERO),
    );

    let info = client.phone_number_info().await.unwrap();
    assert_eq!(info.quality_rating, Some(QualityRating::Green));
    assert_eq!(transport.call_count(), 2);
}

#[test]
fn the_client_stays_send_and_sync_with_the_new_surface() {
    fn assert_send_sync<T: Send + Sync>(_: &T) {}
    let transport = MockTransport::new();
    let client: WhatsAppClient<MockTransport> = client(transport);
    assert_send_sync(&client);
}
