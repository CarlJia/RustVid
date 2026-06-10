use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetId {
    Blog,
    Course,
    Mobile,
    Archive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputTarget {
    Mp4,
    Hls,
}

#[derive(Clone, Debug, Serialize)]
pub struct Preset {
    pub id: PresetId,
    pub name: &'static str,
    pub summary: &'static str,
    pub resolution: &'static str,
    pub bitrate_hint: &'static str,
    pub hls_hint: &'static str,
}

#[derive(Clone, Debug)]
pub struct OutputPlan {
    pub preset: PresetId,
    pub target: OutputTarget,
    pub video_height: u16,
    pub video_bitrate: &'static str,
    pub audio_bitrate: &'static str,
    pub hls_segment_seconds: Option<u8>,
}

impl PresetId {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Blog => "blog",
            Self::Course => "course",
            Self::Mobile => "mobile",
            Self::Archive => "archive",
        }
    }
}

impl OutputTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Hls => "hls",
        }
    }
}

impl TryFrom<&str> for PresetId {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "blog" => Ok(Self::Blog),
            "course" => Ok(Self::Course),
            "mobile" => Ok(Self::Mobile),
            "archive" => Ok(Self::Archive),
            other => anyhow::bail!("未知预设: {other}"),
        }
    }
}

impl TryFrom<&str> for OutputTarget {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "mp4" => Ok(Self::Mp4),
            "hls" => Ok(Self::Hls),
            other => anyhow::bail!("未知输出目标: {other}"),
        }
    }
}

pub fn presets() -> Vec<Preset> {
    vec![
        Preset {
            id: PresetId::Blog,
            name: "博客发布",
            summary: "适合个人博客和普通网页嵌入",
            resolution: "720p",
            bitrate_hint: "中等码率",
            hls_hint: "可生成 6 秒左右分片",
        },
        Preset {
            id: PresetId::Course,
            name: "课程播放",
            summary: "适合课程站和知识付费页面",
            resolution: "1080p",
            bitrate_hint: "稳定清晰",
            hls_hint: "可生成课程播放分片",
        },
        Preset {
            id: PresetId::Mobile,
            name: "移动端优先",
            summary: "适合移动网络下快速加载",
            resolution: "540p",
            bitrate_hint: "较低码率",
            hls_hint: "可生成较轻量分片",
        },
        Preset {
            id: PresetId::Archive,
            name: "高清留档",
            summary: "适合保留较高清晰度副本",
            resolution: "1080p",
            bitrate_hint: "较高质量",
            hls_hint: "可生成高清分片",
        },
    ]
}

pub fn output_plan(preset: PresetId, target: OutputTarget) -> OutputPlan {
    let (height, video_bitrate, audio_bitrate) = match preset {
        PresetId::Blog => (720, "2200k", "128k"),
        PresetId::Course => (1080, "4200k", "160k"),
        PresetId::Mobile => (540, "1200k", "96k"),
        PresetId::Archive => (1080, "6000k", "192k"),
    };

    OutputPlan {
        preset,
        target,
        video_height: height,
        video_bitrate,
        audio_bitrate,
        hls_segment_seconds: (target == OutputTarget::Hls).then_some(6),
    }
}
