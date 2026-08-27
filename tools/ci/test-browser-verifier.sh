#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repository_root"

generated_check_directory="$(mktemp -d "${TMPDIR:-/tmp}/the-verse-browser-verifier.XXXXXX")"
cleanup() {
  rm -rf "${generated_check_directory}"
}
trap cleanup EXIT INT TERM
VERSE_BROWSER_VERIFIER_OUTPUT_DIR="${generated_check_directory}" \
  tools/ci/build-browser-verifier.sh
for generated_asset in verse_interest_verifier.js; do
  if ! cmp -s \
    "${generated_check_directory}/${generated_asset}" \
    "apps/web-command-center/generated/${generated_asset}"; then
    echo "Committed browser verifier drifted: ${generated_asset}" >&2
    echo "Regenerate it with tools/ci/build-browser-verifier.sh and commit the result." >&2
    exit 1
  fi
done
rust_host="$(rustc -vV | awk '/^host:/ { print $2 }')"
if [[ "${rust_host}" == "x86_64-unknown-linux-gnu" ]]; then
  if ! cmp -s \
    "${generated_check_directory}/verse_interest_verifier_bg.wasm" \
    "apps/web-command-center/generated/verse_interest_verifier_bg.wasm"; then
    echo "Committed browser verifier drifted: verse_interest_verifier_bg.wasm" >&2
    echo "Regenerate it on the canonical x86_64 Linux host with tools/ci/build-browser-verifier.sh and commit the result." >&2
    exit 1
  fi
else
  echo "VERSE_BROWSER_VERIFIER_NONCANONICAL_HOST host=${rust_host} wasm_byte_comparison=skipped"
fi
cargo test --locked -p verse-interest-verifier --lib wasm_browser::tests
node --check apps/web-command-center/app.js
node --check apps/web-command-center/verifier-worker.js
node --check apps/web-command-center/verifier-worker-core.js
node --test \
  apps/web-command-center/app.test.mjs \
  apps/web-command-center/verifier-worker-core.test.mjs \
  apps/web-command-center/verifier-wasm-smoke.test.mjs
cargo test --locked -p verse-simulation-worker \
  command_center_assets_are_served_by_the_game_server
cargo test --locked -p verse-simulation-worker \
  generated_browser_verifier_routes_are_typed_and_never_cached

echo "VERSE_BROWSER_VERIFIER_TESTS_OK"
