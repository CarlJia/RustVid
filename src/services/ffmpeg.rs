use std::{path::Path, process::Stdio, sync::Arc, time::Instant};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use tokio::{fs, process::Command};

use crate::domain::preset::{OutputPlan, OutputTarget};
use crate::services::ffmpeg_probe::{ResolvedBitrates, probe_and_resolve};

const FFMPEG_STDERR_TAIL_BYTES: usize = 4 * 1024;

/// 进度回调:每次 ffmpeg 报告一次进度(out_time_us 更新时)调用一次
pub type ProgressFn = Arc<dyn Fn(TranscodeProgress) + Send + Sync>;

/// ffmpeg -progress pipe:1 报告的实时进度
#[derive(Debug, Clone, Default)]
pub struct TranscodeProgress {
    /// 已编码时长(秒)
    pub encoded_secs: f64,
    /// 编码速度(如 "1.5x",去掉 x 后转 f64)
    pub speed: f64,
}

#[async_trait]
pub trait Transcoder: Send + Sync {
    /// 转码入口
    /// - `input`: 源文件路径
    /// - `output`: 完整输出文件路径(包含文件名,例如 `output_20260622-163045.mp4`)
    /// - `work_dir`: HLS 时用,放 m3u8 + ts 分片(MP4 时可忽略)
    /// - `plan`: 转码计划(预设 + 输出格式)
    /// - `progress`: 进度回调(可选)
    async fn transcode(
        &self,
        input: &Path,
        output: &Path,
        work_dir: &Path,
        plan: &OutputPlan,
        progress: Option<ProgressFn>,
    ) -> anyhow::Result<TranscodeOutput>;
}

pub type SharedTranscoder = Arc<dyn Transcoder>;

#[derive(Clone, Debug)]
pub struct TranscodeOutput {
    pub preview_path: std::path::PathBuf,
    pub download_path: std::path::PathBuf,
}

#[derive(Default, Clone)]
pub struct FfmpegTranscoder;

#[async_trait]
impl Transcoder for FfmpegTranscoder {
    async fn transcode(
        &self,
        input: &Path,
        output: &Path,
        work_dir: &Path,
        plan: &OutputPlan,
        progress: Option<ProgressFn>,
    ) -> anyhow::Result<TranscodeOutput> {
        fs::create_dir_all(work_dir).await?;
        // 探测源码率,把 effective 码率 clamp 在 min(预设目标, 源码率),防止低码率源转码后体积膨胀。
        // 探测失败会 fallback 到预设目标 + warn,不阻塞转码。
        let resolved = probe_and_resolve(input, plan).await;
        match plan.target {
            OutputTarget::Mp4 => transcode_mp4(input, output, plan, &resolved, progress).await,
            OutputTarget::Hls => {
                transcode_hls(input, output, work_dir, plan, &resolved, progress).await
            }
        }
    }
}

async fn transcode_mp4(
    input: &Path,
    output: &Path,
    plan: &OutputPlan,
    resolved: &ResolvedBitrates,
    progress: Option<ProgressFn>,
) -> anyhow::Result<TranscodeOutput> {
    let started = Instant::now();
    tracing::info!(
        target = "MP4",
        output = %output.display(),
        preset = ?plan.preset,
        target_video_bps = resolved.target_video_bps,
        source_video_bps = ?resolved.source_video_bps,
        effective_video_bps = resolved.effective_video_bps,
        was_capped = resolved.was_capped,
        "开始转码"
    );
    let scale = format!("scale=-2:{}", plan.video_height);
    let effective_video = resolved.effective_video_str();
    let video_bufsize = resolved.video_bufsize_str();
    let effective_audio = resolved.effective_audio_str();
    let input_str = input
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("输入路径不是有效的 UTF-8"))?;
    let output_str = output
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("输出路径不是有效的 UTF-8"))?;
    run_ffmpeg(
        "MP4",
        &[
            "-y",
            "-i",
            input_str,
            "-vf",
            &scale,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-b:v",
            &effective_video,
            "-maxrate",
            &effective_video,
            "-bufsize",
            &video_bufsize,
            "-c:a",
            "aac",
            "-b:a",
            &effective_audio,
            output_str,
        ],
        progress,
    )
    .await?;
    let elapsed_secs = started.elapsed().as_secs_f64();
    let output_bytes = fs::metadata(output).await.map(|m| m.len()).unwrap_or(0);
    tracing::info!(
        target = "MP4",
        output = %output.display(),
        effective_video_bps = resolved.effective_video_bps,
        was_capped = resolved.was_capped,
        output_bytes,
        elapsed_secs,
        "转码完成"
    );
    Ok(TranscodeOutput {
        preview_path: output.to_path_buf(),
        download_path: output.to_path_buf(),
    })
}

