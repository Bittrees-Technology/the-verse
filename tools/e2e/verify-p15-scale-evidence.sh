#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
verse_scale_dir="$(mktemp -d "${TMPDIR:-/tmp}/the-verse-p15-scale.XXXXXX")"
verse_scale_port="${VERSE_P15_SCALE_PORT:-17779}"
verse_scale_pid=""

cleanup() {
  if [[ -n "${verse_scale_pid}" ]] && kill -0 "${verse_scale_pid}" 2>/dev/null; then
    kill -TERM "${verse_scale_pid}" 2>/dev/null || true
    wait "${verse_scale_pid}" 2>/dev/null || true
  fi
  rm -rf "${verse_scale_dir}"
}
trap cleanup EXIT

cd "${verse_root}"
cargo build --locked -p verse-simulation-worker
target/debug/verse-simulation-worker \
  --data-directory "${verse_scale_dir}" \
  --bind "127.0.0.1:${verse_scale_port}" \
  --snapshot-every 5 \
  --pause-simulation \
  >"${verse_scale_dir}/server.log" 2>&1 &
verse_scale_pid="$!"

for _ in {1..100}; do
  if curl --fail --silent \
    "http://127.0.0.1:${verse_scale_port}/healthz" >/dev/null; then
    node tools/e2e/p15-scale-evidence.mjs \
      "ws://127.0.0.1:${verse_scale_port}/ws"
    exit 0
  fi
  sleep 0.05
done

sed -n '1,200p' "${verse_scale_dir}/server.log" >&2
echo "P1.5 scale-evidence worker did not become healthy" >&2
exit 1
