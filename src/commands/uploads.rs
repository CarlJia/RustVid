use serde::Serialize;
use tauri::State;

use crate::app::AppState;

#[derive(Debug, Serialize)]
pub struct UploadInfo {
    pub id: String,
    pub filename: String,
    pub length: u64,
    /// 始终等于 length —— Tauri 模式下后端一次性 copy 完成,没有"分片上传中"状态
    pub offset: u64,
    pub status: String,
}

/// 从用户选中的本地文件创建上传会话(后端立即复制到 uploads 目录)
#[tauri::command]
pub async fn create_upload(
    state: State<'_, AppState>,
    filename: String,
    path: String,
) -> Result<UploadInfo, String> {
    state
        .uploads
        .create_from_path(&filename, &path)
        .await
        .map_err(|e| e.to_string())
        .map(|s| UploadInfo {
            id: s.id,
            filename: s.filename,
            length: s.length,
            offset: s.offset,
            status: s.status.as_str().to_string(),
        })
}
