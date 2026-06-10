use serde::{Deserialize, Serialize};

use super::preset::OutputTarget;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub job_id: String,
    pub target: OutputTarget,
    pub preview_path: String,
    pub download_path: String,
    pub size_bytes: u64,
}
