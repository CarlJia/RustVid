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
    assert!(artifact.download_path.ends_with("hls-package.zip"));
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
    };
    app.state.db.insert_job(&job).unwrap();

    assert!(app.state.queue.process_one().await.is_err());
    let failed = app.state.db.get_job(&job.id).unwrap().unwrap();
    assert_eq!(failed.status, JobStatus::Failed);
    assert!(failed.error_summary.unwrap().contains("视频转码失败"));
}
