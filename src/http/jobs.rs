use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    app::AppState,
    domain::{
        job::{JobStatus, VideoJob},
        preset::{OutputTarget, PresetId, presets as preset_catalog},
    },
    http::uploads::ApiError,
};

#[derive(Debug, Deserialize)]
pub struct CreateJobRequest {
    pub upload_id: String,
    pub preset: PresetId,
    #[serde(default = "default_target")]
    pub target: OutputTarget,
}

fn default_target() -> OutputTarget {
    OutputTarget::Mp4
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub id: String,
    pub upload_id: String,
    pub preset: PresetId,
    pub target: OutputTarget,
    pub status: JobStatus,
    pub error_summary: Option<String>,
    pub artifact_id: Option<String>,
}

pub async fn presets() -> impl IntoResponse {
    Json(preset_catalog())
}

pub async fn usage(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    Ok(Json(state.capacity.usage()?))
}

pub async fn list_jobs(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let jobs = state
        .db
        .list_jobs()?
        .into_iter()
        .map(JobResponse::from)
        .collect::<Vec<_>>();
    Ok(Json(jobs))
}

pub async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .db
        .get_job(&id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "任务不存在"))?;
    Ok(Json(JobResponse::from(job)))
}

pub async fn create_job(
    State(state): State<AppState>,
    Json(payload): Json<CreateJobRequest>,
) -> Result<impl IntoResponse, ApiError> {
    state.capacity.ensure_can_start_transcode()?;
    let upload = state
        .db
        .get_upload(&payload.upload_id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "上传文件不存在"))?;
    if upload.status != crate::domain::upload::UploadStatus::Uploaded {
        return Err(ApiError::status(
            StatusCode::BAD_REQUEST,
            "上传完成后才能创建转码任务",
        ));
    }
    let job = VideoJob {
        id: Uuid::new_v4().to_string(),
        upload_id: payload.upload_id,
        preset: payload.preset,
        target: payload.target,
        status: JobStatus::Queued,
        error_summary: None,
        artifact_id: None,
    };
    state.db.insert_job(&job)?;
    Ok((StatusCode::CREATED, Json(JobResponse::from(job))))
}

pub async fn retry_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state.db.transition_job(&id, JobStatus::Queued, None)?;
    get_job(State(state), Path(id)).await
}

pub async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let job = state
        .db
        .get_job(&id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "任务不存在"))?;
    let upload = state.db.get_upload(&job.upload_id)?;
    state
        .artifacts
        .delete_job_files(&job.id, upload.as_ref().map(|upload| upload.path.as_str()))
        .await?;
    state.db.mark_deleted(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn process_next(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let processed = state.queue.process_one().await?;
    Ok(Json(serde_json::json!({ "processed": processed })))
}

impl From<VideoJob> for JobResponse {
    fn from(job: VideoJob) -> Self {
        Self {
            id: job.id,
            upload_id: job.upload_id,
            preset: job.preset,
            target: job.target,
            status: job.status,
            error_summary: job.error_summary,
            artifact_id: job.artifact_id,
        }
    }
}
