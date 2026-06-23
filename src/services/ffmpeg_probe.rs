//! ffmpeg 源文件元数据探测 + 码率协商
//!
//! 职责:
//! 1. 用嵌入的 ffmpeg 二进制调 `ffmpeg -hide_banner -i <input>`,从 stderr 解析元数据
//!    (替代 ffprobe,因 BtbN/FFmpeg-Builds 不再提供跨平台 ffprobe,macOS 走 evermeet.cx 也只给 ffmpeg)
//! 2. 提取出视频/音频码率(以 bps 整数表示)
//! 3. 与 `OutputPlan` 的目标码率比较,把 effective 码率 clamp 在 `min(目标, 源)`
//! 4. 任何探测失败都 **不冒泡**,而是回退到目标码率 + `tracing::warn!` 记录
//!
//! 公开 API 与旧实现兼容:`probe_and_resolve` / `ResolvedBitrates` / `parse_bitrate` 全部保留。

use std::path::Path;
use std::process::Stdio;
use std::sync::LazyLock;

use regex::Regex;
use tokio::process::Command;

use crate::domain::preset::OutputPlan;
use crate::services::ffmpeg_binary::{ffmpeg_bitrate_unit, ffmpeg_path};

/// 兜底码率:当 `parse_bitrate(plan.video_bitrate)` 自身失败(理论不应发生)时使用
const FALLBACK_VIDEO_BPS: u64 = 1_200_000;
/// 同上,音频兜底
const FALLBACK_AUDIO_BPS: u64 = 96_000;

/// 码率协商结果。`source_*_bps` 为 `None` 表示 ffmpeg 没能拿到源的码率。
#[derive(Debug, Clone)]
pub struct ResolvedBitrates {
    /// 源视频码率(bps);`None` = ffmpeg 没拿到
    pub source_video_bps: Option<u64>,
    /// 源音频码率(bps);`None` = ffmpeg 没拿到
    pub source_audio_bps: Option<u64>,
    /// 源视频总时长(秒);`0.0` = ffmpeg 没拿到(VFR 或某些特殊格式)
    pub source_duration_secs: f64,
    /// 视频有效码率 = `min(target, source)`;源缺失时 = target
    pub effective_video_bps: u64,
    /// 音频有效码率
    pub effective_audio_bps: u64,
    /// 预设目标视频码率(原始 plan 解析出的 bps,供日志/对比)
    pub target_video_bps: u64,
    /// 预设目标音频码率
    pub target_audio_bps: u64,
    /// 是否被 cap 住(effective < target)。日志/测试断言用
    pub was_capped: bool,
}

impl ResolvedBitrates {
    /// 把 effective 视频码率重新序列化成 ffmpeg 接受的字符串,如 `"1200k"` / `"2.2M"` / `"800"`
    pub fn effective_video_str(&self) -> String {
        format_bitrate(self.effective_video_bps)
    }

    /// 音频版本
    pub fn effective_audio_str(&self) -> String {
        format_bitrate(self.effective_audio_bps)
    }

    /// `-bufsize` 用的字符串(2 × effective),允许短时码率冲顶再回落
    pub fn video_bufsize_str(&self) -> String {
        format_bitrate(self.effective_video_bps.saturating_mul(2))
    }
}

