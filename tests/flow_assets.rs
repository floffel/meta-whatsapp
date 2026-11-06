//! Flow JSON asset uploads: the documented multipart body, response
//! diagnostics, and every local validation boundary.

mod common;

use common::{client, instant_retry, json_response, response, Body, MockTransport};
use meta_whatsapp::flow::{
    FlowAssetUpload, FlowAssetUploadResponse, FLOW_ASSET_TYPE, FLOW_ID_MAX_BYTES,
    FLOW_JSON_FILENAME, FLOW_JSON_MAX_BYTES,
};
use meta_whatsapp::transport::Method;
use meta_whatsapp::Error;
use serde_json::json;

const FLOW_JSON: &[u8] = br#"{"version":"5.0","screens":[]}"#;

fn multipart(call: &common::Call) -> (&str, &[u8]) {
    match &call.body {
        Body::Bytes {
            content_type,
            bytes,
        } => (content_type, bytes),
        other => panic!("expected a pre-encoded bytes body, got {other:?}"),
    }
}

fn validation_field(error: Error) -> &'static str {
    match error {
        Error::Validation { field, .. } => field,
        other => panic!("expected a validation error, got {other:?}"),
    }
}

#[tokio::test]
async fn upload_posts_the_documented_multipart_body_to_the_flow_assets_endpoint() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "success": true,
        "validation_errors": []
    })));
    let client = client(transport.clone());
    let upload = FlowAssetUpload::new("1234567890", FLOW_JSON.to_vec()).unwrap();

    let result = client.upload_flow_asset(&upload).await.unwrap();
    assert!(result.success);
    assert!(result.is_valid());

    let call = transport.only_call();
    assert_eq!(call.method, Method::Post);
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/1234567890/assets"
    );
    assert_eq!(call.token, "EAAG-test-token");

    let (content_type, bytes) = multipart(&call);
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .expect("a multipart content type");
    let expected = [
        format!("--{boundary}\r\n"),
        format!(
            "Content-Disposition: form-data; name=\"asset_type\"\r\n\r\n{FLOW_ASSET_TYPE}\r\n"
        ),
        format!("--{boundary}\r\n"),
        format!(
            "Content-Disposition: form-data; name=\"name\"\r\n\r\n{FLOW_JSON_FILENAME}\r\n"
        ),
        format!("--{boundary}\r\n"),
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{FLOW_JSON_FILENAME}\"\r\nContent-Type: application/json\r\n\r\n"
        ),
    ]
    .concat()
    .into_bytes();
    let expected_len = expected.len() + FLOW_JSON.len() + boundary.len() + 8;
    assert_eq!(bytes.len(), expected_len, "no hidden multipart fields");
    assert_eq!(&bytes[..expected.len()], expected);
    assert_eq!(
        &bytes[expected.len()..expected.len() + FLOW_JSON.len()],
        FLOW_JSON
    );
    assert_eq!(
        &bytes[expected.len() + FLOW_JSON.len()..],
        format!("\r\n--{boundary}--\r\n").as_bytes()
    );
}

#[tokio::test]
async fn validation_errors_are_typed_and_do_not_make_a_successful_upload_valid() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "success": true,
        "validation_errors": [{
            "error": "INVALID_PROPERTY_VALUE",
            "error_type": "FLOW_JSON_ERROR",
            "message": "Invalid value found for property 'type'.",
            "line_start": 10,
            "line_end": 10,
            "column_start": 21,
            "column_end": 34,
            "pointers": [{
                "line_start": 10,
                "line_end": 10,
                "column_start": 21,
                "column_end": 34,
                "path": "screens [0]. layout.children [0].type"
            }],
            "future": "kept"
        }],
        "future": {"a": 1}
    })));

    let result = client(transport)
        .upload_flow_asset(&FlowAssetUpload::new("flow-1", FLOW_JSON.to_vec()).unwrap())
        .await
        .unwrap();
    assert!(result.success);
    assert!(!result.is_valid());
    assert_eq!(result.validation_errors.len(), 1);
    let error = &result.validation_errors[0];
    assert_eq!(error.line_start, Some(10));
    assert_eq!(
        error.pointers[0].path.as_deref(),
        Some("screens [0]. layout.children [0].type")
    );
    assert_eq!(error.extra["future"], "kept");
    assert_eq!(result.extra["future"]["a"], 1);
}

