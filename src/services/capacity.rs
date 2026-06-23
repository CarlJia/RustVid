use std::path::Path;

use fs2::free_space;

use crate::{config::Config, persistence::sqlite::Database};

#[derive(Clone)]
pub struct CapacityService {
    config: Config,
    db: Database,
}

impl CapacityService {
    pub fn new(config: Config, db: Database) -> Self {
        Self { config, db }
    }

    pub fn ensure_can_upload(&self, length: u64) -> anyhow::Result<()> {
        // 单文件上限(2GB)—— 防止单文件过大导致内存/磁盘压力
        if length > self.config.max_file_size {
            anyhow::bail!("单个视频不能超过 2GB");
        }
        // 不再做总容量封顶:展示系统磁盘剩余空间即可,由用户自行决定何时清理
        Ok(())
    }

    pub fn ensure_can_start_transcode(&self) -> anyhow::Result<()> {
        let running = self.db.count_processing_jobs()?;
        if running >= self.config.max_concurrent_transcodes {
            anyhow::bail!("同时转码任务已达到上限");
        }
        Ok(())
    }

    pub fn usage(&self) -> anyhow::Result<StorageUsage> {
        let (disk_free_bytes, disk_total_bytes) = disk_space(&self.config.data_dir)?;
        Ok(StorageUsage {
            used_bytes: self.db.total_artifact_bytes()?,
            disk_free_bytes,
            disk_total_bytes,
        })
    }
}

/// 拿 `path` 所在文件系统的剩余空间和总容量。
///
/// 任何错误(路径消失、权限不足等)都返回 Ok((0, 0))——前端能展示"系统磁盘剩余空间暂不可用"
/// 而不至于让整个 `get_usage` 命令崩。
fn disk_space(path: &Path) -> anyhow::Result<(u64, u64)> {
    match (free_space(path), fs2::total_space(path)) {
        (Ok(free), Ok(total)) => Ok((free, total)),
        // 半失败(比如某些容器 fs 不支持 total)给个回退:total 用 used + free 估算
        (Ok(free), Err(_)) => Ok((free, free)),
        (Err(_), Ok(total)) => Ok((0, total)),
        (Err(_), Err(_)) => Ok((0, 0)),
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StorageUsage {
    /// RustVid 已用字节数(uploads + artifacts)
    pub used_bytes: u64,
    /// 系统磁盘剩余可用字节数(data_dir 所在文件系统)
    pub disk_free_bytes: u64,
    /// 系统磁盘总容量字节数(供前端进度条展示)
    pub disk_total_bytes: u64,
}
