use std::path::{Path, PathBuf};

use anyhow::Context;
use tokio::fs;
use uuid::Uuid;

use crate::{
    config::Config,
    domain::{preset::OutputTarget, storage::Artifact},
};

#[derive(Clone)]
pub struct ArtifactStore {
    config: Config,
}

impl ArtifactStore {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub async fn ensure_dirs(&self) -> anyhow::Result<()> {
        fs::create_dir_all(self.config.uploads_dir()).await?;
        fs::create_dir_all(self.config.artifacts_dir()).await?;
        fs::create_dir_all(self.config.work_dir()).await?;
        Ok(())
    }

    pub fn upload_path(&self, upload_id: &str) -> PathBuf {
        self.config.uploads_dir().join(upload_id)
    }

    pub fn artifact_dir(&self, job_id: &str) -> PathBuf {
        self.config.artifacts_dir().join(job_id)
    }

    pub fn work_dir(&self, job_id: &str) -> PathBuf {
        self.config.work_dir().join(job_id)
    }

    pub async fn create_artifact_record(
        &self,
        job_id: &str,
        target: OutputTarget,
        preview_path: PathBuf,
        download_path: PathBuf,
    ) -> anyhow::Result<Artifact> {
        let size_bytes = directory_size(&self.artifact_dir(job_id))
            .await
            .unwrap_or(0)
            + file_size_if_exists(&download_path).await.unwrap_or(0);
        Ok(Artifact {
            id: Uuid::new_v4().to_string(),
            job_id: job_id.to_string(),
            target,
            preview_path: path_to_string(preview_path)?,
            download_path: path_to_string(download_path)?,
            size_bytes,
        })
    }

    pub async fn delete_job_files(
        &self,
        job_id: &str,
        upload_path: Option<&str>,
    ) -> anyhow::Result<()> {
        remove_if_exists(self.artifact_dir(job_id)).await?;
        remove_if_exists(self.work_dir(job_id)).await?;
        if let Some(path) = upload_path {
            remove_if_exists(PathBuf::from(path)).await?;
        }
        Ok(())
    }
}

pub fn path_to_string(path: PathBuf) -> anyhow::Result<String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .context("路径包含无效字符")
}

async fn file_size_if_exists(path: &Path) -> anyhow::Result<u64> {
    Ok(fs::metadata(path).await.map(|m| m.len()).unwrap_or(0))
}

async fn directory_size(path: &Path) -> anyhow::Result<u64> {
    let mut total = 0;
    let mut entries = match fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(_) => return Ok(0),
    };
    while let Some(entry) = entries.next_entry().await? {
        let meta = entry.metadata().await?;
        if meta.is_file() {
            total += meta.len();
        }
    }
    Ok(total)
}

async fn remove_if_exists(path: PathBuf) -> anyhow::Result<()> {
    match fs::metadata(&path).await {
        Ok(meta) if meta.is_dir() => fs::remove_dir_all(path).await.context("删除目录失败")?,
        Ok(_) => fs::remove_file(path).await.context("删除文件失败")?,
        Err(_) => {}
    }
    Ok(())
}
