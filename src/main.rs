use anyhow::Context;
use baby_burrito::{AppState, auth::apple::AppleIdentityVerifier, build_router, config::Config, db};
use tokio::{net::TcpListener, signal};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .init();

    let config = Config::from_environment()?;
    let pool = db::open_file_database(&config.database_path).await?;
    let apple_verifier = AppleIdentityVerifier::for_apple(&config.apple_bundle_ids)?;
    let router = build_router(AppState::new(pool.clone(), apple_verifier));

    let listener = TcpListener::bind(config.bind_address)
        .await
        .with_context(|| format!("binding {}", config.bind_address))?;

    tracing::info!(address = %config.bind_address, database = %config.database_path.display(), "listening");

    axum::serve(listener, router)
        .with_graceful_shutdown(wait_for_shutdown_signal())
        .await
        .context("serving HTTP")?;

    pool.close().await;

    Ok(())
}

async fn wait_for_shutdown_signal() {
    let interrupt = async {
        signal::ctrl_c().await.expect("installing Ctrl+C handler");
    };

    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("installing SIGTERM handler")
            .recv()
            .await;
    };

    tokio::select! {
        _ = interrupt => {},
        _ = terminate => {},
    }

    tracing::info!("shutting down");
}
