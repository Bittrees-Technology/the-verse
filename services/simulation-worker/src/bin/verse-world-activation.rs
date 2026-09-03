// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use verse_simulation::{
    Protocol19ActivationTrustPolicy, activate_protocol19_world, open_activated_protocol19_world,
    prepare_protocol19_for_activation,
};

const MAX_POLICY_BYTES: u64 = 64 * 1_024;
const MAX_AUTHORIZATION_BYTES: u64 = 256 * 1_024;

#[derive(Debug, Parser)]
#[command(
    name = "verse-world-activation",
    about = "Prepare, activate, or verify one protocol-19 universe"
)]
struct Arguments {
    #[arg(long, env = "VERSE_DATA_DIR", default_value = "data/local-universe")]
    data_directory: PathBuf,

    #[arg(long, env = "VERSE_WORLD_SEED", default_value_t = 20260826)]
    world_seed: u64,

    #[command(subcommand)]
    command: ActivationCommand,
}

#[derive(Debug, Subcommand)]
enum ActivationCommand {
    /// Prepare or strictly reopen the dormant target and print its signed fields.
    Prepare,
    /// Commit the exact signed authorization and global active head.
    Activate {
        #[arg(long)]
        policy: PathBuf,
        #[arg(long)]
        policy_hash: String,
        #[arg(long)]
        authorization: PathBuf,
    },
    /// Verify the committed global head and every selected target artifact.
    Verify {
        #[arg(long)]
        policy: PathBuf,
        #[arg(long)]
        policy_hash: String,
    },
}

fn main() -> Result<()> {
    let arguments = Arguments::parse();
    let output = match arguments.command {
        ActivationCommand::Prepare => {
            let prepared =
                prepare_protocol19_for_activation(&arguments.data_directory, arguments.world_seed)
                    .with_context(|| {
                        format!(
                            "failed to prepare protocol-19 world at {}",
                            arguments.data_directory.display()
                        )
                    })?;
            serde_json::to_vec(&prepared).context("failed to encode prepared-world summary")?
        }
        ActivationCommand::Activate {
            policy,
            policy_hash,
            authorization,
        } => {
            let policy = load_policy(&policy, &policy_hash)?;
            let authorization = read_bounded(&authorization, MAX_AUTHORIZATION_BYTES)
                .context("failed to read signed activation authorization")?;
            let activated = activate_protocol19_world(
                &arguments.data_directory,
                arguments.world_seed,
                &policy,
                &authorization,
            )
            .with_context(|| {
                format!(
                    "failed to activate protocol-19 world at {}",
                    arguments.data_directory.display()
                )
            })?;
            serde_json::to_vec(&activated).context("failed to encode activation summary")?
        }
        ActivationCommand::Verify {
            policy,
            policy_hash,
        } => {
            let policy = load_policy(&policy, &policy_hash)?;
            let activated = open_activated_protocol19_world(
                &arguments.data_directory,
                arguments.world_seed,
                &policy,
            )
            .with_context(|| {
                format!(
                    "failed to verify protocol-19 world at {}",
                    arguments.data_directory.display()
                )
            })?;
            serde_json::to_vec(activated.summary())
                .context("failed to encode verified activation summary")?
        }
    };
    println!(
        "{}",
        String::from_utf8(output).context("activation summary was not UTF-8")?
    );
    Ok(())
}

fn load_policy(path: &Path, expected_hash: &str) -> Result<Protocol19ActivationTrustPolicy> {
    let bytes = read_bounded(path, MAX_POLICY_BYTES).context("failed to read trust policy")?;
    Protocol19ActivationTrustPolicy::from_canonical_bytes(&bytes, expected_hash)
        .context("activation trust policy failed verification")
}

fn read_bounded(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("failed to open {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > maximum {
        bail!("{} is not a bounded nonempty file", path.display());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {}", path.display()))?;
    if bytes.is_empty() || u64::try_from(bytes.len()).unwrap_or(u64::MAX) > maximum {
        bail!(
            "{} changed size or exceeded its bound while reading",
            path.display()
        );
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_modes_are_explicit() {
        let prepare = Arguments::parse_from(["verse-world-activation", "prepare"]);
        assert!(matches!(prepare.command, ActivationCommand::Prepare));

        let verify = Arguments::parse_from([
            "verse-world-activation",
            "verify",
            "--policy",
            "policy.json",
            "--policy-hash",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]);
        assert!(matches!(verify.command, ActivationCommand::Verify { .. }));
    }
}
