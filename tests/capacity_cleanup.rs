mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn 删除历史任务后任务列表不再展示该任务() {
    let app = support::app_with_fake_transcoder(false).await;
    let router = support::router(app.state);

    let upload_id = create_upload(router.clone()).await;
    let job = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"upload_id":"{upload_id}","preset":"blog","target":"mp4"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(job.into_body(), usize::MAX)
        .await
        .unwrap();
    let job: Value = serde_json::from_slice(&body).unwrap();
    let job_id = job["id"].as_str().unwrap();

    let processed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/jobs/process-next")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(processed.status(), StatusCode::OK);

    let deleted = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/jobs/{job_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let list = router
        .oneshot(
            Request::builder()
                .uri("/api/jobs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(list.into_body(), usize::MAX)
        .await
        .unwrap();
    let jobs: Value = serde_json::from_slice(&body).unwrap();
    assert!(jobs.as_array().unwrap().is_empty());
}

async fn create_upload(router: axum::Router) -> String {
    let create = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/uploads")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"filename":"demo.mp4","length":4}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(create.into_body(), usize::MAX)
        .await
        .unwrap();
    let upload: Value = serde_json::from_slice(&body).unwrap();
    let id = upload["id"].as_str().unwrap().to_string();
    let patch = router
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(format!("/api/uploads/{id}"))
                .header(header::CONTENT_TYPE, "application/offset+octet-stream")
                .header("Upload-Offset", "0")
                .body(Body::from("demo"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(patch.status(), StatusCode::NO_CONTENT);
    id
}
