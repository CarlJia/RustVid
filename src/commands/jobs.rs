use serde::Serialize;
use tauri::State;

use crate::{
    app::AppState,
    domain::{
        job::JobStatus,
        preset::{OutputTarget, PresetId, presets as domain_presets, Preset},
    },
};

#[derive(Debug, Serialize)]
pub struct JobInfo {
    pub id: String,
    pub upload_id: String,
    /// 源上传文件的原始文件名(由 commands 层从 uploads 表查得填入)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload_filename: Option<String>,
    pub preset: String,
    /// 中文可读预设名(例如 "博客发布"),不再让用户看 "blog"
    pub preset_label: String,
    /// 预设一句话描述(例如 "适合个人博客和普通网页嵌入")
    pub preset_summary: String,
    pub target: String,
    /// 中文可读输出目标(例如 "MP4 视频" / "HLS 流媒体")
    pub target_label: String,
    pub status: String,
    pub error_summary: Option<String>,
    pub artifact_id: Option<String>,
    /// 源视频总时长(秒),`None` = 探测失败或旧任务
    pub source_duration_secs: Option<f64>,
    /// 创建时间(ISO 8601 字符串)
    pub created_at: String,
}

impl From<crate::domain::job::VideoJob> for JobInfo {
    fn from(job: crate::domain::job::VideoJob) -> Self {
        // 查预设中文名(从全局 presets() 表里按 id 找)
        let all_presets = domain_presets();
        let preset: &Preset = all_presets
            .iter()
            .find(|p| p.id == job.preset)
            .expect("preset id 一定在全局表里");
        let target_label = match job.target {
            OutputTarget::Mp4 => "MP4 视频".to_string(),
            OutputTarget::Hls => "HLS 流媒体".to_string(),
        };
        Self {
            id: job.id,
            upload_id: job.upload_id,
            upload_filename: None, // 由 enrich_with_upload 填充
            preset: job.preset.as_str().to_string(),
            preset_label: preset.name.to_string(),
            preset_summary: preset.summary.to_string(),
            target: job.target.as_str().to_string(),
            target_label,
            status: job.status.as_str().to_string(),
            error_summary: job.error_summary,
            artifact_id: job.artifact_id,
            source_duration_secs: job.source_duration_secs,
            created_at: job.created_at,
        }
    }
}

/// 触发 worker 立即处理下一个 queued 任务(替代原 HTTP `/api/jobs/process-next` 轮询)
#[tauri::command]
pub async fn process_next(state: State<'_, AppState>) -> Result<bool, String> {
    state
        .queue
        .process_one()
        .await
        .map_err(|e| e.to_string())
}

/// 给 JobInfo 填上 upload_filename(从 uploads 表查)
/// 这是个 helper,所有暴露 JobInfo 的 command 都用,保证字段一致
fn enrich_with_upload(state: &AppState, mut info: JobInfo) -> JobInfo {
    if let Ok(Some(upload)) = state.db.get_upload(&info.upload_id) {
        info.upload_filename = Some(upload.filename);
    }
    info
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>) -> Result<Vec<JobInfo>, String> {
    state.db.list_jobs().map_err(|e| e.to_string()).map(|jobs| {
        jobs
            .into_iter()
            .map(JobInfo::from)
            .map(|info| enrich_with_upload(state.inner(), info))
            .collect()
    })
}

