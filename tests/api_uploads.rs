mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn 支持按偏移断点续传并拒绝冲突偏移() {
    let app = support::app_with_fake_transcoder(false).await;
    let router = support::router(app.state);

    let create = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/uploads")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"filename":"demo.mp4","length":11}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload: Value = serde_json::from_slice(&body).unwrap();
    let id = upload["id"].as_str().unwrap();

    let first = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/uploads/{id}"))
                .header(header::CONTENT_TYPE, "application/offset+octet-stream")
                .header("Upload-Offset", "0")
                .body(Body::from("hello "))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::NO_CONTENT);
    assert_eq!(first.headers().get("Upload-Offset").unwrap(), "6");

    let conflict = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/uploads/{id}"))
                .header(header::CONTENT_TYPE, "application/offset+octet-stream")
                .header("Upload-Offset", "0")
                .body(Body::from("world"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let second = router
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/uploads/{id}"))
                .header(header::CONTENT_TYPE, "application/offset+octet-stream")
                .header("Upload-Offset", "6")
                .body(Body::from("world"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::NO_CONTENT);
    assert_eq!(second.headers().get("Upload-Offset").unwrap(), "11");
}
