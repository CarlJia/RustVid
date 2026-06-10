mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::Value;
use tower::ServiceExt;

#[tokio::test]
async fn 默认_mp4_任务可以完成并提供产物() {
    let app = support::app_with_fake_transcoder(false).await;
    let router = support::router(app.state);
    let upload_id = upload_complete_file(router.clone()).await;

    let create_job = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/jobs")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    r#"{{"upload_id":"{upload_id}","preset":"blog"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_job.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(create_job.into_body(), usize::MAX)
        .await
        .unwrap();
    let job: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(job["target"], "mp4");

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

    let job_id = job["id"].as_str().unwrap();
    let get_job = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/jobs/{job_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(get_job.into_body(), usize::MAX)
        .await
        .unwrap();
    let job: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(job["status"], "completed");
    assert!(job["artifact_id"].as_str().is_some());
}

pub async fn upload_complete_file(router: axum::Router) -> String {
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
