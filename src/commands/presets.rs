use serde::Serialize;

use crate::domain::preset::{Preset, presets as domain_presets};

#[derive(Debug, Serialize)]
pub struct PresetInfo {
    pub id: String,
    pub name: &'static str,
    pub summary: &'static str,
    pub resolution: &'static str,
    pub bitrate_hint: &'static str,
    pub hls_hint: &'static str,
}

impl From<Preset> for PresetInfo {
    fn from(p: Preset) -> Self {
        Self {
            id: p.id.as_str().to_string(),
            name: p.name,
            summary: p.summary,
            resolution: p.resolution,
            bitrate_hint: p.bitrate_hint,
            hls_hint: p.hls_hint,
        }
    }
}

#[tauri::command]
pub fn get_presets() -> Vec<PresetInfo> {
    domain_presets().into_iter().map(PresetInfo::from).collect()
}
