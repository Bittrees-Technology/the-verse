// SPDX-License-Identifier: AGPL-3.0-or-later

use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use axum::{Json, Router, extract::State, routing::get};
use clap::{Parser, ValueEnum};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use verse_protocol::CellKeyV1;
use verse_simulation::{
    ActivatedProtocol19World, LifecycleMode, LocalTwoCellRuntime, Protocol19ActivatedWorldSummary,
    Protocol19ActivationTrustPolicy, Runtime, cell_origin_key, open_activated_protocol19_world,
    protocol19_is_activated,
};
use verse_simulation_worker::{AppState, router};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum GenesisProfile {
    Orbital,
    EarthStart,
    OreWorkshop,
    CapitalStart,
}

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

    /// Development-only fixture selected before the first canonical event.
    #[arg(
        long,
        env = "VERSE_GENESIS_PROFILE",
        value_enum,
        default_value = "orbital"
    )]
    genesis_profile: GenesisProfile,

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

    /// Boot the activated protocol-19 world as a verified readiness service.
    /// Gameplay admission remains disabled until the protocol-19 runtime tuple
    /// is implemented.
    #[arg(long, env = "VERSE_PROTOCOL19_VERIFIED_BOOT", default_value_t = false)]
    protocol19_verified_boot: bool,

    /// Canonical activation trust-policy JSON. The file is not trusted without
    /// the separately configured expected hash.
    #[arg(long, env = "VERSE_PROTOCOL19_ACTIVATION_POLICY")]
    protocol19_activation_policy: Option<PathBuf>,

    /// Externally anchored BLAKE3 hash of the activation trust policy.
    #[arg(long, env = "VERSE_PROTOCOL19_ACTIVATION_POLICY_HASH")]
    protocol19_activation_policy_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct VerifiedProtocol19BootStatus {
    service: &'static str,
    status: &'static str,
    gameplay_session_admission: bool,
    activation: Protocol19ActivatedWorldSummary,
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
    if arguments.protocol19_verified_boot {
        return run_verified_protocol19_boot(&arguments).await;
    }
    if protocol19_is_activated(&arguments.data_directory)
        .context("failed to determine the active universe protocol")?
    {
        bail!(
            "the universe has an active protocol-19 head; legacy protocol-18 startup is fenced (use --protocol19-verified-boot with the externally anchored policy until interactive protocol-19 service is released)"
        );
    }
    let state = if arguments.two_cell_universe {
        if arguments.pause_simulation
            || arguments.cell_key_json.is_some()
            || arguments.genesis_profile != GenesisProfile::Orbital
        {
            bail!(
                "the two-cell universe owns both cell routes and does not accept pause, a standalone cell key, or a development genesis profile"
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
        let is_origin_cell = cell_key == cell_origin_key();
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
        if arguments.genesis_profile == GenesisProfile::CapitalStart {
            if !is_origin_cell {
                bail!("capital start requires the origin cell");
            }
            if runtime.state().event_sequence == 0 {
                runtime
                    .configure_capital_start()
                    .context("configure capital start")?;
            }
        }
        if arguments.genesis_profile == GenesisProfile::OreWorkshop {
            if !is_origin_cell {
                bail!("ore workshop requires the origin cell");
            }
            if runtime.state().event_sequence == 0 {
                runtime
                    .configure_ore_workshop()
                    .context("failed to configure ore workshop")?;
            }
        }
        if arguments.genesis_profile == GenesisProfile::EarthStart {
            if !is_origin_cell {
                bail!(
                    "the Earthlike surface playtest profile is available only in the origin cell"
                );
            }
            if runtime.state().event_sequence == 0 {
                let configured = runtime
                    .configure_earth_start_playtest()
                    .context("failed to configure the fresh Earthlike surface playtest")?;
                info!(configured, "Earthlike surface playtest profile is ready");
            }
        }
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
    let (failure_tx, mut failure_rx) = tokio::sync::watch::channel(false);
    let lifecycle_task = if arguments.pause_simulation {
        let lease_state = Arc::clone(&state);
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if let Err(source) = lease_state.renew_lease() {
                    error!(%source, "authoritative lease renewal failed; worker is fenced");
                    let _ = failure_tx.send(true);
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
                let _ = failure_tx.send(true);
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
        .with_graceful_shutdown(async move {
            tokio::select! { () = shutdown_signal() => {}, _ = failure_rx.wait_for(|failed| *failed) => {} }
        })
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

async fn run_verified_protocol19_boot(arguments: &Arguments) -> Result<()> {
    if arguments.pause_simulation
        || arguments.two_cell_universe
        || arguments.cell_key_json.is_some()
        || arguments.genesis_profile != GenesisProfile::Orbital
    {
        bail!(
            "verified protocol-19 boot derives its complete cell set from the active global head and does not accept legacy cell, fixture, player, pause, or two-cell options"
        );
    }
    let policy_path = arguments
        .protocol19_activation_policy
        .as_ref()
        .context("--protocol19-activation-policy is required for verified protocol-19 boot")?;
    let expected_policy_hash = arguments
        .protocol19_activation_policy_hash
        .as_deref()
        .context("--protocol19-activation-policy-hash is required for verified protocol-19 boot")?;
    let policy_bytes = read_bounded_policy(policy_path)?;
    let policy =
        Protocol19ActivationTrustPolicy::from_canonical_bytes(&policy_bytes, expected_policy_hash)
            .context("protocol-19 activation policy failed verification")?;
    let activated = Arc::new(
        open_activated_protocol19_world(&arguments.data_directory, arguments.world_seed, &policy)
            .with_context(|| {
            format!(
                "failed to boot the active protocol-19 universe at {}",
                arguments.data_directory.display()
            )
        })?,
    );
    let listener = TcpListener::bind(arguments.bind)
        .await
        .with_context(|| format!("failed to bind {}", arguments.bind))?;
    info!(
        address = %arguments.bind,
        data = %arguments.data_directory.display(),
        active_head = %activated.summary().active_head_hash,
        cells = activated.summary().cell_count,
        gameplay_session_admission = false,
        "verified protocol-19 universe is ready"
    );
    let app = Router::new()
        .route("/healthz", get(verified_protocol19_status))
        .route(
            "/api/v1/protocol19/activation",
            get(verified_protocol19_status),
        )
        .with_state(activated);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("verified protocol-19 readiness server failed")
}

async fn verified_protocol19_status(
    State(world): State<Arc<ActivatedProtocol19World>>,
) -> Json<VerifiedProtocol19BootStatus> {
    Json(VerifiedProtocol19BootStatus {
        service: "verse-simulation-worker",
        status: "verified_activation_ready",
        gameplay_session_admission: false,
        activation: world.summary().clone(),
    })
}

fn read_bounded_policy(path: &std::path::Path) -> Result<Vec<u8>> {
    const MAX_POLICY_BYTES: u64 = 64 * 1_024;
    let file = std::fs::File::open(path)
        .with_context(|| format!("failed to open activation policy at {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect activation policy at {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_POLICY_BYTES {
        bail!("activation policy is not a bounded nonempty file");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(MAX_POLICY_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read activation policy at {}", path.display()))?;
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_POLICY_BYTES {
        bail!("activation policy changed size or exceeded its bound while reading");
    }
    Ok(bytes)
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
        assert_eq!(arguments.genesis_profile, GenesisProfile::Orbital);
        assert!(!arguments.protocol19_verified_boot);
        assert!(arguments.protocol19_activation_policy.is_none());
        assert!(arguments.protocol19_activation_policy_hash.is_none());
        assert!(f64::from(arguments.tick_millis) <= 1_000.0 / 60.0);
    }

    #[test]
    fn earth_start_profile_is_explicitly_selectable() {
        let arguments = Arguments::parse_from([
            "verse-simulation-worker",
            "--genesis-profile",
            "earth-start",
        ]);
        assert_eq!(arguments.genesis_profile, GenesisProfile::EarthStart);
    }

    #[test]
    fn verified_protocol19_boot_requires_explicit_trust_configuration() {
        let arguments = Arguments::parse_from([
            "verse-simulation-worker",
            "--protocol19-verified-boot",
            "--protocol19-activation-policy",
            "/operator/policy.json",
            "--protocol19-activation-policy-hash",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]);
        assert!(arguments.protocol19_verified_boot);
        assert_eq!(
            arguments.protocol19_activation_policy,
            Some(PathBuf::from("/operator/policy.json"))
        );
        assert_eq!(
            arguments.protocol19_activation_policy_hash.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
    }
}
