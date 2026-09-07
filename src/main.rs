use kube::Client;
use tokio_util::sync::CancellationToken;

use crate::config::Config;

pub mod config;
pub mod terminator;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let token = CancellationToken::new();
    let signal_token = token.clone();
    tokio::spawn(async move {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigterm.recv() => tracing::info!("received SIGTERM"),
            _ = tokio::signal::ctrl_c() => tracing::info!("received SIGINT"),
        }
        signal_token.cancel();
    });

    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,kube_client::client::builder=off")
            }),
        )
        .init();

    let cfg = Config::load()?;
    tracing::info!(pod = %cfg.pod_name, namespace = %cfg.pod_namespace, "starting istio-proxy-terminator");

    let client = Client::try_default().await?;
    let http_client = reqwest::Client::new();

    tokio::select! {
        _ = token.cancelled() => tracing::info!("shutdown requested, pod is being terminated externally, exiting"),
        result = terminator::run(&client, &http_client, &cfg) => result?,
    };

    tracing::info!("i'll be back");

    Ok(())
}
