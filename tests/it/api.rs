//! What the API answers when it refuses a request.
// Case tables are tuples on purpose: one row per case, read at a glance.
#![allow(clippy::type_complexity)]

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use openmod::config::Config;
use openmod::engine::Engine;
use openmod::keys::{KeySet, LiveKeys};
use openmod::server::{App, api_router};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const KEY: &str = "test-key-0123456789abcdefghij";

/// A server with no model file to download: QR codes for images, the rules
/// for texts, and request bodies capped at 4,096 bytes.
fn api() -> Router {
    let config = Config::parse(
        r#"
[server]
max_upload_bytes = 4096

[models.qr]
kind = "qr_reader"

[workflows.default.image]
qr_code = { review = 1.0 }

[workflows.default.text]
pii_phone = { review = 1.0 }
"#,
        Path::new("."),
    )
    .unwrap();
    let state = std::env::temp_dir().join(format!("openmod-api-{}.toml", std::process::id()));
    let keys = LiveKeys::load(KeySet::from_lines([KEY].into_iter()).unwrap(), &state).unwrap();
    let engine = Engine::load(config, None);
    api_router(Arc::new(App::new(
        engine,
        keys,
        PathBuf::from("openmod.toml"),
    )))
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image::RgbImage::new(width, height)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[tokio::test]
async fn a_refused_request_gets_the_error_envelope_and_a_true_reason() {
    let api = api();
    let long_text = format!(r#"{{"text": "{}"}}"#, "a".repeat(5_000));
    let json = Some("application/json");

    // (case, path, with the key, content type, body, status, error type, in the message)
    let cases: Vec<(&str, &str, bool, Option<&str>, Vec<u8>, u16, &str, &str)> = vec![
        (
            "a text without a key is refused before its body is read",
            "/v1/text",
            false,
            json,
            b"{".to_vec(),
            401,
            "unauthorized",
            "",
        ),
        (
            "a text sent without the JSON content type",
            "/v1/text",
            true,
            None,
            br#"{"text": "hello"}"#.to_vec(),
            415,
            "unsupported_media",
            "application/json",
        ),
        (
            "a text body that is not JSON",
            "/v1/text",
            true,
            json,
            b"{".to_vec(),
            400,
            "bad_request",
            "JSON",
        ),
        (
            "a JSON body without a text",
            "/v1/text",
            true,
            json,
            br#"{"workflow": "default"}"#.to_vec(),
            400,
            "bad_request",
            "text",
        ),
        (
            "a text body over max_upload_bytes",
            "/v1/text",
            true,
            json,
            long_text.into_bytes(),
            413,
            "body_too_large",
            "4096 bytes",
        ),
        (
            "an image body over max_upload_bytes",
            "/v1/image",
            true,
            None,
            vec![0; 5_000],
            413,
            "body_too_large",
            "4096 bytes",
        ),
        (
            "an image wider than the decoder takes",
            "/v1/image",
            true,
            None,
            png(16_385, 1),
            413,
            "image_too_large",
            "16384 pixels a side",
        ),
        (
            "a SightEngine form that is not multipart",
            "/1.0/check-workflow.json",
            true,
            json,
            b"{}".to_vec(),
            400,
            "bad_request",
            "",
        ),
    ];

    let mut failures = Vec::new();
    for (case, path, with_key, content_type, body, status, kind, message) in cases {
        let mut request = Request::post(path);
        if with_key {
            request = request.header("authorization", format!("Bearer {KEY}"));
        }
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let response = api
            .clone()
            .oneshot(request.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let got_status = response.status().as_u16();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).to_string();
        let Ok(envelope) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            failures.push(format!("{case}: {got_status}, not the envelope: {text}"));
            continue;
        };
        let error = &envelope["error"];
        let ok = got_status == status
            && envelope["status"] == "failure"
            && error["type"] == kind
            && error["code"] == status
            && error["message"]
                .as_str()
                .is_some_and(|m| m.contains(message));
        if !ok {
            failures.push(format!(
                "{case}: want {status} {kind} with {message:?}, got {got_status} {text}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[tokio::test]
async fn a_check_keeps_its_place_until_the_work_ends_even_if_the_caller_leaves() {
    use openmod::server::run_check;
    use std::time::Duration;

    let config = Config::parse(
        r#"
[server]
max_concurrent = 1

[workflows.default.text]
pii_phone = { review = 1.0 }
"#,
        Path::new("."),
    )
    .unwrap();
    let state = std::env::temp_dir().join(format!("openmod-permit-{}.toml", std::process::id()));
    let keys = LiveKeys::load(KeySet::from_lines([KEY].into_iter()).unwrap(), &state).unwrap();
    let app = Arc::new(App::new(
        Engine::load(config, None),
        keys,
        PathBuf::from("openmod.toml"),
    ));

    // The first check blocks until told to go on; its caller gives up at once.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    let first = run_check(&app, move |engine| {
        let _ = wait.recv();
        engine.check_text("hello", "default")
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(100), first)
            .await
            .is_err(),
        "the first check should still be running when its caller leaves"
    );

    // While that work runs, the only place stays taken.
    let second = run_check(&app, |engine| engine.check_text("hello", "default"));
    assert!(
        tokio::time::timeout(Duration::from_millis(300), second)
            .await
            .is_err(),
        "a second check got in while the first one still ran"
    );

    // When the work ends, the place is free again.
    go.send(()).unwrap();
    let third = run_check(&app, |engine| engine.check_text("hello", "default"));
    assert!(
        tokio::time::timeout(Duration::from_secs(5), third)
            .await
            .expect("the place frees when the work ends")
            .is_ok()
    );
}