#[tauri::command]
pub async fn get_job(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<JobInfo>, String> {
    Ok(state
        .db
        .get_job(&id)
        .map_err(|e| e.to_string())?
        .map(JobInfo::from)
        .map(|info| enrich_with_upload(state.inner(), info)))
}

#[tauri::command]
pub async fn create_job(
    state: State<'_, AppState>,
    upload_id: String,
    preset: String,
    target: String,
) -> Result<JobInfo, String> {
    let preset_id: PresetId = preset
        .as_str()
        .try_into()
        .map_err(|e: anyhow::Error| e.to_string())?;
    let target_enum: crate::domain::preset::OutputTarget = target
        .as_str()
        .try_into()
        .map_err(|e: anyhow::Error| e.to_string())?;
    state
        .queue
        .enqueue(&upload_id, preset_id, target_enum)
        .await
        .map_err(|e| e.to_string())
        .map(JobInfo::from)
        .map(|info| enrich_with_upload(state.inner(), info))
}

#[tauri::command]
pub async fn delete_job(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let job = state
        .db
        .get_job(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "任务不存在".to_string())?;
    // Processing 状态:转码进行中,先标记 Failed 再删(走 can_transition_to 校验)
    if matches!(job.status, crate::domain::job::JobStatus::Processing) {
        state
            .db
            .transition_job(&id, crate::domain::job::JobStatus::Failed, None)
            .map_err(|e| format!("回收 Processing 状态失败:{e}"))?;
    }
    let upload = state
        .db
        .get_upload(&job.upload_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "上传记录不存在".to_string())?;
    state
        .artifacts
        .delete_job_files(&id, Some(&upload.path))
        .await
        .map_err(|e| e.to_string())?;
    // 删 DB 里的 artifact 记录(释放 storage 用量计数)
    state
        .db
        .delete_artifact_by_job(&id)
        .map_err(|e| e.to_string())?;
    // 软删:把 jobs 标 'deleted',list_jobs 自动过滤
    state.db.mark_deleted(&id).map_err(|e| e.to_string())?;
    Ok(())
}

/// 完整清理一个 job 的所有痕迹:文件 + DB 行
/// (用于一键清理:真删,不留 history)
async fn purge_job(
    state: &AppState,
    job: &crate::domain::job::VideoJob,
) -> anyhow::Result<()> {
    // 上传源文件:可能已被前面的删除清理过,这里 silent 失败
    if let Ok(Some(upload)) = state.db.get_upload(&job.upload_id) {
        let _ = state
            .artifacts
            .delete_job_files(&job.id, Some(&upload.path))
            .await;
    } else {
        // 至少清掉 artifact 目录
        let _ = state.artifacts.delete_job_files(&job.id, None).await;
    }
    // DB 记录:artifacts 行 + jobs 行都真删
    state.db.delete_artifact_by_job(&job.id)?;
    state.db.hard_delete_job(&job.id)?;
    Ok(())
}

/// 一键清理所有失败任务(文件 + DB 行)。返回清理数量。
#[tauri::command]
pub async fn delete_failed_jobs(state: State<'_, AppState>) -> Result<usize, String> {
    let failed = state
        .db
        .list_failed_jobs()
        .map_err(|e| e.to_string())?;
    let total = failed.len();
    tracing::info!(count = total, "开始一键清理失败任务");
    let mut purged = 0usize;
    for job in &failed {
        if let Err(e) = purge_job(&state, job).await {
            // 单个失败不让整批中断,记 warn 继续
            tracing::warn!(job_id = %job.id, error = %e, "清理失败任务失败,跳过");
            continue;
        }
        purged += 1;
    }
    tracing::info!(
        total,
        purged,
        skipped = total - purged,
        "一键清理完成"
    );
    Ok(purged)
}

#[tauri::command]
pub async fn retry_job(
    state: State<'_, AppState>,
    id: String,
) -> Result<JobInfo, String> {
    let job = state
        .db
        .get_job(&id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "任务不存在".to_string())?;
    // Processing 状态:先回收成 Failed 再重试
    if matches!(job.status, JobStatus::Processing) {
        state
            .db
            .transition_job(&id, JobStatus::Failed, None)
            .map_err(|e| format!("回收 Processing 状态失败:{e}"))?;
    }
    state
        .db
        .transition_job(&id, JobStatus::Queued, None)
        .map_err(|e| e.to_string())?;
    state
        .db
        .get_job(&id)
        .map_err(|e| e.to_string())?
        .map(JobInfo::from)
        .map(|info| enrich_with_upload(state.inner(), info))
        .ok_or_else(|| "任务不存在".to_string())
}