#[test]
fn response_decoding_is_permissive_and_fails_only_for_a_non_object() {
    let empty = FlowAssetUploadResponse::from_value(&json!({})).unwrap();
    assert!(!empty.success);
    assert!(!empty.is_valid());

    let diagnostics: FlowAssetUploadResponse =
        serde_json::from_str(r#"{"success":true,"validation_errors":[{"future":1}],"future":2}"#)
            .unwrap();
    assert_eq!(diagnostics.validation_errors[0].extra["future"], 1);
    assert_eq!(diagnostics.extra["future"], 2);

    for bad in [json!([]), json!(null), json!("ok")] {
        assert!(matches!(
            FlowAssetUploadResponse::from_value(&bad),
            Err(Error::Json { .. })
        ));
    }
}

#[tokio::test]
async fn invalid_uploads_never_reach_the_transport() {
    let cases = vec![
        (String::new(), FLOW_JSON.to_vec(), "flow_id"),
        ("../me".to_string(), FLOW_JSON.to_vec(), "flow_id"),
        ("flow/1".to_string(), FLOW_JSON.to_vec(), "flow_id"),
        ("flow 1".to_string(), FLOW_JSON.to_vec(), "flow_id"),
        (
            "a".repeat(FLOW_ID_MAX_BYTES + 1),
            FLOW_JSON.to_vec(),
            "flow_id",
        ),
        ("flow-1".to_string(), Vec::new(), "flow_json"),
    ];
    for (flow_id, payload, field) in cases {
        let transport = MockTransport::new();
        let error = FlowAssetUpload::new(flow_id.clone(), payload).unwrap_err();
        assert_eq!(validation_field(error), field, "flow id {flow_id:?}");
        assert_eq!(transport.call_count(), 0);
    }
}

#[test]
fn typed_upload_getters_and_validation_are_consistent() {
    let upload = FlowAssetUpload::new("flow-1", FLOW_JSON.to_vec()).unwrap();
    assert_eq!(upload.flow_id(), "flow-1");
    assert_eq!(upload.flow_json(), FLOW_JSON);
    assert_eq!(upload.len(), FLOW_JSON.len());
    assert!(!upload.is_empty());
    assert!(upload.validate().is_ok());
}

#[test]
fn flow_id_accepts_exactly_256_bytes_and_rejects_257() {
    let at_limit = "a".repeat(FLOW_ID_MAX_BYTES);
    assert!(FlowAssetUpload::new(at_limit, FLOW_JSON.to_vec()).is_ok());
    let over_limit = "a".repeat(FLOW_ID_MAX_BYTES + 1);
    assert_eq!(
        validation_field(FlowAssetUpload::new(over_limit, FLOW_JSON.to_vec()).unwrap_err()),
        "flow_id"
    );
}

#[test]
fn flow_json_accepts_exactly_ten_mib_and_rejects_one_byte_more() {
    let at_limit = FlowAssetUpload::new("flow-1", vec![b' '; FLOW_JSON_MAX_BYTES]).unwrap();
    assert_eq!(at_limit.len(), FLOW_JSON_MAX_BYTES);
    drop(at_limit);

    let error = FlowAssetUpload::new("flow-1", vec![b' '; FLOW_JSON_MAX_BYTES + 1]).unwrap_err();
    assert_eq!(validation_field(error), "flow_json");
}

#[test]
fn flow_json_is_retained_as_bytes_so_non_utf8_payloads_are_not_mangled() {
    let bytes = vec![0xff, 0x00, b'{', b'}'];
    let upload = FlowAssetUpload::new("flow-1", bytes.clone()).unwrap();
    assert_eq!(upload.flow_json(), bytes);
}

#[tokio::test]
async fn a_5xx_upload_is_ambiguous_and_not_retried() {
    let transport = MockTransport::new();
    for _ in 0..3 {
        transport.push_response(response(
            500,
            &[],
            b"{\"error\":{\"code\":2,\"message\":\"internal\"}}",
        ));
    }
    let client = client(transport.clone()).with_retry(instant_retry(3));
    let upload = FlowAssetUpload::new("flow-1", FLOW_JSON.to_vec()).unwrap();

    let error = client.upload_flow_asset(&upload).await.unwrap_err();
    assert!(error.is_ambiguous(), "{error:?}");
    assert!(!error.is_retryable());
    assert_eq!(transport.call_count(), 1);
}

#[tokio::test]
async fn a_rate_limited_upload_is_retried_before_the_write() {
    let transport = MockTransport::new();
    transport.push_response(response(
        429,
        &[("Retry-After", "0")],
        b"{\"error\":{\"code\":4,\"message\":\"slow down\"}}",
    ));
    transport.push_response(json_response(&json!({
        "success": true,
        "validation_errors": []
    })));
    let client = client(transport.clone()).with_retry(instant_retry(2));
    let upload = FlowAssetUpload::new("flow-1", FLOW_JSON.to_vec()).unwrap();

    let result = client.upload_flow_asset(&upload).await.unwrap();
    assert!(result.is_valid());
    assert_eq!(transport.call_count(), 2);
    assert_eq!(transport.calls()[0].url, transport.calls()[1].url);
}

#[tokio::test]
async fn a_malformed_success_body_is_a_json_error() {
    let transport = MockTransport::new();
    transport.push_response(response(
        200,
        &[("Content-Type", "application/json")],
        b"[]",
    ));
    let upload = FlowAssetUpload::new("flow-1", FLOW_JSON.to_vec()).unwrap();

    assert!(matches!(
        client(transport).upload_flow_asset(&upload).await,
        Err(Error::Json { .. })
    ));
}