async fn transcode_hls(
    input: &Path,
    zip_output: &Path, // 最终的 .zip 输出(用户下载用)
    work_dir: &Path,   // 放 m3u8 + ts 分片的工作目录
    plan: &OutputPlan,
    resolved: &ResolvedBitrates,
    progress: Option<ProgressFn>,
) -> anyhow::Result<TranscodeOutput> {
    let started = Instant::now();
    tracing::info!(
        target = "HLS",
        output = %zip_output.display(),
        work_dir = %work_dir.display(),
        preset = ?plan.preset,
        target_video_bps = resolved.target_video_bps,
        source_video_bps = ?resolved.source_video_bps,
        effective_video_bps = resolved.effective_video_bps,
        was_capped = resolved.was_capped,
        "开始转码"
    );
    let playlist = work_dir.join("stream.m3u8");
    let files_dir = work_dir.join("files");
    fs::create_dir_all(&files_dir).await?;
    let segment = files_dir.join("segment_%03d.ts");
    let scale = format!("scale=-2:{}", plan.video_height);
    let hls_time = plan.hls_segment_seconds.unwrap_or(6).to_string();
    let effective_video = resolved.effective_video_str();
    let video_bufsize = resolved.video_bufsize_str();
    let effective_audio = resolved.effective_audio_str();
    let input_str = input
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("输入路径不是有效的 UTF-8"))?;
    let playlist_str = playlist
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("HLS 播放列表路径不是有效的 UTF-8"))?;
    let segment_str = segment
        .as_os_str()
        .to_str()
        .ok_or_else(|| anyhow!("HLS 分片路径不是有效的 UTF-8"))?;
    run_ffmpeg(
        "HLS",
        &[
            "-y",
            "-i",
            input_str,
            "-vf",
            &scale,
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-b:v",
            &effective_video,
            "-maxrate",
            &effective_video,
            "-bufsize",
            &video_bufsize,
            "-c:a",
            "aac",
            "-b:a",
            &effective_audio,
            "-f",
            "hls",
            "-hls_time",
            &hls_time,
            "-hls_segment_filename",
            segment_str,
            playlist_str,
        ],
        progress,
    )
    .await?;
    write_hls_readme(work_dir).await?;
    zip_hls_package(work_dir, zip_output)?;
    let elapsed_secs = started.elapsed().as_secs_f64();
    let output_bytes = dir_size(work_dir).await;
    tracing::info!(
        target = "HLS",
        zip_output = %zip_output.display(),
        effective_video_bps = resolved.effective_video_bps,
        was_capped = resolved.was_capped,
        output_bytes,
        elapsed_secs,
        "转码完成"
    );
    Ok(TranscodeOutput {
        preview_path: playlist,
        download_path: zip_output.to_path_buf(),
    })
}

/// 递归累加目录下所有文件大小。HLS 由 m3u8 + 多个 .ts 分片组成,用总大小做日志
async fn dir_size(p: &Path) -> u64 {
    let mut stack = vec![p.to_path_buf()];
    let mut total = 0u64;
    while let Some(dir) = stack.pop() {
        let mut read = match fs::read_dir(&dir).await {
            Ok(r) => r,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = read.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = entry.metadata().await {
                total += meta.len();
            }
        }
    }
    total
}

