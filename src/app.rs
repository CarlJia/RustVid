use axum::{
    Router,
    routing::{get, post},
};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::{
    config::Config,
    http::{assets, jobs, pages, uploads},
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
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        let db = Database::open(&config.database_path())?;
        let state = build_state(config, db).await?;
        spawn_worker(state.queue.clone());
        Ok(state)
    }

    pub async fn for_test(
        config: Config,
        db: Database,
        transcoder: crate::services::ffmpeg::SharedTranscoder,
    ) -> anyhow::Result<Self> {
        build_state_with_transcoder(config, db, transcoder).await
    }
}

async fn build_state(config: Config, db: Database) -> anyhow::Result<AppState> {
    build_state_with_transcoder(config, db, shared_transcoder(FfmpegTranscoder)).await
}

async fn build_state_with_transcoder(
    config: Config,
    db: Database,
    transcoder: crate::services::ffmpeg::SharedTranscoder,
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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(pages::index))
        .route("/jobs/{id}", get(pages::job_page))
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/presets", get(jobs::presets))
        .route("/api/usage", get(jobs::usage))
        .route("/api/uploads", post(uploads::create_upload))
        .route(
            "/api/uploads/{id}",
            get(uploads::head_upload)
                .head(uploads::head_upload)
                .patch(uploads::patch_upload),
        )
        .route("/api/jobs", get(jobs::list_jobs).post(jobs::create_job))
        .route(
            "/api/jobs/{id}",
            get(jobs::get_job).delete(jobs::delete_job),
        )
        .route("/api/jobs/{id}/retry", post(jobs::retry_job))
        .route("/api/jobs/process-next", post(jobs::process_next))
        .route("/assets/{artifact_id}/preview", get(assets::preview))
        .route("/assets/{artifact_id}/files/{file}", get(assets::hls_file))
        .route("/assets/{artifact_id}/download", get(assets::download))
        .nest_service("/static", ServeDir::new("src/ui/static"))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn spawn_worker(queue: JobQueue) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
        loop {
            interval.tick().await;
            if let Err(err) = queue.process_one().await {
                tracing::warn!("后台转码轮询失败: {err}");
            }
        }
    });
}
