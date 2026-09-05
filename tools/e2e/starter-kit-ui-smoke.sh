#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail
verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${GODOT_BIN:-${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot}"
verse_test_directory="$(mktemp -d "${TMPDIR:-/tmp}/verse-starter-kit.XXXXXX")"
verse_port="${VERSE_KIT_UI_PORT:-17779}"
verse_server_pid=""
cleanup() {
  if [[ -n "${verse_server_pid}" ]] && kill -0 "${verse_server_pid}" 2>/dev/null; then
    kill -TERM "${verse_server_pid}"
    wait "${verse_server_pid}" || true
  fi
  rm -rf "${verse_test_directory}"
}
trap cleanup EXIT INT TERM
cd "${verse_root}"
if [[ ! -x "${godot_binary}" ]]; then
  echo "Set GODOT_BIN to the pinned Godot executable." >&2
  exit 1
fi
cargo build --locked --release -p verse-simulation-worker
tools/ci/build-native-verifier.sh release
target/release/verse-simulation-worker \
  --data-directory "${verse_test_directory}/universe" \
  --genesis-profile "${VERSE_UI_PROFILE:-ore-workshop}" --snapshot-every 600 --bind "127.0.0.1:${verse_port}" \
  >"${verse_test_directory}/server.log" 2>&1 &
verse_server_pid="$!"
ready=0
for _ in {1..200}; do
  if ! kill -0 "${verse_server_pid}" 2>/dev/null; then
    cat "${verse_test_directory}/server.log" >&2
    exit 1
  fi
  if curl --fail --silent "http://127.0.0.1:${verse_port}/healthz" >/dev/null; then
    ready=1
    break
  fi
  sleep 0.05
done
if [[ "${ready}" != "1" ]]; then
  cat "${verse_test_directory}/server.log" >&2
  exit 1
fi
"${godot_binary}" --path apps/native-client \
  --script res://tests/starter_kit_ui_smoke.gd -- \
  "--server=ws://127.0.0.1:${verse_port}/ws" \
  "--output-directory=${verse_root}/artifacts/starter-kit-review" "${VERSE_UI_TEST_ARGUMENT:---ui-test}"
