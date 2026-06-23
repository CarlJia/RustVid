use std::sync::{Arc, Mutex};

use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};

use crate::domain::{
    job::{JobStatus, VideoJob},
    preset::{OutputTarget, PresetId},
    storage::Artifact,
    upload::{UploadSession, UploadStatus},
};

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    pub fn open(path: &std::path::Path) -> anyhow::Result<Self> {
        let conn = Connection::open(path).context("打开 SQLite 数据库失败")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory().context("打开内存 SQLite 失败")?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS uploads (
                id TEXT PRIMARY KEY,
                filename TEXT NOT NULL,
                length INTEGER NOT NULL,
                offset INTEGER NOT NULL,
                path TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                upload_id TEXT NOT NULL,
                preset TEXT NOT NULL,
                target TEXT NOT NULL,
                status TEXT NOT NULL,
                error_summary TEXT,
                artifact_id TEXT,
                source_duration_secs REAL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(upload_id) REFERENCES uploads(id)
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                target TEXT NOT NULL,
                preview_path TEXT NOT NULL,
                download_path TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(job_id) REFERENCES jobs(id)
            );
            "#,
        )
        .context("初始化数据库结构失败")?;
        // 老库 schema 迁移:为已存在的 jobs 表加 source_duration_secs 列(忽略"已存在"错误)
        let _ = conn.execute("ALTER TABLE jobs ADD COLUMN source_duration_secs REAL", []);
        Ok(())
    }

    pub fn insert_upload(&self, upload: &UploadSession) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "INSERT INTO uploads (id, filename, length, offset, path, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                upload.id,
                upload.filename,
                upload.length as i64,
                upload.offset as i64,
                upload.path,
                upload.status.as_str(),
            ],
        )
        .context("保存上传会话失败")?;
        Ok(())
    }

    pub fn get_upload(&self, id: &str) -> anyhow::Result<Option<UploadSession>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.query_row(
            "SELECT id, filename, length, offset, path, status FROM uploads WHERE id = ?1",
            params![id],
            |row| {
                let status: String = row.get(5)?;
                Ok(UploadSession {
                    id: row.get(0)?,
                    filename: row.get(1)?,
                    length: row.get::<_, i64>(2)? as u64,
                    offset: row.get::<_, i64>(3)? as u64,
                    path: row.get(4)?,
                    status: UploadStatus::try_from(status.as_str())
                        .map_err(|err| conversion_error(5, err))?,
                })
            },
        )
        .optional()
        .context("读取上传会话失败")
    }

    pub fn update_upload_offset(
        &self,
        id: &str,
        offset: u64,
        status: UploadStatus,
    ) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "UPDATE uploads SET offset = ?2, status = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![id, offset as i64, status.as_str()],
        )
        .context("更新上传进度失败")?;
        Ok(())
    }

    pub fn insert_job(&self, job: &VideoJob) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "INSERT INTO jobs (id, upload_id, preset, target, status, error_summary, artifact_id, source_duration_secs) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                job.id,
                job.upload_id,
                job.preset.as_str(),
                job.target.as_str(),
                job.status.as_str(),
                job.error_summary,
                job.artifact_id,
                job.source_duration_secs,
            ],
        )
        .context("保存转码任务失败")?;
        Ok(())
    }

    pub fn list_jobs(&self) -> anyhow::Result<Vec<VideoJob>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let mut stmt = conn
            .prepare(
                "SELECT id, upload_id, preset, target, status, error_summary, artifact_id, source_duration_secs, created_at FROM jobs WHERE status != 'deleted' ORDER BY created_at DESC",
            )
            .context("准备任务列表查询失败")?;
        let rows = stmt.query_map([], map_job)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("读取任务列表失败")
    }

    pub fn get_job(&self, id: &str) -> anyhow::Result<Option<VideoJob>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.query_row(
            "SELECT id, upload_id, preset, target, status, error_summary, artifact_id, source_duration_secs, created_at FROM jobs WHERE id = ?1",
            params![id],
            map_job,
        )
        .optional()
        .context("读取任务失败")
    }

    pub fn next_queued_job(&self) -> anyhow::Result<Option<VideoJob>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.query_row(
            "SELECT id, upload_id, preset, target, status, error_summary, artifact_id, source_duration_secs, created_at FROM jobs WHERE status = 'queued' ORDER BY created_at ASC LIMIT 1",
            [],
            map_job,
        )
        .optional()
        .context("读取等待任务失败")
    }

    pub fn count_processing_jobs(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE status = 'processing'",
                [],
                |row| row.get(0),
            )
            .context("统计转码中任务失败")?;
        Ok(count as usize)
    }

    /// 启动时回收:把卡在 Processing 状态的任务标记为 Failed(原因:应用重启,转码中断)
    /// 返回被回收的 job ID 列表
    pub fn reap_stale_processing(&self) -> anyhow::Result<Vec<String>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let mut stmt = conn
            .prepare("SELECT id FROM jobs WHERE status = 'processing'")
            .context("查询僵尸任务失败")?;
        let ids: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()
            .context("读取僵尸任务 ID 失败")?;
        for id in &ids {
            conn.execute(
                "UPDATE jobs SET status = 'failed', error_summary = '应用重启,转码中断,请手动重试', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
                params![id],
            )
            .context("标记僵尸任务失败")?;
        }
        Ok(ids)
    }

    /// 删除任务的 artifact 记录(释放存储容量计数)
    pub fn delete_artifact_by_job(&self, job_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute("DELETE FROM artifacts WHERE job_id = ?1", params![job_id])
            .context("删除产物记录失败")?;
        Ok(())
    }

    pub fn transition_job(
        &self,
        id: &str,
        next: JobStatus,
        error: Option<&str>,
    ) -> anyhow::Result<()> {
        let current = self
            .get_job(id)?
            .ok_or_else(|| anyhow::anyhow!("任务不存在"))?;
        if !current.status.can_transition_to(next) && current.status != next {
            anyhow::bail!("任务状态不能从 {:?} 切换到 {:?}", current.status, next);
        }
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "UPDATE jobs SET status = ?2, error_summary = ?3, updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![id, next.as_str(), error],
        )
        .context("更新任务状态失败")?;
        Ok(())
    }

    pub fn attach_artifact(&self, job_id: &str, artifact: &Artifact) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "INSERT INTO artifacts (id, job_id, target, preview_path, download_path, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                artifact.id,
                artifact.job_id,
                artifact.target.as_str(),
                artifact.preview_path,
                artifact.download_path,
                artifact.size_bytes as i64,
            ],
        )
        .context("保存转码产物失败")?;
        conn.execute(
            "UPDATE jobs SET artifact_id = ?2, status = 'completed', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![job_id, artifact.id],
        )
        .context("关联转码产物失败")?;
        Ok(())
    }

    pub fn get_artifact(&self, id: &str) -> anyhow::Result<Option<Artifact>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.query_row(
            "SELECT id, job_id, target, preview_path, download_path, size_bytes FROM artifacts WHERE id = ?1",
            params![id],
            map_artifact,
        )
        .optional()
        .context("读取产物失败")
    }

    pub fn total_artifact_bytes(&self) -> anyhow::Result<u64> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let total: Option<i64> = conn
            .query_row("SELECT SUM(size_bytes) FROM artifacts", [], |row| {
                row.get(0)
            })
            .context("统计产物容量失败")?;
        Ok(total.unwrap_or(0) as u64)
    }

    pub fn mark_deleted(&self, job_id: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        conn.execute(
            "UPDATE jobs SET status = 'deleted', updated_at = CURRENT_TIMESTAMP WHERE id = ?1",
            params![job_id],
        )
        .context("标记任务删除失败")?;
        Ok(())
    }

    /// 真删(用于一键清理):从 jobs 表移除,触发 ON DELETE CASCADE 自动删 artifacts 行
    /// 返回影响的行数(0/1)
    pub fn hard_delete_job(&self, job_id: &str) -> anyhow::Result<usize> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let n = conn
            .execute("DELETE FROM jobs WHERE id = ?1", params![job_id])
            .context("硬删任务失败")?;
        Ok(n)
    }

    /// 列出所有失败任务(状态='failed'),按时间正序(旧的先清)
    pub fn list_failed_jobs(&self) -> anyhow::Result<Vec<VideoJob>> {
        let conn = self.conn.lock().expect("数据库锁被污染");
        let mut stmt = conn
            .prepare(
                "SELECT id, upload_id, preset, target, status, error_summary, artifact_id, source_duration_secs, created_at FROM jobs WHERE status = 'failed' ORDER BY created_at ASC",
            )
            .context("准备失败任务查询失败")?;
        let rows = stmt.query_map([], map_job)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("读取失败任务列表失败")
    }
}

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<VideoJob> {
    let preset: String = row.get(2)?;
    let target: String = row.get(3)?;
    let status: String = row.get(4)?;
    Ok(VideoJob {
        id: row.get(0)?,
        upload_id: row.get(1)?,
        preset: PresetId::try_from(preset.as_str()).map_err(|err| conversion_error(2, err))?,
        target: OutputTarget::try_from(target.as_str()).map_err(|err| conversion_error(3, err))?,
        status: JobStatus::try_from(status.as_str()).map_err(|err| conversion_error(4, err))?,
        error_summary: row.get(5)?,
        artifact_id: row.get(6)?,
        source_duration_secs: row.get::<_, Option<f64>>(7)?,
        created_at: row.get::<_, String>(8).unwrap_or_default(),
    })
}

fn map_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    let target: String = row.get(2)?;
    Ok(Artifact {
        id: row.get(0)?,
        job_id: row.get(1)?,
        target: OutputTarget::try_from(target.as_str()).map_err(|err| conversion_error(2, err))?,
        preview_path: row.get(3)?,
        download_path: row.get(4)?,
        size_bytes: row.get::<_, i64>(5)? as u64,
    })
}

fn conversion_error(index: usize, err: anyhow::Error) -> rusqlite::Error {
    let err = std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string());
    rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
}
