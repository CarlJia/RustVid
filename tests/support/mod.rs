use std::{path::Path, sync::Arc};

use async_trait::async_trait;
use rustvid::{
    app::{self, AppState},
    config::Config,
    domain::preset::{OutputPlan, OutputTarget},
    persistence::sqlite::Database,
    services::ffmpeg::{TranscodeOutput, Transcoder},
};
use tempfile::TempDir;
use tokio::fs;

pub struct TestApp {
    pub state: AppState,
    pub _tmp: TempDir,
}

pub async fn app_with_fake_transcoder(fail: bool) -> TestApp {
    let tmp = tempfile::tempdir().expect("创建临时目录失败");
    let config = Config::new(tmp.path().to_path_buf()).expect("创建测试配置失败");
    let db = Database::in_memory().expect("创建测试数据库失败");
    let state = app::AppState::for_test(config, db, Arc::new(TestTranscoder { fail }))
        .await
        .expect("创建测试应用失败");
    TestApp { state, _tmp: tmp }
}

pub struct TestTranscoder {
    pub fail: bool,
}

#[async_trait]
impl Transcoder for TestTranscoder {
    async fn transcode(
        &self,
        _input: &Path,
        output: &Path,
        work_dir: &Path,
        plan: &OutputPlan,
        _progress: Option<rustvid::services::ffmpeg::ProgressFn>,
    ) -> anyhow::Result<TranscodeOutput> {
        if self.fail {
            anyhow::bail!("模拟 FFmpeg 失败");
        }
        fs::create_dir_all(work_dir).await?;
        match plan.target {
            OutputTarget::Mp4 => {
                fs::write(output, b"fake mp4").await?;
                Ok(TranscodeOutput {
                    preview_path: output.to_path_buf(),
                    download_path: output.to_path_buf(),
                })
            }
            OutputTarget::Hls => {
                let playlist = work_dir.join("stream.m3u8");
                let segment = work_dir.join("segment_000.ts");
                let zip = output;
                fs::write(&playlist, b"#EXTM3U\n").await?;
                fs::write(&segment, b"fake segment").await?;
                fs::write(&zip, b"fake zip").await?;
                Ok(TranscodeOutput {
                    preview_path: playlist,
                    download_path: zip.to_path_buf(),
                })
            }
        }
    }
}
