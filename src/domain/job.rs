use serde::{Deserialize, Serialize};

use super::preset::{OutputTarget, PresetId};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoJob {
    pub id: String,
    pub upload_id: String,
    pub preset: PresetId,
    pub target: OutputTarget,
    pub status: JobStatus,
    pub error_summary: Option<String>,
    pub artifact_id: Option<String>,
    /// 源视频总时长(秒),由 ffmpeg 探测填充;`None` = 探测失败或旧任务(无此字段)
    #[serde(default)]
    pub source_duration_secs: Option<f64>,
    /// 创建时间,SQLite CURRENT_TIMESTAMP(ISO 8601 字符串)
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Processing,
    Completed,
    Failed,
    Deleted,
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Processing => "processing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Deleted => "deleted",
        }
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Queued, Self::Processing)
                | (Self::Processing, Self::Completed)
                | (Self::Processing, Self::Failed)
                | (Self::Processing, Self::Deleted)
                | (Self::Failed, Self::Queued)
                | (Self::Completed, Self::Deleted)
                | (Self::Failed, Self::Deleted)
                | (Self::Queued, Self::Deleted)
        )
    }
}

impl TryFrom<&str> for JobStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "queued" => Ok(Self::Queued),
            "processing" => Ok(Self::Processing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "deleted" => Ok(Self::Deleted),
            other => anyhow::bail!("未知任务状态: {other}"),
        }
    }
}
