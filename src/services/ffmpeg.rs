use std::{path::Path, process::Stdio, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use tokio::{fs, process::Command};

use crate::domain::preset::{OutputPlan, OutputTarget};

#[async_trait]
pub trait Transcoder: Send + Sync {
    async fn transcode(
        &self,
        input: &Path,
        output_dir: &Path,
        plan: &OutputPlan,
    ) -> anyhow::Result<TranscodeOutput>;
}

pub type SharedTranscoder = Arc<dyn Transcoder>;

#[derive(Clone, Debug)]
pub struct TranscodeOutput {
    pub preview_path: std::path::PathBuf,
    pub download_path: std::path::PathBuf,
}

#[derive(Default)]
pub struct FfmpegTranscoder;

#[async_trait]
impl Transcoder for FfmpegTranscoder {
    async fn transcode(
        &self,
        input: &Path,
        output_dir: &Path,
        plan: &OutputPlan,
    ) -> anyhow::Result<TranscodeOutput> {
        fs::create_dir_all(output_dir).await?;
        match plan.target {
            OutputTarget::Mp4 => transcode_mp4(input, output_dir, plan).await,
            OutputTarget::Hls => transcode_hls(input, output_dir, plan).await,
        }
    }
}

async fn transcode_mp4(
    input: &Path,
    output_dir: &Path,
    plan: &OutputPlan,
) -> anyhow::Result<TranscodeOutput> {
    let output = output_dir.join("output.mp4");
    let scale = format!("scale=-2:{}", plan.video_height);
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-vf", &scale])
        .args(["-c:v", "libx264", "-preset", "veryfast"])
        .args(["-b:v", plan.video_bitrate])
        .args(["-c:a", "aac", "-b:a", plan.audio_bitrate])
        .arg(&output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("启动 FFmpeg 失败")?;
    if !status.success() {
        anyhow::bail!("FFmpeg 生成 MP4 失败");
    }
    Ok(TranscodeOutput {
        preview_path: output.clone(),
        download_path: output,
    })
}

async fn transcode_hls(
    input: &Path,
    output_dir: &Path,
    plan: &OutputPlan,
) -> anyhow::Result<TranscodeOutput> {
    let playlist = output_dir.join("stream.m3u8");
    let files_dir = output_dir.join("files");
    fs::create_dir_all(&files_dir).await?;
    let segment = files_dir.join("segment_%03d.ts");
    let zip_path = output_dir.join("hls-package.zip");
    let scale = format!("scale=-2:{}", plan.video_height);
    let hls_time = plan.hls_segment_seconds.unwrap_or(6).to_string();
    let status = Command::new("ffmpeg")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .args(["-vf", &scale])
        .args(["-c:v", "libx264", "-preset", "veryfast"])
        .args(["-b:v", plan.video_bitrate])
        .args(["-c:a", "aac", "-b:a", plan.audio_bitrate])
        .args(["-f", "hls", "-hls_time", &hls_time])
        .arg("-hls_segment_filename")
        .arg(&segment)
        .arg(&playlist)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .status()
        .await
        .context("启动 FFmpeg 失败")?;
    if !status.success() {
        anyhow::bail!("FFmpeg 生成 HLS 失败");
    }
    write_hls_readme(output_dir).await?;
    zip_hls_package(output_dir, &zip_path)?;
    Ok(TranscodeOutput {
        preview_path: playlist,
        download_path: zip_path,
    })
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