/// 探测源文件并协商有效码率。失败永远回退,不返回 `Err`。
pub async fn probe_and_resolve(input: &Path, plan: &OutputPlan) -> ResolvedBitrates {
    let target_video_bps = parse_bitrate(plan.video_bitrate).unwrap_or_else(|| {
        tracing::error!(
            target_bitrate = plan.video_bitrate,
            "预设视频码率解析失败,使用兜底 {FALLBACK_VIDEO_BPS}"
        );
        FALLBACK_VIDEO_BPS
    });
    let target_audio_bps = parse_bitrate(plan.audio_bitrate).unwrap_or_else(|| {
        tracing::error!(
            target_bitrate = plan.audio_bitrate,
            "预设音频码率解析失败,使用兜底 {FALLBACK_AUDIO_BPS}"
        );
        FALLBACK_AUDIO_BPS
    });

    let (source_video_bps, source_audio_bps, source_duration_secs) = match run_ffmpeg_meta(input).await
    {
        Ok(meta) => (meta.video_bps, meta.audio_bps, meta.duration_secs),
        Err(reason) => {
            tracing::warn!(%reason, "ffmpeg 元数据探测失败,使用预设目标码率(不会触发 cap)");
            (None, None, 0.0)
        }
    };

    let effective_video_bps = source_video_bps
        .map(|s| s.min(target_video_bps))
        .unwrap_or(target_video_bps);
    let effective_audio_bps = source_audio_bps
        .map(|s| s.min(target_audio_bps))
        .unwrap_or(target_audio_bps);

    let was_capped =
        effective_video_bps < target_video_bps || effective_audio_bps < target_audio_bps;

    if was_capped {
        tracing::info!(
            source_video_bps = ?source_video_bps,
            source_audio_bps = ?source_audio_bps,
            target_video_bps,
            target_audio_bps,
            effective_video_bps,
            effective_audio_bps,
            "码率被 cap:输出不会超过源"
        );
    }

    ResolvedBitrates {
        source_video_bps,
        source_audio_bps,
        source_duration_secs,
        effective_video_bps,
        effective_audio_bps,
        target_video_bps,
        target_audio_bps,
        was_capped,
    }
}

// ---- ffmpeg 子进程调用 ----

/// 调 ffmpeg -hide_banner -i 拿元数据。注意:ffmpeg 单独跑 -i 时 exit code 非 0 是正常的(stderr 仍有元数据)
async fn run_ffmpeg_meta(input: &Path) -> anyhow::Result<FfmpegMeta> {
    let input_str = input
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("输入路径不是有效的 UTF-8"))?;
    let output = Command::new(ffmpeg_path())
        .args(["-hide_banner", "-i", input_str])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| anyhow::anyhow!("启动 ffmpeg 失败:{e}"))?;

    // ffmpeg 单独 -i 时会返回非 0 退出码,但 stderr 含完整元数据,正常处理
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.is_empty() {
        anyhow::bail!("ffmpeg 未输出任何 stderr,可能二进制损坏");
    }
    Ok(parse_ffmpeg_stderr(&stderr))
}

#[derive(Debug, Default, Clone, Copy)]
struct FfmpegMeta {
    /// 视频流最大码率(多视频流时取最大)
    video_bps: Option<u64>,
    /// 第一条音频流的码率
    audio_bps: Option<u64>,
    /// 源总时长(秒),从 `Duration: HH:MM:SS.ff` 提取
    duration_secs: f64,
}

// ---- 正则解析 stderr ----

/// 匹配 `Duration: HH:MM:SS.ff, start: 0.000000, bitrate: X kb/s`
/// capture(1)=HH, (2)=MM, (3)=SS.ff
static RE_DURATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?),\s*start:[^,]+,\s*bitrate:\s*([0-9.]+)\s*(kbit/s|Mbit/s|kb/s|Mb/s|kbps|Mbps|bit/s|b/s|bps)")
        .expect("编译期正则")
});

/// 匹配 `Stream #0:0 ... Video: ..., X kb/s, 30 fps, ...`
static RE_VIDEO_BITRATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Stream\s+#\d+:\d+[^\n]*?Video:[^\n]*?,\s*([0-9.]+)\s*(kbit/s|Mbit/s|kb/s|Mb/s|kbps|Mbps|bit/s|b/s|bps)")
        .expect("编译期正则")
});

/// 匹配 `Stream #0:1 ... Audio: ..., X kb/s ...`
static RE_AUDIO_BITRATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Stream\s+#\d+:\d+[^\n]*?Audio:[^\n]*?,\s*([0-9.]+)\s*(kbit/s|Mbit/s|kb/s|Mb/s|kbps|Mbps|bit/s|b/s|bps)")
        .expect("编译期正则")
});