async fn run_ffmpeg(
    target_label: &str,
    args: &[&str],
    progress: Option<ProgressFn>,
) -> anyhow::Result<()> {
    // 在 args 末尾加 -progress pipe:1 -nostats
    // -progress pipe:1: 把 key=value 进度报告写到 stdout
    // -nostats: 不在 stderr 写统计行(避免和 progress 重复,更易解析)
    let mut full_args: Vec<&str> = args.to_vec();
    full_args.extend_from_slice(&["-progress", "pipe:1", "-nostats"]);

    let mut child = Command::new(crate::services::ffmpeg_binary::ffmpeg_path())
        .args(&full_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("启动 FFmpeg 失败(目标:{target_label})"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("无法获取 ffmpeg stdout"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("无法获取 ffmpeg stderr"))?;

    // 进度读取任务:解析 -progress pipe:1 的 key=value 行
    // job_id / total_duration / emit 都在回调闭包里,read_progress 只负责调
    let progress_task = tokio::spawn(read_progress(stdout, progress));

    // 错误信息收集:读 stderr 到结束,截取尾部
    let stderr_task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut buf = Vec::with_capacity(8 * 1024);
        let _ = stderr.read_to_end(&mut buf).await;
        buf
    });

    let status = child.wait().await?;
    let _ = progress_task.await;
    let stderr_bytes = stderr_task.await.unwrap_or_default();
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    if status.success() {
        return Ok(());
    }

    let tail = stderr
        .chars()
        .rev()
        .take(FFMPEG_STDERR_TAIL_BYTES)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    tracing::error!(
        target = target_label,
        exit_code = ?status.code(),
        stderr_tail = %tail,
        "FFmpeg 转码失败"
    );
    anyhow::bail!(
        "FFmpeg 生成 {target_label} 失败(exit={:?}): {}",
        status.code(),
        tail.trim()
    )
}

/// 读 ffmpeg `-progress pipe:1` 输出,逐 report 调 progress 回调
/// 回调由 process_job_inner 构造,负责 emit Tauri 事件(已知 job_id 和 AppHandle)
async fn read_progress(stdout: tokio::process::ChildStdout, progress: Option<ProgressFn>) {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let mut reader = BufReader::new(stdout).lines();
    let mut out_time_us: u64 = 0;
    let mut speed_str = String::new();

    while let Ok(Some(line)) = reader.next_line().await {
        if let Some(rest) = line.strip_prefix("out_time_us=") {
            out_time_us = rest.trim().parse().unwrap_or(0);
        } else if let Some(rest) = line.strip_prefix("speed=") {
            speed_str = rest.trim().trim_end_matches('x').to_string();
        } else if line == "progress=continue" || line == "progress=end" {
            let encoded_secs = out_time_us as f64 / 1_000_000.0;
            let speed: f64 = speed_str.parse().unwrap_or(0.0);

            if let Some(cb) = &progress {
                cb(TranscodeProgress {
                    encoded_secs,
                    speed,
                });
            }

            if line == "progress=end" {
                break;
            }
        }
    }
}

async fn write_hls_readme(output_dir: &Path) -> anyhow::Result<()> {
    fs::write(
        output_dir.join("README.txt"),
        "这是 RustVid 生成的 HLS/m3u8 文件包。请把 stream.m3u8、README.txt 和 files/ 目录一起部署。\n",
    )
    .await
    .context("写入 HLS 说明失败")
}

fn zip_hls_package(output_dir: &Path, zip_path: &Path) -> anyhow::Result<()> {
    let file = std::fs::File::create(zip_path).context("创建 HLS zip 失败")?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    add_dir_to_zip(output_dir, output_dir, zip_path, &mut zip, options)?;
    zip.finish()?;
    Ok(())
}

fn add_dir_to_zip(
    root: &Path,
    dir: &Path,
    zip_path: &Path,
    zip: &mut zip::ZipWriter<std::fs::File>,
    options: zip::write::SimpleFileOptions,
) -> anyhow::Result<()> {
    for entry in std::fs::read_dir(dir).context("读取 HLS 目录失败")? {
        let entry = entry?;
        let path = entry.path();
        if path == zip_path {
            continue;
        }
        if path.is_dir() {
            add_dir_to_zip(root, &path, zip_path, zip, options)?;
        } else if path.is_file() {
            let name = path
                .strip_prefix(root)?
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("HLS 文件名无效"))?;
            zip.start_file(name, options)?;
            let mut input = std::fs::File::open(&path)?;
            std::io::copy(&mut input, zip)?;
        }
    }
    Ok(())
}
