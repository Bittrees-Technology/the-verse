#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${GODOT_BIN:-${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot}"
player_id="${VERSE_PLAYER_ID:-${1:-player-local}}"
server_url="${VERSE_SERVER_URL:-ws://127.0.0.1:7777/ws}"

cd "${verse_root}"
if [[ ! -x "${godot_binary}" ]]; then
  echo "The pinned Godot client is missing. Run tools/dev/bootstrap-macos.sh first." >&2
  exit 1
fi

echo "Connecting authoritative pilot ${player_id} to ${server_url}"
"${godot_binary}" \
  --path apps/native-client \
  -- \
  "--server=${server_url}" \
  "--player-id=${player_id}"
