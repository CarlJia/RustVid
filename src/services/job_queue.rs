use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::{
    domain::{
        job::{JobStatus, VideoJob},
        preset::{OutputTarget, output_plan},
    },
    persistence::sqlite::Database,
    services::{
        artifact_store::ArtifactStore,
        capacity::CapacityService,
        ffmpeg::{SharedTranscoder, Transcoder},
    },
};

#[derive(Clone)]
pub struct JobQueue {
    db: Database,
    capacity: CapacityService,
    artifacts: ArtifactStore,
    transcoder: SharedTranscoder,
    semaphore: Arc<Semaphore>,
}

impl JobQueue {
    pub fn new(
        db: Database,
        capacity: CapacityService,
        artifacts: ArtifactStore,
        transcoder: SharedTranscoder,
        max_concurrent: usize,
    ) -> Self {
        Self {
            db,
            capacity,
            artifacts,
            transcoder,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    pub async fn process_one(&self) -> anyhow::Result<bool> {
        self.capacity.ensure_can_start_transcode()?;
        let Some(job) = self.db.next_queued_job()? else {
            return Ok(false);
        };
        let _permit = self.semaphore.acquire().await?;
        self.process_job(job).await?;
        Ok(true)
    }

    pub async fn process_job(&self, job: VideoJob) -> anyhow::Result<()> {
        self.db
            .transition_job(&job.id, JobStatus::Processing, None)?;
        let result = self.process_job_inner(&job).await;
        if let Err(err) = result {
            let summary = user_error_summary(&err);
            self.db
                .transition_job(&job.id, JobStatus::Failed, Some(&summary))?;
            return Err(err);
        }
        Ok(())
    }

    async fn process_job_inner(&self, job: &VideoJob) -> anyhow::Result<()> {
        let upload = self
            .db
            .get_upload(&job.upload_id)?
            .ok_or_else(|| anyhow::anyhow!("上传文件不存在"))?;
        let output_dir = self.artifacts.artifact_dir(&job.id);
        let plan = output_plan(job.preset, job.target);
        let output = self
            .transcoder
            .transcode(std::path::Path::new(&upload.path), &output_dir, &plan)
            .await?;
        if job.target == OutputTarget::Hls && !output.download_path.exists() {
            anyhow::bail!("HLS zip 包没有生成");
        }
        let artifact = self
            .artifacts
            .create_artifact_record(
                &job.id,
                job.target,
                output.preview_path,
                output.download_path,
            )
            .await?;
        self.db.attach_artifact(&job.id, &artifact)?;
        Ok(())
    }
}

pub fn user_error_summary(err: &anyhow::Error) -> String {
    let text = err.to_string();
    if text.contains("FFmpeg") {
        "视频转码失败，请检查原视频格式后重试".to_string()
    } else {
        format!("处理失败：{text}")
    }
}

pub fn shared_transcoder<T: Transcoder + 'static>(transcoder: T) -> SharedTranscoder {
    Arc::new(transcoder)
}
