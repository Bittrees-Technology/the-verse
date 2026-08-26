#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_version="4.7.2"
godot_archive="Godot_v${godot_version}-stable_macos.universal.zip"
godot_sha256="c58a24e31d720be9d62f60cb5627c4e695fb72f21b0cfe1bc9ccaa9a3b3ba63e"
godot_download="https://github.com/godotengine/godot-builds/releases/download/${godot_version}-stable/${godot_archive}"
godot_tool_dir="${verse_root}/artifacts/toolchains/godot-${godot_version}"
godot_binary="${godot_tool_dir}/Godot.app/Contents/MacOS/Godot"
godot_zip="${verse_root}/artifacts/toolchains/${godot_archive}"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This bootstrap is for macOS. Use a Linux Godot 4.7.2 binary and cargo directly."
  exit 1
fi
command -v cargo >/dev/null
command -v curl >/dev/null
command -v shasum >/dev/null

mkdir -p "${verse_root}/artifacts/toolchains" "${godot_tool_dir}"
if [[ ! -x "${godot_binary}" ]]; then
  if [[ ! -f "${godot_zip}" ]]; then
    curl -L --fail --show-error --progress-bar -o "${godot_zip}" "${godot_download}"
  fi
  echo "${godot_sha256}  ${godot_zip}" | shasum -a 256 -c -
  ditto -x -k "${godot_zip}" "${godot_tool_dir}"
fi

"${godot_binary}" --version
cd "${verse_root}"
cargo build -p verse-simulation-worker
echo "VERSE_BOOTSTRAP_OK"
