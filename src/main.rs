// 在 Windows 上,防止 console 窗口在 release 模式弹出
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use rustvid::app::AppState;
use rustvid::commands;
use rustvid::config::Config;
use tauri::Manager;

fn main() {
    rustvid::app::setup_logging();
    run();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 同步阻塞初始化 AppState(setup 回调不能 async)
            let app_handle = app.handle().clone();
            let handle = tauri::async_runtime::handle();
            let state = handle.block_on(async {
                let config = Config::from_env()
                    .or_else(|_| Config::new(default_data_dir()))
                    .map_err(|e| anyhow::anyhow!("初始化配置失败:{e}"))?;
                AppState::new(config, app_handle).await
            })?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::uploads::create_upload,
            commands::presets::get_presets,
            commands::usage::get_usage,
            commands::jobs::list_jobs,
            commands::jobs::get_job,
            commands::jobs::create_job,
            commands::jobs::delete_job,
            commands::jobs::delete_failed_jobs,
            commands::jobs::retry_job,
            commands::jobs::process_next,
            commands::artifacts::get_artifact_preview_path,
            commands::artifacts::get_artifact_download_path,
            commands::artifacts::download_artifact,
            commands::artifacts::reveal_in_finder,
        ])
        .run(tauri::generate_context!())
        .expect("Tauri 启动失败");
}

/// Tauri 模式下数据目录:macOS `~/Library/Application Support/RustVid` 等
fn default_data_dir() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        std::path::PathBuf::from(home).join("Library/Application Support/RustVid")
    } else {
        std::path::PathBuf::from("./data")
    }
}
