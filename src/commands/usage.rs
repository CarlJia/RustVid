use serde::Serialize;
use tauri::State;

use crate::{app::AppState, services::capacity::StorageUsage};

#[tauri::command]
pub fn get_usage(state: State<'_, AppState>) -> Result<UsageDto, String> {
    state
        .capacity
        .usage()
        .map(UsageDto::from)
        .map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct UsageDto {
    /// RustVid 已用字节数(uploads + artifacts)
    pub used_bytes: u64,
    /// 系统磁盘剩余可用字节数(data_dir 所在文件系统)
    pub disk_free_bytes: u64,
    /// 系统磁盘总容量字节数(供进度条展示)
    pub disk_total_bytes: u64,
}

impl From<StorageUsage> for UsageDto {
    fn from(u: StorageUsage) -> Self {
        Self {
            used_bytes: u.used_bytes,
            disk_free_bytes: u.disk_free_bytes,
            disk_total_bytes: u.disk_total_bytes,
        }
    }
}
