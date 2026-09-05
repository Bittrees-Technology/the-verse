#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${verse_root}"

cargo fmt --all -- --check
cargo test --workspace --locked --no-fail-fast
cargo clippy --workspace --all-targets --locked -- -D warnings
tools/ci/test-browser-verifier.sh
node --check tools/e2e/protocol-smoke.mjs
node --check tools/e2e/browser-verifier-smoke.mjs
node --check tools/e2e/browser-command-center-smoke.mjs
node --check tools/e2e/two-player-control-smoke.mjs
node --check tools/e2e/two-cell-handoff-smoke.mjs
node --check tools/e2e/p15-scale-evidence.mjs
npm ci --ignore-scripts --prefix tools/markdownlint
npm exec --prefix tools/markdownlint --no -- markdownlint-cli2 '**/*.md'

godot_binary="${GODOT_BIN:-}"
if [[ -z "${godot_binary}" ]] && [[ -x "artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot" ]]; then
  godot_binary="artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
fi
if [[ -n "${godot_binary}" ]]; then
  GODOT_BIN="${godot_binary}" tools/ci/verify-native-verifier.sh
  "${godot_binary}" --headless --path apps/native-client --script res://tests/voxel_surface_normals_smoke.gd
  "${godot_binary}" --headless --path apps/native-client --script res://tests/gameplay_structure_smoke.gd
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
tools/e2e/verify-p15-scale-evidence.sh
tools/e2e/verify-two-cell-handoff.sh

git diff --check
echo "VERSE_CHECKS_OK"
