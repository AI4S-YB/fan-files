mod api;
mod config;
mod db;
mod error;
mod models;
mod state;

use clap::Parser;
use config::{Args, Settings};
use state::AppState;
use std::{sync::Arc, time::Duration};
use tower::ServiceBuilder;
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{Any, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let settings = Settings::load(Args::parse())?;
    let state = Arc::new(AppState::new(settings.clone())?);
    let request_id = http::HeaderName::from_static("x-request-id");
    // 注意：请求级超时（TimeoutLayer）按路由应用在 api::router 内——
    // chat-search 要调 LLM（可能数秒），不能套普通端点的 5s 上限
    let app = api::router(state)
        // CORS：Tauri WebView（origin=tauri://localhost / http://localhost）fetch
        // 本服务是跨源请求，必须放行——否则前端所有 HTTP API 调用被 WebView 拦截
        // （实测症状：toast 无数据集数、首页统计空、数据集列表空、搜索失败，
        //  而 Tauri 命令路径如传输历史正常）。
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::new(request_id.clone(), MakeRequestUuid))
                .layer(PropagateRequestIdLayer::new(request_id))
                .layer(TraceLayer::new_for_http())
                .layer(CatchPanicLayer::new()),
        );

    let listener = tokio::net::TcpListener::bind(settings.bind).await?;
    info!(address = %settings.bind, database = %settings.database.display(), "fan-files-share listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = terminate => {} }
}
