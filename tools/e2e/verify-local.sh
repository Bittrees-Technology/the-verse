#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
verse_test_dir="$(mktemp -d "${TMPDIR:-/tmp}/the-verse-e2e.XXXXXX")"
verse_server_pid=""
verse_port="${VERSE_E2E_PORT:-17777}"

cleanup() {
  if [[ -n "${verse_server_pid}" ]] && kill -0 "${verse_server_pid}" 2>/dev/null; then
    kill -TERM "${verse_server_pid}" 2>/dev/null || true
    wait "${verse_server_pid}" 2>/dev/null || true
  fi
  rm -rf "${verse_test_dir}"
}
trap cleanup EXIT

cd "${verse_root}"
cargo build --locked -p verse-simulation-worker

start_server() {
  target/debug/verse-simulation-worker \
    --data-directory "${verse_test_dir}" \
    --bind "127.0.0.1:${verse_port}" \
    --snapshot-every 5 \
    >"${verse_test_dir}/server.log" 2>&1 &
  verse_server_pid="$!"
  for _ in {1..100}; do
    if curl --fail --silent "http://127.0.0.1:${verse_port}/healthz" >/dev/null; then
      return
    fi
    sleep 0.05
  done
  sed -n '1,200p' "${verse_test_dir}/server.log"
  return 1
}

stop_server() {
  kill -TERM "${verse_server_pid}"
  wait "${verse_server_pid}"
  verse_server_pid=""
}

start_server
node tools/e2e/protocol-smoke.mjs "ws://127.0.0.1:${verse_port}/ws"

before_restart="$(
  curl --fail --silent "http://127.0.0.1:${verse_port}/api/v1/status"
)"
before_hash="$(jq -r '.world_hash' <<<"${before_restart}")"
before_sequence="$(jq -r '.event_sequence' <<<"${before_restart}")"
before_fence="$(jq -r '.fencing_token' <<<"${before_restart}")"
stop_server

start_server
after_restart="$(
  curl --fail --silent "http://127.0.0.1:${verse_port}/api/v1/status"
)"
[[ "$(jq -r '.world_hash' <<<"${after_restart}")" == "${before_hash}" ]]
[[ "$(jq -r '.event_sequence' <<<"${after_restart}")" == "${before_sequence}" ]]
[[ "$(jq -r '.fencing_token' <<<"${after_restart}")" -gt "${before_fence}" ]]

godot_bin="${GODOT_BIN:-}"
if [[ -z "${godot_bin}" ]] && [[ -x "artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot" ]]; then
  godot_bin="artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
fi
if [[ -n "${godot_bin}" ]]; then
  "${godot_bin}" \
    --headless \
    --path apps/native-client \
    -- \
    "--server=ws://127.0.0.1:${verse_port}/ws" \
    --smoke-test
else
  echo "Godot smoke skipped: set GODOT_BIN to a Godot 4.7.2 executable"
fi

echo "VERSE_LOCAL_VERIFICATION_OK sequence=${before_sequence} hash=${before_hash}"