fn parse_ffmpeg_stderr(stderr: &str) -> FfmpegMeta {
    let mut meta = FfmpegMeta::default();

    // 视频流:取所有 Stream 行中最大的那条(主轨道通常最高)
    for c in RE_VIDEO_BITRATE.captures_iter(stderr) {
        if let Some(bps) = ffmpeg_bitrate_unit(&c[1], &c[2]) {
            meta.video_bps = Some(meta.video_bps.map_or(bps, |m| m.max(bps)));
        }
    }

    // 音频流:第一条
    if let Some(c) = RE_AUDIO_BITRATE.captures(stderr) {
        meta.audio_bps = ffmpeg_bitrate_unit(&c[1], &c[2]);
    }

    // Duration:提取时长 + 整体码率(整体码率只在 video_bps 没拿到时用)
    if let Some(c) = RE_DURATION.captures(stderr) {
        let h: f64 = c[1].parse().unwrap_or(0.0);
        let m: f64 = c[2].parse().unwrap_or(0.0);
        let s: f64 = c[3].parse().unwrap_or(0.0);
        meta.duration_secs = h * 3600.0 + m * 60.0 + s;
        if meta.video_bps.is_none() {
            meta.video_bps = ffmpeg_bitrate_unit(&c[4], &c[5]);
        }
    }

    meta
}

// ---- 码率字符串解析(预设格式) ----

/// 解析 ffmpeg 风格的码率字符串。单位遵循 ffmpeg 的十进制惯例:
/// - `k` / `kbps` = 1000
/// - `M` / `mbps` = 1_000_000
/// - 无后缀 / `bps` = 1
///
/// 接受 `"1200k"` / `"2.2M"` / `"128kbps"` / `"1500"` 等;大小写不敏感;失败返回 `None`。
pub fn parse_bitrate(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // 找到数字结束的索引(数字字符 + 小数点)
    let split_at = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());

    let (num_part, suffix) = s.split_at(split_at);
    if num_part.is_empty() {
        return None;
    }
    let n: f64 = num_part.parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }

    let multiplier: u64 = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "bps" => 1,
        "k" | "kbps" => 1_000,
        "m" | "mbps" => 1_000_000,
        // 未知单位(本项目 plan 不会用到):返回 None 让调用方 fallback
        _ => return None,
    };

    Some((n * multiplier as f64) as u64)
}

/// 把 bps 整数格式化成 ffmpeg 接受的字符串。优先用 k / M,小值不加单位。
/// 规则:能四舍五入到整数的就不带小数,否则保留 1 位小数。
fn format_bitrate(bps: u64) -> String {
    if bps >= 1_000_000 {
        let m = bps as f64 / 1_000_000.0;
        let rounded = m.round();
        if (m - rounded).abs() < 0.05 {
            format!("{}M", rounded as u64)
        } else {
            format!("{m:.1}M")
        }
    } else if bps >= 1_000 {
        let k = bps as f64 / 1_000.0;
        let rounded = k.round();
        if (k - rounded).abs() < 0.05 {
            format!("{}k", rounded as u64)
        } else {
            format!("{k:.1}k")
        }
    } else {
        bps.to_string()
    }
}

// ---- 单元测试 ----

#[cfg(test)]
mod tests {
    use super::*;

    /// 真实 ffmpeg -i 在 720p 5秒低码率 MOV 上的输出
    const FIXTURE_STDERR: &str = "\
Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'low_bitrate.mov':
  Metadata:
    major_brand     : qt
    minor_version   : 0
    compatible_brands: qt
    encoder         : Lavf62.19.101
  Duration: 00:00:05.00, start: 0.000000, bitrate: 1844 kb/s
  Stream #0:0(und): Video: h264 (High 4:4:4 Predictive) (avc1 / 0x31637661), yuv444p, 1280x720 [SAR 1:1 DAR 16:9], 185 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
    Metadata:
      handler_name    : VideoHandler
      vendor_id       : [0][0][0][0]
  Stream #0:1(und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 64 kb/s (default)
    Metadata:
      handler_name    : SoundHandler
      vendor_id       : [0][0][0][0]
At least one output file must be specified
";

    #[test]
    fn parse_ffmpeg_stderr_完整输出取_video_audio() {
        let meta = parse_ffmpeg_stderr(FIXTURE_STDERR);
        // stream 行覆盖 Duration 行:185000(注意 stream 取最大值,如果只有 1 条就是 185000)
        assert_eq!(meta.video_bps, Some(185_000));
        assert_eq!(meta.audio_bps, Some(64_000));
    }

