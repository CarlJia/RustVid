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
        if length > self.config.max_file_size {
            anyhow::bail!("单个视频不能超过 2GB");
        }
        let projected = self.db.total_artifact_bytes()?.saturating_add(length);
        if projected > self.config.max_total_storage {
            anyhow::bail!("存储容量已达到 200GB 上限，请先删除历史任务");
        }
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
        Ok(StorageUsage {
            used_bytes: self.db.total_artifact_bytes()?,
            max_bytes: self.config.max_total_storage,
        })
    }
}

#[derive(Debug, serde::Serialize)]
pub struct StorageUsage {
    pub used_bytes: u64,
    pub max_bytes: u64,
}
