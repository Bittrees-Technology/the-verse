#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${GODOT_BIN:-${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot}"
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
if curl --fail --silent --max-time 2 http://127.0.0.1:7777/healthz >/dev/null; then
  echo "A Verse server is already running. Close that session or use tools/dev/run-client.sh." >&2
  exit 1
fi
if [[ ! -x "${godot_binary}" ]]; then
  tools/dev/bootstrap-macos.sh
fi
cargo build --locked --release -p verse-simulation-worker
tools/ci/build-native-verifier.sh release
mkdir -p artifacts
target/release/verse-simulation-worker \
  --data-directory "${VERSE_DATA_DIR:-data/local-universe}" \
  --genesis-profile "${VERSE_GENESIS_PROFILE:-orbital}" \
  --snapshot-every "${VERSE_SNAPSHOT_EVERY:-600}" \
  --bind 127.0.0.1:7777 \
  >artifacts/local-server.log 2>&1 &
server_pid="$!"

server_ready=0
for _ in {1..200}; do
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    sed -n '1,200p' artifacts/local-server.log
    exit 1
  fi
  if curl --fail --silent --max-time 2 http://127.0.0.1:7777/healthz >/dev/null; then
    server_ready=1
    break
  fi
  sleep 0.05
done
if [[ "${server_ready}" != "1" ]]; then
  echo "The Verse server did not become ready. See artifacts/local-server.log." >&2
  exit 1
fi

echo "The Verse server is live at http://127.0.0.1:7777"
echo "Launching authoritative pilot ${player_id}"
"${godot_binary}" \
  --path apps/native-client \
  -- \
  --server=ws://127.0.0.1:7777/ws \
  "--player-id=${player_id}"
