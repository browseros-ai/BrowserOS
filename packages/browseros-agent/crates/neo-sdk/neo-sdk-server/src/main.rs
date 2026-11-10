//! Neo: Standalone development host for the same router claw-server mounts. The listener and
//! process signals belong here; the library owns browser requests and their cleanup.

use std::error::Error;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use neo_sdk_server::{Config, NeoSdkServer};
use tokio::net::TcpListener;
use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{self, SignalKind};
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "127.0.0.1:9310")]
    listen: SocketAddr,
    #[arg(long, default_value_t = 9110)]
    cdp_port: u16,
    /// Shared artifact and retained-result directory for this server instance.
    #[arg(long)]
    state_dir: PathBuf,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(io::stderr)
        .init();
    let listener = TcpListener::bind(args.listen).await?;
    let shutdown = CancellationToken::new();
    let server = NeoSdkServer::new(Config::new(args.cdp_port, args.state_dir), shutdown);
    let closing = server.clone();
    tracing::info!(address = %listener.local_addr()?, "Neo MCP server listening");
    let result = axum::serve(listener, server.router())
        .with_graceful_shutdown(async move {
            stop_signal().await;
            // Keep serving while runs cancel and their bodies drain. Only then end GET and
            // listen streams, so axum's connection drain cannot wait forever on those streams.
            closing.close_transport().await;
        })
        .await;
    server.shutdown().await;
    result?;
    Ok(())
}

async fn stop_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) = unix::signal(SignalKind::terminate()) {
        tokio::select! {
            _ = signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        return;
    }
    let _ = signal::ctrl_c().await;
}
