use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UploadSession {
    pub id: String,
    pub filename: String,
    pub length: u64,
    pub offset: u64,
    pub path: String,
    pub status: UploadStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadStatus {
    Uploading,
    Uploaded,
}

impl UploadStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Uploading => "uploading",
            Self::Uploaded => "uploaded",
        }
    }
}

impl TryFrom<&str> for UploadStatus {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "uploading" => Ok(Self::Uploading),
            "uploaded" => Ok(Self::Uploaded),
            other => anyhow::bail!("未知上传状态: {other}"),
        }
    }
}
