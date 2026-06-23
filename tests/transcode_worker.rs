mod support;

use rustvid::{
    domain::{
        job::{JobStatus, VideoJob},
        preset::{OutputTarget, PresetId},
        upload::{UploadSession, UploadStatus},
    },
    services::artifact_store::path_to_string,
};
use uuid::Uuid;

#[tokio::test]
async fn hls_任务完成后记录_zip_下载产物() {
    let app = support::app_with_fake_transcoder(false).await;
    let upload_path = app.state.config.uploads_dir().join("video");
    tokio::fs::write(&upload_path, b"demo").await.unwrap();
    let upload = UploadSession {
        id: Uuid::new_v4().to_string(),
        filename: "demo.mp4".to_string(),
        length: 4,
        offset: 4,
        path: path_to_string(upload_path).unwrap(),
        status: UploadStatus::Uploaded,
    };
    app.state.db.insert_upload(&upload).unwrap();
    let job = VideoJob {
        id: Uuid::new_v4().to_string(),
        upload_id: upload.id,
        preset: PresetId::Course,
        target: OutputTarget::Hls,
        status: JobStatus::Queued,
        error_summary: None,
        artifact_id: None,
        source_duration_secs: None,
        created_at: String::new(),
    };
    app.state.db.insert_job(&job).unwrap();

    app.state.queue.process_one().await.unwrap();

    let completed = app.state.db.get_job(&job.id).unwrap().unwrap();
    assert_eq!(completed.status, JobStatus::Completed);
    let artifact = app
        .state
        .db
        .get_artifact(completed.artifact_id.as_ref().unwrap())
        .unwrap()
        .unwrap();
    assert!(artifact.download_path.ends_with(".zip"));
}

#[tokio::test]
async fn 转码失败会保存可重试状态() {
    let app = support::app_with_fake_transcoder(true).await;
    let upload_path = app.state.config.uploads_dir().join("video");
    tokio::fs::write(&upload_path, b"demo").await.unwrap();
    let upload = UploadSession {
        id: Uuid::new_v4().to_string(),
        filename: "demo.mp4".to_string(),
        length: 4,
        offset: 4,
        path: path_to_string(upload_path).unwrap(),
        status: UploadStatus::Uploaded,
    };
    app.state.db.insert_upload(&upload).unwrap();
    let job = VideoJob {
        id: Uuid::new_v4().to_string(),
        upload_id: upload.id,
        preset: PresetId::Blog,
        target: OutputTarget::Mp4,
        status: JobStatus::Queued,
        error_summary: None,
        artifact_id: None,
        source_duration_secs: None,
        created_at: String::new(),
    };
    app.state.db.insert_job(&job).unwrap();

    assert!(app.state.queue.process_one().await.is_err());
    let failed = app.state.db.get_job(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(failed.error_summary.unwrap().contains("FFmpeg"));
}

#[tokio::test]
async fn 启动时回收僵尸_processing_任务() {
    let app = support::app_with_fake_transcoder(false).await;
    let upload_path = app.state.config.uploads_dir().join("video");
    tokio::fs::write(&upload_path, b"demo").await.unwrap();
    let upload = UploadSession {
        id: Uuid::new_v4().to_string(),
        filename: "demo.mp4".to_string(),
        length: 4,
        offset: 4,
        path: path_to_string(upload_path).unwrap(),
        status: UploadStatus::Uploaded,
    };
    app.state.db.insert_upload(&upload).unwrap();
    let job = VideoJob {
        id: Uuid::new_v4().to_string(),
        upload_id: upload.id,
        preset: PresetId::Blog,
        target: OutputTarget::Mp4,
        status: JobStatus::Processing, // 模拟上次崩溃留下的僵尸状态
        error_summary: None,
        artifact_id: None,
        source_duration_secs: None,
        created_at: String::new(),
    };
    app.state.db.insert_job(&job).unwrap();

    // 启动时 reaper 应把 Processing → Failed
    let reaped = app.state.db.reap_stale_processing().unwrap();
    assert_eq!(reaped.len(), 1);
    assert_eq!(reaped[0], job.id);

    let after = app.state.db.get_job(&job.id).unwrap().unwrap();
    assert_eq!(after.status, JobStatus::Failed);
    assert!(after
        .error_summary
        .as_deref()
        .unwrap()
        .contains("应用重启"));
}

#[tokio::test]
async fn 没有僵尸任务时_reaper_返回空() {
    let app = support::app_with_fake_transcoder(false).await;
    // 空库直接 reaper
    let reaped = app.state.db.reap_stale_processing().unwrap();
    assert!(reaped.is_empty());
}

#[tokio::test]
async fn list_failed_jobs_只返回_failed_状态() {
    let app = support::app_with_fake_transcoder(false).await;
    let upload_path = app.state.config.uploads_dir().join("video");
    tokio::fs::write(&upload_path, b"demo").await.unwrap();
    let upload = UploadSession {
        id: Uuid::new_v4().to_string(),
        filename: "demo.mp4".to_string(),
        length: 4,
        offset: 4,
        path: path_to_string(upload_path).unwrap(),
        status: UploadStatus::Uploaded,
    };
    app.state.db.insert_upload(&upload).unwrap();

    // 3 个不同状态的任务
    for status in [
        JobStatus::Failed,
        JobStatus::Queued,
        JobStatus::Completed,
    ] {
        let job = VideoJob {
            id: Uuid::new_v4().to_string(),
            upload_id: upload.id.clone(),
            preset: PresetId::Blog,
            target: OutputTarget::Mp4,
            status,
            error_summary: None,
            artifact_id: None,
            source_duration_secs: None,
            created_at: String::new(),
        };
        app.state.db.insert_job(&job).unwrap();
    }

    let failed = app.state.db.list_failed_jobs().unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].status, JobStatus::Failed);
}

#[tokio::test]
async fn hard_delete_job_真删行() {
    let app = support::app_with_fake_transcoder(false).await;
    let upload_path = app.state.config.uploads_dir().join("video");
    tokio::fs::write(&upload_path, b"demo").await.unwrap();
    let upload = UploadSession {
        id: Uuid::new_v4().to_string(),
        filename: "demo.mp4".to_string(),
        length: 4,
        offset: 4,
        path: path_to_string(upload_path).unwrap(),
        status: UploadStatus::Uploaded,
    };
    app.state.db.insert_upload(&upload).unwrap();
    let job = VideoJob {
        id: Uuid::new_v4().to_string(),
        upload_id: upload.id,
        preset: PresetId::Blog,
        target: OutputTarget::Mp4,
        status: JobStatus::Failed,
        error_summary: Some("测试失败".to_string()),
        artifact_id: None,
        source_duration_secs: None,
        created_at: String::new(),
    };
    app.state.db.insert_job(&job).unwrap();

    // 存在
    assert!(app.state.db.get_job(&job.id).unwrap().is_some());
    // 真删
    let n = app.state.db.hard_delete_job(&job.id).unwrap();
    assert_eq!(n, 1);
    // 没了
    assert!(app.state.db.get_job(&job.id).unwrap().is_none());
    // 多次删 idempotent(返回 0)
    assert_eq!(app.state.db.hard_delete_job(&job.id).unwrap(), 0);
}
