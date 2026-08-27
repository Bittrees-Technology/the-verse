// SPDX-License-Identifier: AGPL-3.0-or-later

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use verse_simulation::Runtime;
use verse_simulation_worker::{AppState, router};

#[derive(Debug, Parser)]
#[command(
    name = "verse-simulation-worker",
    about = "The Verse authoritative local P0 universe"
)]
struct Arguments {
    #[arg(long, env = "VERSE_BIND", default_value = "127.0.0.1:7777")]
    bind: SocketAddr,

    #[arg(long, env = "VERSE_DATA_DIR", default_value = "data/local-universe")]
    data_directory: PathBuf,

    #[arg(long, env = "VERSE_WORLD_SEED", default_value_t = 20260826)]
    world_seed: u64,

    #[arg(long, env = "VERSE_SNAPSHOT_EVERY", default_value_t = 25)]
    snapshot_every: u64,

    #[arg(long, env = "VERSE_TICK_MS", default_value_t = 16)]
    tick_millis: u16,

    /// Open the authoritative state for recovery inspection without advancing time.
    #[arg(long, env = "VERSE_PAUSE_SIMULATION", default_value_t = false)]
    pause_simulation: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("verse_simulation_worker=info,tower_http=info")),
        )
        .with_target(false)
        .compact()
        .init();

    let arguments = Arguments::parse();
    let runtime = Runtime::open(
        &arguments.data_directory,
        arguments.world_seed,
        arguments.snapshot_every,
    )
    .with_context(|| {
        format!(
            "failed to open universe data at {}",
            arguments.data_directory.display()
        )
    })?;
    let state = AppState::new(runtime);
    let tick_task = if arguments.pause_simulation {
        None
    } else {
        let tick_state = Arc::clone(&state);
        // The native client samples controls at 60 Hz. Keeping the worker's
        // scheduling gap below one client physics frame prevents a valid tap
        // or mouse impulse from being overwritten by its following neutral
        // sample before any authoritative substep can observe it.
        let tick_millis = arguments.tick_millis.clamp(1, 16);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(u64::from(tick_millis)));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if let Err(source) = tick_state.advance(tick_millis) {
                    error!(%source, "authoritative simulation tick failed");
                    break;
                }
            }
        }))
    };

    let listener = TcpListener::bind(arguments.bind)
        .await
        .with_context(|| format!("failed to bind {}", arguments.bind))?;
    info!(
        address = %arguments.bind,
        data = %arguments.data_directory.display(),
        simulation_paused = arguments.pause_simulation,
        "The Verse local universe is ready"
    );
    axum::serve(listener, router(Arc::clone(&state)))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("simulation HTTP server failed")?;

    if let Some(tick_task) = tick_task {
        tick_task.abort();
    }
    state
        .persist_snapshot()
        .context("failed to persist shutdown snapshot")?;
    info!("authoritative shutdown snapshot persisted");
    Ok(())
}

async fn shutdown_signal() {
    let control_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install terminate handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = control_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_tick_defaults_to_less_than_one_sixty_hz_frame() {
        let arguments = Arguments::parse_from(["verse-simulation-worker"]);
        assert_eq!(arguments.tick_millis, 16);
        assert!(f64::from(arguments.tick_millis) < 1_000.0 / 60.0);
    }
}
