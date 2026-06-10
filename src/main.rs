use std::net::SocketAddr;

use anyhow::Context;
use rustvid::{app, config};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "rustvid=info,tower_http=info".into()),
        )
        .init();

    let config = config::Config::from_env()?;
    let state = app::AppState::new(config).await?;
    let router = app::router(state);
    let addr: SocketAddr = "127.0.0.1:3000".parse().context("监听地址无效")?;
    let listener = TcpListener::bind(addr).await.context("绑定监听端口失败")?;
    tracing::info!("RustVid 正在监听 http://{addr}");
    axum::serve(listener, router)
        .await
        .context("HTTP 服务异常退出")?;
    Ok(())
}
