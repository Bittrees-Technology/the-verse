#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

package_directory="$(cd "$(dirname "$0")" && pwd)"
universe_directory="${VERSE_DATA_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/the-verse/universe}"
snapshot_every="${VERSE_SNAPSHOT_EVERY:-600}"
export VERSE_BROWSER_VERIFIER_ASSET_DIR="${package_directory}/browser-verifier"
server_pid=""

cleanup() {
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill -TERM "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

if curl --fail --silent --max-time 2 http://127.0.0.1:7777/healthz >/dev/null; then
  echo "A Verse session is already running. Close it before opening another workshop." >&2
  exit 1
fi

mkdir -p "${universe_directory}"
"${package_directory}/verse-simulation-worker" \
  --data-directory "${universe_directory}" \
  --snapshot-every "${snapshot_every}" \
  --bind 127.0.0.1:7777 \
  >"${universe_directory}/server.log" 2>&1 &
server_pid="$!"

for _ in {1..200}; do
  if curl --fail --silent http://127.0.0.1:7777/healthz >/dev/null; then
    "${package_directory}/TheVerse.x86_64" \
      -- --server=ws://127.0.0.1:7777/ws
    exit 0
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    sed -n '1,200p' "${universe_directory}/server.log" >&2
    exit 1
  fi
  sleep 0.05
done

echo "The Verse server did not become ready." >&2
exit 1
