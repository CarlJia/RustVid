use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::{
    config::Config,
    persistence::sqlite::Database,
    services::{
        artifact_store::ArtifactStore,
        capacity::CapacityService,
        ffmpeg::FfmpegTranscoder,
        job_queue::{JobQueue, shared_transcoder},
        upload_sessions::UploadService,
    },
};

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub capacity: CapacityService,
    pub uploads: UploadService,
    pub artifacts: ArtifactStore,
    pub queue: JobQueue,
}

impl AppState {
    /// 生产入口:由 Tauri setup 调用,持有 AppHandle 用于 emit Tauri 事件
    pub async fn new(config: Config, app: AppHandle) -> anyhow::Result<Self> {
        let db = Database::open(&config.database_path())?;
        // 启动时回收上一次崩溃留下的"僵尸"Processing 任务 → 标记 Failed
        // 不重试,让用户自己决定是重试还是删除(避免误转码)
        match db.reap_stale_processing() {
            Ok(ids) if !ids.is_empty() => {
                tracing::warn!(
                    count = ids.len(),
                    "回收僵尸 Processing 任务(应用上次未正常退出): {ids:?}"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!("回收僵尸任务失败:{e}"),
        }
        let state = build_state(config, db, Some(app)).await?;
        // 后台预热:启动时就把 bundled ffmpeg 抽到 cache,避免首次转码延迟
        // + 让损坏/版本不一致的旧 cache 在启动时就被覆盖(extract_to_cache 内部有 magic 字节检查)
        tokio::spawn(async {
            let path = crate::services::ffmpeg_binary::ffmpeg_path();
            tracing::info!(path = %path.display(), "ffmpeg 预热完成");
        });
        spawn_worker(state.queue.clone());
        Ok(state)
    }

    /// 测试入口:不持有 AppHandle,进度回调不 emit 事件
    pub async fn for_test(
        config: Config,
        db: Database,
        transcoder: crate::services::ffmpeg::SharedTranscoder,
    ) -> anyhow::Result<Self> {
        build_state_with_transcoder(config, db, transcoder, None).await
    }
}

async fn build_state(
    config: Config,
    db: Database,
    app: Option<AppHandle>,
) -> anyhow::Result<AppState> {
    let transcoder = shared_transcoder(FfmpegTranscoder);
    build_state_with_transcoder(config, db, transcoder, app).await
}

async fn build_state_with_transcoder(
    config: Config,
    db: Database,
    transcoder: crate::services::ffmpeg::SharedTranscoder,
    app: Option<AppHandle>,
) -> anyhow::Result<AppState> {
    let artifacts = ArtifactStore::new(config.clone());
    artifacts.ensure_dirs().await?;
    let capacity = CapacityService::new(config.clone(), db.clone());
    let uploads = UploadService::new(db.clone(), capacity.clone(), config.uploads_dir());
    let queue = JobQueue::new(
        db.clone(),
        capacity.clone(),
        artifacts.clone(),
        transcoder,
        config.max_concurrent_transcodes,
        app,
    );
    Ok(AppState {
        config,
        db,
        capacity,
        uploads,
        artifacts,
        queue,
    })
}

fn spawn_worker(queue: JobQueue) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            interval.tick().await;
            if let Err(err) = queue.process_one().await {
                tracing::warn!("后台转码轮询失败: {err}");
            }
        }
    });
}

/// 初始化 tracing(东八区 +08:00 + Tauri 日志桥接)
/// 由 main.rs 在 tauri::Builder 启动前调用。
pub fn setup_logging() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        use time::UtcOffset;
        use time::format_description::well_known::Rfc3339;
        use tracing_subscriber::fmt::time::OffsetTime;

        let east8 = UtcOffset::from_hms(8, 0, 0).expect("构造 UTC+8 偏移量失败");
        let timer = OffsetTime::new(east8, Rfc3339);
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "rustvid=info".into()),
            )
            .with_timer(timer)
            .init();
    });
}

// 静默 unused 警告
#[allow(dead_code)]
const _INSTANT_TYPE_CHECK: Option<Instant> = None;
