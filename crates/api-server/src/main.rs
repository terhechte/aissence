mod api;
mod auth;
mod config;
mod db;
mod error;
mod state;
mod worker;

use std::sync::Arc;

use aissence::Summarizer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "aissence_server=info".into()),
        )
        .init();

    let config = Arc::new(Config::from_env()?);
    let pool = db::create_pool(&config.database_url)?;
    db::migrate(&pool).await?;
    let summarizer = Arc::new(
        Summarizer::from_env()?
            .with_max_request_retries(worker::HOSTED_REQUEST_RETRIES)
            .with_hosted_policy(),
    );
    let state = AppState {
        config: config.clone(),
        pool,
        summarizer,
    };

    tokio::spawn(worker::run(state.clone()));
    let app = api::router(state)
        .layer(RequestBodyLimitLayer::new(config.max_upload_bytes))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = %request.uri().path()
                )
            }),
        );
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    tracing::info!(address = %config.bind, "aissence server listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
