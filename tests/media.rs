//! Media: upload encoding, download by id, metadata, deletion, and the limits
//! enforced before a request is built.

mod common;

use common::{client, json_response, response, Body, MockTransport};
use meta_whatsapp::media::{max_bytes_for_mime, MAX_IMAGE_BYTES};
use meta_whatsapp::{Error, MediaUpload};
use serde_json::json;

fn png_bytes() -> Vec<u8> {
    vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
}

#[tokio::test]
async fn upload_posts_multipart_and_returns_the_id() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({ "id": "media-123" })));
    let client = client(transport.clone());

    let upload = MediaUpload::new("cat.png", "image/png", png_bytes()).unwrap();
    let id = client.upload_media(&upload).await.unwrap();
    assert_eq!(id, "media-123");

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "POST");
    assert_eq!(
        call.url,
        "https://graph.facebook.com/v21.0/106540352242922/media"
    );

    match &call.body {
        Body::Multipart {
            content_type,
            bytes,
        } => {
            assert!(
                content_type.starts_with("multipart/form-data; boundary="),
                "content type was {content_type}"
            );
            let boundary = content_type
                .split("boundary=")
                .nth(1)
                .expect("boundary in content type");
            let text = String::from_utf8_lossy(bytes);
            assert!(text.contains(&format!("--{boundary}\r\n")));
            assert!(text.contains("name=\"messaging_product\"\r\n\r\nwhatsapp\r\n"));
            assert!(text.contains("name=\"file\"; filename=\"cat.png\""));
            assert!(text.contains("Content-Type: image/png"));
            assert!(text.ends_with(&format!("--{boundary}--\r\n")));
            assert!(
                bytes
                    .windows(png_bytes().len())
                    .any(|window| window == png_bytes()),
                "raw bytes must be part of the body"
            );
        }
        other => panic!("expected multipart, got {other:?}"),
    }
}

#[tokio::test]
async fn upload_without_an_id_in_the_response_is_a_payload_error() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({ "success": true })));
    let client = client(transport);
    let upload = MediaUpload::new("a.txt", "text/plain", b"hi".to_vec()).unwrap();

    let err = client.upload_media(&upload).await.unwrap_err();
    assert!(matches!(err, Error::Payload(_)));
}

#[tokio::test]
async fn media_info_gets_the_media_path_and_parses_the_body() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({
        "id": "42",
        "url": "https://lookaside.fbsbx.com/whatsapp/42",
        "mime_type": "image/png",
        "sha256": "abc",
        "file_size": 8,
    })));
    let client = client(transport.clone());

    let info = client.media_info("42").await.unwrap();
    assert_eq!(info.id, "42");
    assert_eq!(info.mime_type.as_deref(), Some("image/png"));
    assert_eq!(info.file_size, Some(8));

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "GET");
    assert_eq!(call.url, "https://graph.facebook.com/v21.0/42");
}

#[tokio::test]
async fn download_returns_bytes_and_content_type() {
    let transport = MockTransport::new();
    transport.push_response(response(
        200,
        &[("Content-Type", "image/png")],
        &png_bytes(),
    ));
    let client = client(transport.clone());

    let media = client.download_media("42").await.unwrap();
    assert_eq!(media.content_type(), Some("image/png"));
    assert_eq!(media.bytes(), png_bytes());

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "GET");
    assert_eq!(call.url, "https://graph.facebook.com/v21.0/42");
}

#[tokio::test]
async fn download_from_the_cdn_url_sends_the_token() {
    let transport = MockTransport::new();
    transport.push_response(response(200, &[], b"bytes"));
    let client = client(transport.clone());

    let bytes = client
        .download_from_url("https://lookaside.fbsbx.com/whatsapp/42")
        .await
        .unwrap();
    assert_eq!(bytes, b"bytes");
    assert_eq!(transport.only_call().token, "EAAG-test-token");
}

#[tokio::test]
async fn download_from_url_rejects_plain_http() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let err = client
        .download_from_url("http://lookaside.fbsbx.com/whatsapp/42")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Validation { .. }));
    assert_eq!(transport.call_count(), 0);
}

#[tokio::test]
async fn delete_media_uses_the_delete_verb() {
    let transport = MockTransport::new();
    transport.push_response(json_response(&json!({ "success": true })));
    let client = client(transport.clone());

    let value = client.delete_media("42").await.unwrap();
    assert_eq!(value["success"], true);

    let call = transport.only_call();
    assert_eq!(call.method.as_str(), "DELETE");
    assert_eq!(call.url, "https://graph.facebook.com/v21.0/42");
    assert_eq!(call.body, Body::Empty);
}

#[tokio::test]
async fn unsafe_media_ids_never_reach_the_wire() {
    for bad in [
        "",
        "../v21.0/me",
        "42/../me",
        "4 2",
        "42?fields=x",
        "42#f",
        &"9".repeat(257),
    ] {
        let transport = MockTransport::new();
        let client = client(transport.clone());

        for result in [
            client.media_info(bad).await.map(|_| ()),
            client.download_media(bad).await.map(|_| ()),
            client.delete_media(bad).await.map(|_| ()),
        ] {
            match result {
                Err(Error::Validation { field, .. }) => assert_eq!(field, "media_id"),
                other => panic!("id {bad:?} was not rejected: {other:?}"),
            }
        }
        assert_eq!(transport.call_count(), 0, "id {bad:?} produced a request");
    }
}

#[test]
fn upload_limits_are_per_type() {
    assert_eq!(max_bytes_for_mime("image/png"), Some(MAX_IMAGE_BYTES));

    // Exactly at the limit is fine.
    let at_limit = vec![0_u8; usize::try_from(MAX_IMAGE_BYTES).unwrap()];
    assert!(MediaUpload::new("ok.png", "image/png", at_limit).is_ok());

    // One byte over is not.
    let over = vec![0_u8; usize::try_from(MAX_IMAGE_BYTES + 1).unwrap()];
    let err = MediaUpload::new("big.png", "image/png", over).unwrap_err();
    assert!(matches!(
        err,
        Error::Validation {
            field: "media.data",
            ..
        }
    ));
}

#[test]
fn unsupported_types_and_empty_payloads_are_refused() {
    for (filename, mime, data, field) in [
        (
            "a.exe",
            "application/x-msdownload",
            b"x".to_vec(),
            "media.mime_type",
        ),
        ("", "image/png", b"x".to_vec(), "media.filename"),
        ("a.png", "image/png", Vec::new(), "media.data"),
        ("a\r\nX: y", "image/png", b"x".to_vec(), "media.filename"),
    ] {
        let err = MediaUpload::new(filename, mime, data).unwrap_err();
        match err {
            Error::Validation { field: actual, .. } => assert_eq!(actual, field),
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_hand_built_upload_is_validated_again_by_the_client() {
    let transport = MockTransport::new();
    let client = client(transport.clone());
    let upload = MediaUpload {
        data: vec![1, 2, 3],
        filename: "a.png".into(),
        mime_type: "application/octet-stream".into(),
    };

    let err = client.upload_media(&upload).await.unwrap_err();
    assert!(matches!(err, Error::Validation { .. }));
    assert_eq!(transport.call_count(), 0);
}

#[test]
fn media_ids_length_boundary() {
    use meta_whatsapp::media::{validate_media_id, MAX_MEDIA_ID_LEN};

    assert!(validate_media_id(&"a".repeat(MAX_MEDIA_ID_LEN)).is_ok());
    assert!(validate_media_id(&"a".repeat(MAX_MEDIA_ID_LEN + 1)).is_err());
}
