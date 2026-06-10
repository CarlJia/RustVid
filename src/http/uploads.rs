use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use http_body_util::BodyExt;
use serde::{Deserialize, Serialize};

use crate::{app::AppState, domain::upload::UploadSession};

#[derive(Debug, Deserialize)]
pub struct CreateUploadRequest {
    pub filename: String,
    pub length: u64,
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub id: String,
    pub filename: String,
    pub length: u64,
    pub offset: u64,
    pub status: String,
}

pub async fn create_upload(
    State(state): State<AppState>,
    Json(payload): Json<CreateUploadRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let session = state
        .uploads
        .create(payload.filename, payload.length)
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        HeaderValue::from_str(&format!("/api/uploads/{}", session.id)).unwrap(),
    );
    headers.insert("Upload-Offset", HeaderValue::from(session.offset));
    Ok((
        StatusCode::CREATED,
        headers,
        Json(UploadResponse::from(session)),
    ))
}

pub async fn head_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let session = state.uploads.get(&id)?;
    let mut headers = HeaderMap::new();
    headers.insert("Upload-Offset", HeaderValue::from(session.offset));
    headers.insert("Upload-Length", HeaderValue::from(session.length));
    Ok((StatusCode::NO_CONTENT, headers))
}

pub async fn patch_upload(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    mut body: Body,
) -> Result<impl IntoResponse, ApiError> {
    let mut expected = header_u64(&headers, "Upload-Offset")?;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type != "application/offset+octet-stream" {
        return Err(ApiError::status(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "请使用 TUS 上传内容类型",
        ));
    }
    let mut session = state.uploads.get(&id)?;
    while let Some(frame) = body.frame().await {
        let frame =
            frame.map_err(|_| ApiError::status(StatusCode::BAD_REQUEST, "读取上传内容失败"))?;
        if let Some(data) = frame.data_ref() {
            session = state.uploads.append_chunk(&id, expected, data).await?;
            expected = session.offset;
        }
    }
    let mut headers = HeaderMap::new();
    headers.insert("Upload-Offset", HeaderValue::from(session.offset));
    Ok((StatusCode::NO_CONTENT, headers))
}

fn header_u64(headers: &HeaderMap, name: &str) -> Result<u64, ApiError> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::status(StatusCode::BAD_REQUEST, "缺少有效上传偏移"))
}

impl From<UploadSession> for UploadResponse {
    fn from(session: UploadSession) -> Self {
        Self {
            id: session.id,
            filename: session.filename,
            length: session.length,
            offset: session.offset,
            status: session.status.as_str().to_string(),
        }
    }
}

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn status(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        let message = value.to_string();
        let status = if message.contains("偏移不一致") {
            StatusCode::CONFLICT
        } else if message.contains("2GB") || message.contains("200GB") || message.contains("上限")
        {
            StatusCode::PAYLOAD_TOO_LARGE
        } else if message.contains("不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::BAD_REQUEST
        };
        Self { status, message }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(Serialize)]
        struct ErrorBody {
            error: String,
        }
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}