    #[test]
    fn parse_ffmpeg_stderr_多视频流取最大值() {
        let s = "\
  Duration: 00:00:05.00, start: 0.000000, bitrate: 1000 kb/s
  Stream #0:0(und): Video: h264, 800 kb/s, 30 fps
  Stream #0:1(und): Video: h264, 1500 kb/s, 30 fps
  Stream #0:2(und): Audio: aac, 96 kb/s
";
        let meta = parse_ffmpeg_stderr(s);
        assert_eq!(meta.video_bps, Some(1_500_000));
        assert_eq!(meta.audio_bps, Some(96_000));
    }

    #[test]
    fn parse_ffmpeg_stderr_只有_duration_行() {
        let s = "  Duration: 00:00:05.00, start: 0.000000, bitrate: 1500 kb/s\n";
        let meta = parse_ffmpeg_stderr(s);
        // 没 stream 行时退回到 Duration
        assert_eq!(meta.video_bps, Some(1_500_000));
        assert_eq!(meta.audio_bps, None);
    }

    #[test]
    fn parse_ffmpeg_stderr_无音频轨() {
        let s = "\
  Duration: 00:00:05.00, start: 0.000000, bitrate: 800 kb/s
  Stream #0:0(und): Video: h264, 800 kb/s, 30 fps
";
        let meta = parse_ffmpeg_stderr(s);
        assert_eq!(meta.video_bps, Some(800_000));
        assert_eq!(meta.audio_bps, None);
    }

    #[test]
    fn parse_ffmpeg_stderr_空字符串() {
        let meta = parse_ffmpeg_stderr("");
        assert!(meta.video_bps.is_none());
        assert!(meta.audio_bps.is_none());
    }

    #[test]
    fn parse_ffmpeg_stderr_mbit_s_单位() {
        let s = "\
  Duration: 00:00:30.00, start: 0.000000, bitrate: 2.5 Mbit/s
  Stream #0:0(und): Video: h264, 1.8 Mbit/s, 30 fps
";
        let meta = parse_ffmpeg_stderr(s);
        assert_eq!(meta.video_bps, Some(1_800_000));
    }

    #[test]
    fn parse_bitrate_解析_k_单位() {
        assert_eq!(parse_bitrate("1200k"), Some(1_200_000));
        assert_eq!(parse_bitrate("96K"), Some(96_000));
    }

    #[test]
    fn parse_bitrate_解析_m_单位() {
        assert_eq!(parse_bitrate("2.2M"), Some(2_200_000));
        assert_eq!(parse_bitrate("6Mbps"), Some(6_000_000));
    }

    #[test]
    fn parse_bitrate_解析_无单位() {
        assert_eq!(parse_bitrate("1500"), Some(1500));
        assert_eq!(parse_bitrate("192bps"), Some(192));
    }

    #[test]
    fn parse_bitrate_解析_kbps_后缀() {
        assert_eq!(parse_bitrate("128kbps"), Some(128_000));
    }

    #[test]
    fn parse_bitrate_带前后空白() {
        assert_eq!(parse_bitrate("  1200k  "), Some(1_200_000));
    }

    #[test]
    fn parse_bitrate_无效输入返回_none() {
        assert_eq!(parse_bitrate(""), None);
        assert_eq!(parse_bitrate("   "), None);
        assert_eq!(parse_bitrate("abc"), None);
        assert_eq!(parse_bitrate("1G"), None);
        assert_eq!(parse_bitrate("k1200"), None);
    }

    #[test]
    fn format_bitrate_选合适单位() {
        assert_eq!(format_bitrate(0), "0");
        assert_eq!(format_bitrate(800), "800");
        assert_eq!(format_bitrate(96_000), "96k");
        assert_eq!(format_bitrate(1_200_000), "1.2M");
        assert_eq!(format_bitrate(2_200_000), "2.2M");
        assert_eq!(format_bitrate(2_000_000), "2M");
    }

    #[test]
    fn resolved_bitrates_字符串方法_往返正确() {
        let r = ResolvedBitrates {
            source_video_bps: Some(800_000),
            source_audio_bps: Some(96_000),
            source_duration_secs: 5.0,
            effective_video_bps: 800_000,
            effective_audio_bps: 96_000,
            target_video_bps: 1_200_000,
            target_audio_bps: 96_000,
            was_capped: true,
        };
        assert_eq!(r.effective_video_str(), "800k");
        assert_eq!(r.effective_audio_str(), "96k");
        // 2x 800k = 1600k = 1.6M
        assert_eq!(r.video_bufsize_str(), "1.6M");
    }
}
