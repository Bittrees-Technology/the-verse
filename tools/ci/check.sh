#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${verse_root}"

cargo fmt --all -- --check
cargo test --workspace --locked --no-fail-fast
cargo clippy --workspace --all-targets --locked -- -D warnings
node --check apps/web-command-center/app.js
node --test apps/web-command-center/app.test.mjs
node --check tools/e2e/protocol-smoke.mjs
node --check tools/e2e/two-player-control-smoke.mjs
npx --yes markdownlint-cli2 '**/*.md'

godot_binary="${GODOT_BIN:-}"
if [[ -z "${godot_binary}" ]] && [[ -x "artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot" ]]; then
  godot_binary="artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
fi
if [[ -n "${godot_binary}" ]]; then
  "${godot_binary}" --headless --editor --path apps/native-client --quit
  "${godot_binary}" \
    --headless \
    --path apps/native-client \
    --script res://tests/motion_impairment_smoke.gd
  "${godot_binary}" \
    --headless \
    --path apps/native-client \
    --script res://tests/p15_interest_stream_smoke.gd
  GODOT_BIN="${godot_binary}" tools/e2e/verify-local.sh
else
  tools/e2e/verify-local.sh
fi

git diff --check
echo "VERSE_CHECKS_OK"
