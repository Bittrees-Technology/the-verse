// SPDX-License-Identifier: AGPL-3.0-or-later

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use verse_protocol::CellKeyV1;
use verse_simulation::{LifecycleMode, LocalTwoCellRuntime, Runtime, cell_origin_key};
use verse_simulation_worker::{AppState, router};

#[derive(Debug, Parser)]
#[command(
    name = "verse-simulation-worker",
    about = "The Verse authoritative simulation cell"
)]
struct Arguments {
    #[arg(long, env = "VERSE_BIND", default_value = "127.0.0.1:7777")]
    bind: SocketAddr,

    #[arg(long, env = "VERSE_DATA_DIR", default_value = "data/local-universe")]
    data_directory: PathBuf,

    /// Canonical `CellKeyV1` JSON for this worker. Omission selects the origin cell.
    #[arg(long, env = "VERSE_CELL_KEY_JSON")]
    cell_key_json: Option<String>,

    #[arg(long, env = "VERSE_WORLD_SEED", default_value_t = 20260826)]
    world_seed: u64,

    #[arg(long, env = "VERSE_SNAPSHOT_EVERY", default_value_t = 25)]
    snapshot_every: u64,

    #[arg(long, env = "VERSE_TICK_MS", default_value_t = 16)]
    tick_millis: u16,

    /// Seconds without an authenticated player before the active cell drains.
    #[arg(long, env = "VERSE_IDLE_DRAIN_SECONDS", default_value_t = 30)]
    idle_drain_seconds: u64,

    /// Comma-separated loopback-only development actors pre-admitted before
    /// the first event. Gameplay hello messages can bind but never create one.
    #[arg(
        long,
        env = "VERSE_DEVELOPMENT_PLAYERS",
        value_delimiter = ',',
        default_value = "player-remote"
    )]
    development_players: Vec<String>,

    /// Open the authoritative state for recovery inspection without advancing time.
    #[arg(long, env = "VERSE_PAUSE_SIMULATION", default_value_t = false)]
    pause_simulation: bool,

    /// Run the bounded directory-managed adjacent-cell universe and route
    /// sessions through its reconciled coordinator.
    #[arg(long, env = "VERSE_TWO_CELL_UNIVERSE", default_value_t = false)]
    two_cell_universe: bool,
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
    if !arguments.bind.ip().is_loopback() {
        bail!(
            "protocol 11 local-development player authentication is restricted to a loopback bind; use 127.0.0.1 or wait for the configured session authority"
        );
    }
    let state = if arguments.two_cell_universe {
        if arguments.pause_simulation || arguments.cell_key_json.is_some() {
            bail!(
                "the two-cell universe owns both cell routes and does not accept pause or a standalone cell key"
            );
        }
        let coordinator = LocalTwoCellRuntime::open(
            &arguments.data_directory,
            arguments.world_seed,
            arguments.snapshot_every,
            "simulation-gateway",
        )
        .with_context(|| {
            format!(
                "failed to reconcile the two-cell universe at {}",
                arguments.data_directory.display()
            )
        })?;
        for player_id in &arguments.development_players {
            if coordinator.runtime_for_player(player_id).is_err() {
                info!(%player_id, "two-cell gateway ignores development actors without a resident placement");
            }
        }
        AppState::new_two_cell(coordinator)
    } else {
        let cell_key = arguments.cell_key_json.as_deref().map_or_else(
            || Ok(cell_origin_key()),
            |json| {
                serde_json::from_str::<CellKeyV1>(json).context("VERSE_CELL_KEY_JSON is invalid")
            },
        )?;
        let mut runtime = if arguments.pause_simulation {
            Runtime::open_for_cell(
                &arguments.data_directory,
                arguments.world_seed,
                cell_key,
                arguments.snapshot_every,
            )
        } else {
            Runtime::open_hosted_for_cell(
                &arguments.data_directory,
                arguments.world_seed,
                cell_key,
                arguments.snapshot_every,
            )
        }
        .with_context(|| {
            format!(
                "failed to open universe data at {}",
                arguments.data_directory.display()
            )
        })?;
        if runtime.state().player.by_id.is_empty() {
            if !arguments.development_players.is_empty() {
                info!(
                    "empty frontier cells ignore development-player admission until a canonical transfer arrives"
                );
            }
        } else {
            for player_id in &arguments.development_players {
                runtime
                    .admit_development_player(player_id)
                    .with_context(|| {
                        format!("failed to pre-admit development player {player_id}")
                    })?;
            }
        }
        AppState::new(runtime)
    };
    let lifecycle_task = if arguments.pause_simulation {
        let lease_state = Arc::clone(&state);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Err(source) = lease_state.renew_lease() {
                    error!(%source, "authoritative lease renewal failed; worker is fenced");
                    break;
                }
            }
        }))
    } else {
        let lifecycle_state = Arc::clone(&state);
        let tick_millis = arguments.tick_millis;
        let idle_drain_after = Duration::from_secs(arguments.idle_drain_seconds);
        Some(tokio::spawn(async move {
            if let Err(source) = lifecycle_state
                .supervise(tick_millis, idle_drain_after)
                .await
            {
                error!(%source, "authoritative lifecycle supervisor failed");
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

    if let Some(lifecycle_task) = lifecycle_task {
        lifecycle_task.abort();
    }
    match state.lifecycle_mode() {
        LifecycleMode::Active => {
            state
                .drain_to_background_or_sleeping()
                .context("failed to drain the shutdown cell")?;
            info!("authoritative shutdown drain persisted");
        }
        LifecycleMode::Background | LifecycleMode::Activating | LifecycleMode::Draining => {
            state
                .persist_snapshot()
                .context("failed to persist shutdown snapshot")?;
            info!("authoritative shutdown snapshot persisted");
        }
        LifecycleMode::Sleeping => info!("sleeping cell already persisted and released"),
    }
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
    fn authoritative_tick_defaults_to_at_most_one_sixty_hz_frame() {
        let arguments = Arguments::parse_from(["verse-simulation-worker"]);
        assert_eq!(arguments.tick_millis, 16);
        assert_eq!(arguments.idle_drain_seconds, 30);
        assert_eq!(arguments.development_players, ["player-remote"]);
        assert!(arguments.cell_key_json.is_none());
        assert!(f64::from(arguments.tick_millis) <= 1_000.0 / 60.0);
    }
}
