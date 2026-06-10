use axum::{
    body::Body,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use tokio::fs;

use crate::{app::AppState, http::uploads::ApiError};

pub async fn preview(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> Result<Response, ApiError> {
    let artifact = state
        .db
        .get_artifact(&artifact_id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "产物不存在"))?;
    serve_file(&artifact.preview_path, false).await
}

pub async fn download(
    State(state): State<AppState>,
    Path(artifact_id): Path<String>,
) -> Result<Response, ApiError> {
    let artifact = state
        .db
        .get_artifact(&artifact_id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "产物不存在"))?;
    serve_file(&artifact.download_path, true).await
}

pub async fn hls_file(
    State(state): State<AppState>,
    Path((artifact_id, file)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let artifact = state
        .db
        .get_artifact(&artifact_id)?
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "产物不存在"))?;
    let base = std::path::Path::new(&artifact.preview_path)
        .parent()
        .ok_or_else(|| ApiError::status(StatusCode::NOT_FOUND, "HLS 目录不存在"))?;
    let path = base.join("files").join(file);
    serve_file(path.to_str().unwrap_or_default(), false).await
}

async fn serve_file(path: &str, download: bool) -> Result<Response, ApiError> {
    let bytes = fs::read(path)
        .await
        .map_err(|_| ApiError::status(StatusCode::NOT_FOUND, "文件不存在"))?;
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let mut response = Body::from(bytes).into_response();
    let headers = response.headers_mut();
    if download {
        headers.insert(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{filename}\"")
                .parse()
                .unwrap(),
        );
    }
    if filename.ends_with(".m3u8") {
        headers.insert(
            header::CONTENT_TYPE,
            "application/vnd.apple.mpegurl".parse().unwrap(),
        );
    } else if filename.ends_with(".mp4") {
        headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
    } else if filename.ends_with(".zip") {
        headers.insert(header::CONTENT_TYPE, "application/zip".parse().unwrap());
    } else if filename.ends_with(".ts") {
        headers.insert(header::CONTENT_TYPE, "video/mp2t".parse().unwrap());
    }
    Ok(response)
}
