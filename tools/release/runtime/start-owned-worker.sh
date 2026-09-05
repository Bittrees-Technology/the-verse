#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail
runtime_directory="$(cd "$(dirname "$0")" && pwd)"
universe_directory="$1"
local_port="${VERSE_LOCAL_PORT:-7777}"
if [[ ! "${local_port}" =~ ^[0-9]+$ ]] || ((local_port < 1024 || local_port > 65535)); then
  echo "Invalid local server port" >&2
  exit 1
fi
mkdir -p "${universe_directory}"
printf '%s\n' "$$" >"${universe_directory}/owned-worker.pid"
export RUST_LOG="${VERSE_SERVER_LOG_LEVEL:-verse_simulation_worker=info,tower_http=info}"
export VERSE_BROWSER_VERIFIER_ASSET_DIR="${runtime_directory}/browser-verifier"
exec "${runtime_directory}/verse-simulation-worker" \
  --data-directory "${universe_directory}" --genesis-profile capital-start \
  --snapshot-every 600 --bind "127.0.0.1:${local_port}" \
  >>"${universe_directory}/server.log" 2>&1
