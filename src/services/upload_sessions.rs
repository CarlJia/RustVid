use std::path::PathBuf;

use anyhow::Context;
use tokio::{
    fs::{self, OpenOptions},
    io::{AsyncSeekExt, AsyncWriteExt, SeekFrom},
};
use uuid::Uuid;

use crate::{
    domain::upload::{UploadSession, UploadStatus},
    persistence::sqlite::Database,
    services::{artifact_store::path_to_string, capacity::CapacityService},
};

#[derive(Clone)]
pub struct UploadService {
    db: Database,
    capacity: CapacityService,
    uploads_dir: PathBuf,
}

impl UploadService {
    pub fn new(db: Database, capacity: CapacityService, uploads_dir: PathBuf) -> Self {
        Self {
            db,
            capacity,
            uploads_dir,
        }
    }

    pub async fn create(&self, filename: String, length: u64) -> anyhow::Result<UploadSession> {
        self.capacity.ensure_can_upload(length)?;
        fs::create_dir_all(&self.uploads_dir).await?;
        let id = Uuid::new_v4().to_string();
        let path = self.uploads_dir.join(&id);
        fs::File::create(&path).await.context("创建上传文件失败")?;
        let session = UploadSession {
            id,
            filename,
            length,
            offset: 0,
            path: path_to_string(path)?,
            status: UploadStatus::Uploading,
        };
        self.db.insert_upload(&session)?;
        Ok(session)
    }

    pub fn get(&self, id: &str) -> anyhow::Result<UploadSession> {
        self.db
            .get_upload(id)?
            .ok_or_else(|| anyhow::anyhow!("上传会话不存在"))
    }

    pub async fn append_chunk(
        &self,
        id: &str,
        expected_offset: u64,
        chunk: &[u8],
    ) -> anyhow::Result<UploadSession> {
        let mut session = self.get(id)?;
        if session.offset != expected_offset {
            anyhow::bail!("上传偏移不一致，请刷新后继续上传");
        }
        let next_offset = session
            .offset
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow::anyhow!("上传大小溢出"))?;
        if next_offset > session.length {
            anyhow::bail!("上传内容超过声明大小");
        }
        self.capacity.ensure_can_upload(session.length)?;

        let mut file = OpenOptions::new()
            .write(true)
            .open(&session.path)
            .await
            .context("打开上传文件失败")?;
        file.seek(SeekFrom::Start(expected_offset)).await?;
        file.write_all(chunk).await.context("写入上传分片失败")?;
        file.flush().await.context("刷新上传文件失败")?;

        let status = if next_offset == session.length {
            UploadStatus::Uploaded
        } else {
            UploadStatus::Uploading
        };
        self.db.update_upload_offset(id, next_offset, status)?;
        session.offset = next_offset;
        session.status = status;
        Ok(session)
    }
}
