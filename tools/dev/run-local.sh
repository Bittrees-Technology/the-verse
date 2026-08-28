#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
player_id="${VERSE_PLAYER_ID:-player-local}"
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill -TERM "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

cd "${verse_root}"
if [[ ! -x "${godot_binary}" ]]; then
  tools/dev/bootstrap-macos.sh
fi
cargo build -p verse-simulation-worker
mkdir -p artifacts
target/debug/verse-simulation-worker \
  --data-directory data/local-universe \
  --bind 127.0.0.1:7777 \
  >artifacts/local-server.log 2>&1 &
server_pid="$!"

for _ in {1..100}; do
  if curl --fail --silent http://127.0.0.1:7777/healthz >/dev/null; then
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    sed -n '1,200p' artifacts/local-server.log
    exit 1
  fi
  sleep 0.05
done

echo "The Verse server is live at http://127.0.0.1:7777"
echo "Launching authoritative pilot ${player_id}"
"${godot_binary}" \
  --path apps/native-client \
  -- \
  --server=ws://127.0.0.1:7777/ws \
  "--player-id=${player_id}"
