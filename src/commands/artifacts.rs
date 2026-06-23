use std::path::PathBuf;
use tauri::State;

use crate::app::AppState;

/// 返回本地预览文件的绝对路径,前端用 `convertFileSrc()` 转 webview 可访问的 URL
#[tauri::command]
pub fn get_artifact_preview_path(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let artifact = state
        .db
        .get_artifact(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "产物不存在".to_string())?;
    Ok(artifact.preview_path)
}

/// 返回本地下载文件(MP4 或 HLS zip)的绝对路径
#[tauri::command]
pub fn get_artifact_download_path(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let artifact = state
        .db
        .get_artifact(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "产物不存在".to_string())?;
    Ok(artifact.download_path)
}

/// 下载产物到用户指定位置(由前端 save dialog 选)。
/// 用 Rust 复制而不是前端 fs.copyFile,避免 fs scope 限制。
#[tauri::command]
pub async fn download_artifact(
    state: State<'_, AppState>,
    id: String,
    dest: String,
) -> Result<String, String> {
    let artifact = state
        .db
        .get_artifact(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "产物不存在".to_string())?;
    let src = std::path::PathBuf::from(&artifact.download_path);
    let dst = PathBuf::from(&dest);
    // 父目录不存在就建(用户可能输了不存在的目录)
    if let Some(parent) = dst.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("创建目标目录失败:{e}"))?;
    }
    tokio::fs::copy(&src, &dst)
        .await
        .map_err(|e| format!("复制失败({} → {}):{e}", src.display(), dst.display()))?;
    tracing::info!(%id, src = %src.display(), dst = %dst.display(), "产物已下载");
    Ok(dst.display().to_string())
}

/// 在 Finder/Explorer 中显示产物文件
#[tauri::command]
pub fn reveal_in_finder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // Linux:xdg-open 打开父目录
        let parent = std::path::Path::new(&path)
            .parent()
            .ok_or_else(|| "无法取父目录".to_string())?;
        std::process::Command::new("xdg-open")
            .arg(parent)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
