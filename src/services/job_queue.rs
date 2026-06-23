use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{
    domain::{
        job::{JobStatus, VideoJob},
        preset::{OutputTarget, PresetId, output_plan},
        upload::UploadStatus,
    },
    persistence::sqlite::Database,
    services::{
        artifact_store::ArtifactStore,
        capacity::CapacityService,
        ffmpeg::{ProgressFn, SharedTranscoder, TranscodeProgress, Transcoder},
    },
};

/// Tauri 事件 payload,前端 `listen('transcode-progress')` 收到的结构
#[derive(Debug, Clone, Serialize)]
pub struct TranscodeProgressEvent {
    pub job_id: String,
    pub encoded_secs: f64,
    pub speed: f64,
    /// 0-100,基于 source_duration_secs 计算;`None` = 源时长未知
    pub percent: Option<f64>,
}

#[derive(Clone)]
pub struct JobQueue {
    db: Database,
    capacity: CapacityService,
    artifacts: ArtifactStore,
    transcoder: SharedTranscoder,
    semaphore: Arc<Semaphore>,
    /// `None` 时不 emit 事件(测试场景);`Some` 时进度回调会触发 `transcode-progress` 事件
    app: Option<AppHandle>,
}

impl JobQueue {
    pub fn new(
        db: Database,
        capacity: CapacityService,
        artifacts: ArtifactStore,
        transcoder: SharedTranscoder,
        max_concurrent: usize,
        app: Option<AppHandle>,
    ) -> Self {
        Self {
            db,
            capacity,
            artifacts,
            transcoder,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            app,
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

    /// 创建转码任务(从原 http/jobs.rs::create_job 抽出来)
    pub async fn enqueue(
        &self,
        upload_id: &str,
        preset: PresetId,
        target: OutputTarget,
    ) -> anyhow::Result<VideoJob> {
        self.capacity.ensure_can_start_transcode()?;
        let upload = self
            .db
            .get_upload(upload_id)?
            .ok_or_else(|| anyhow::anyhow!("上传文件不存在"))?;
        if upload.status != UploadStatus::Uploaded {
            anyhow::bail!("上传完成后才能创建转码任务");
        }
        // 探测源时长,作为 percent 计算依据;失败则 None
        let source_duration_secs = probe_duration(std::path::Path::new(&upload.path))
            .await
            .ok();
        let job = VideoJob {
            id: Uuid::new_v4().to_string(),
            upload_id: upload_id.to_string(),
            preset,
            target,
            status: JobStatus::Queued,
            error_summary: None,
            artifact_id: None,
            source_duration_secs,
            // INSERT 走 DEFAULT CURRENT_TIMESTAMP,这里填空即可
            created_at: String::new(),
        };
        self.db.insert_job(&job)?;
        Ok(job)
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

        // 用源文件名(去后缀)+ 时间戳命名产物,例:`my_video_20260622-163045.mp4`
        // 时间戳用本地时间(用户视角),格式 YYYYMMDD-HHMMSS
        let base = std::path::Path::new(&upload.filename)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output")
            .to_string();
        let ext = match job.target {
            OutputTarget::Mp4 => "mp4",
            OutputTarget::Hls => "zip",
        };
        let timestamp = chrono_like_timestamp();
        let output_name = format!("{base}_{timestamp}.{ext}");
        let output = output_dir.join(&output_name);

        // 构建进度回调:有 AppHandle 时,emit Tauri 事件;无时(测试)noop
        let total_duration = job.source_duration_secs.unwrap_or(0.0);
        let progress: Option<ProgressFn> = self.app.as_ref().map(|app| {
            let app = app.clone();
            let job_id = job.id.clone();
            Arc::new(move |p: TranscodeProgress| {
                let percent = if total_duration > 0.0 {
                    Some((p.encoded_secs / total_duration * 100.0).clamp(0.0, 100.0))
                } else {
                    None
                };
                let event = TranscodeProgressEvent {
                    job_id: job_id.clone(),
                    encoded_secs: p.encoded_secs,
                    speed: p.speed,
                    percent,
                };
                let _ = app.emit("transcode-progress", &event);
            }) as ProgressFn
        });

        let result = self
            .transcoder
            .transcode(
                std::path::Path::new(&upload.path),
                &output,
                &output_dir, // HLS 时 ts 分片放这里
                &plan,
                progress,
            )
            .await?;
        if job.target == OutputTarget::Hls && !result.download_path.exists() {
            anyhow::bail!("HLS zip 包没有生成");
        }
        let artifact = self
            .artifacts
            .create_artifact_record(
                &job.id,
                job.target,
                result.preview_path,
                result.download_path,
            )
            .await?;
        self.db.attach_artifact(&job.id, &artifact)?;
        Ok(())
    }
}

/// 本地时间戳,格式 `YYYYMMDD-HHMMSS`
fn chrono_like_timestamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub fn user_error_summary(err: &anyhow::Error) -> String {
    let text = err.to_string();
    // run_ffmpeg 的错误格式: "FFmpeg 生成 {label} 失败(exit={code}): {stderr_tail}"
    // 把 stderr tail 抽出来给用户看,而不是吞成通用提示
    if let Some(rest) = text.strip_prefix("FFmpeg 生成 ")
        && let Some(stderr) = rest.split("): ").nth(1)
    {
        let trimmed = stderr.trim();
        // 截断到 280 字符避免 UI 卡片太长
        let head: String = if trimmed.chars().count() > 280 {
            format!("{}…", trimmed.chars().take(280).collect::<String>())
        } else {
            trimmed.to_string()
        };
        return format!("FFmpeg 失败: {head}");
    }
    format!("处理失败:{text}")
}

/// 探测源视频时长(秒)。失败返回 `Err`,调用方 fallback。
async fn probe_duration(input: &std::path::Path) -> anyhow::Result<f64> {
    use crate::services::ffmpeg_probe::probe_and_resolve;
    use crate::domain::preset::output_plan;
    use crate::domain::preset::PresetId;
    use crate::domain::preset::OutputTarget;

    // 用最低规格探测,只关心 duration,不影响后续转码
    let plan = output_plan(PresetId::Blog, OutputTarget::Mp4);
    let resolved = probe_and_resolve(input, &plan).await;
    if resolved.source_duration_secs > 0.0 {
        Ok(resolved.source_duration_secs)
    } else {
        anyhow::bail!("无法获取源时长")
    }
}

pub fn shared_transcoder<T: Transcoder + 'static>(transcoder: T) -> SharedTranscoder {
    Arc::new(transcoder)
}
