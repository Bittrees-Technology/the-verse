#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
output_directory="${VERSE_BROWSER_VERIFIER_OUTPUT_DIR:-$repository_root/apps/web-command-center/generated}"
wasm_path="$repository_root/target/wasm32-unknown-unknown/release/verse_interest_verifier.wasm"

if ! rustup target list --installed | grep -qx 'wasm32-unknown-unknown'; then
  echo "missing Rust target wasm32-unknown-unknown; install it with: rustup target add wasm32-unknown-unknown" >&2
  exit 1
fi
if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "missing wasm-bindgen CLI 0.2.127; install it with: cargo install wasm-bindgen-cli --version 0.2.127 --locked" >&2
  exit 1
fi
if [[ "$(wasm-bindgen --version)" != "wasm-bindgen 0.2.127" ]]; then
  echo "wasm-bindgen CLI must be exactly 0.2.127" >&2
  exit 1
fi

cargo build \
  --locked \
  --release \
  --target wasm32-unknown-unknown \
  --features browser-wasm \
  -p verse-interest-verifier \
  --manifest-path "$repository_root/Cargo.toml"
mkdir -p "$output_directory"
wasm-bindgen \
  --target web \
  --no-typescript \
  --out-dir "$output_directory" \
  --out-name verse_interest_verifier \
  "$wasm_path"

test -s "$output_directory/verse_interest_verifier.js"
test -s "$output_directory/verse_interest_verifier_bg.wasm"
echo "VERSE_BROWSER_VERIFIER_BUILT"
