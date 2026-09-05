#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail
verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${verse_root}"
verse_probe_directory="$(mktemp -d)"
verse_probe_port="${VERSE_PACING_PORT:-17789}"
godot_binary="${GODOT_BIN:-${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot}"
target/release/verse-simulation-worker --data-directory "${verse_probe_directory}" \
  --genesis-profile capital-start --snapshot-every 600 --bind "127.0.0.1:${verse_probe_port}" \
  > "${verse_probe_directory}/server.log" 2>&1 &
verse_probe_pid=$!
cleanup() {
  kill -TERM "${verse_probe_pid}" 2>/dev/null || true
  wait "${verse_probe_pid}" || true
  rm -rf "${verse_probe_directory}"
}
trap cleanup EXIT
for _ in {1..100}; do
  if ! kill -0 "${verse_probe_pid}" 2>/dev/null; then
    cat "${verse_probe_directory}/server.log"
    exit 1
  fi
  if curl -fs "http://127.0.0.1:${verse_probe_port}/healthz" >/dev/null; then
    break
  fi
  sleep .1
done
"${godot_binary}" --path apps/native-client --script res://tests/gameplay_pacing_probe.gd -- \
  "--server=ws://127.0.0.1:${verse_probe_port}/ws" "$@"
