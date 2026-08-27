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
command -v xcrun >/dev/null

if ! command -v cmake >/dev/null; then
  echo "CMake 3.16 or newer is required to build the Jolt physics adapter (for example: brew install cmake)." >&2
  exit 1
fi
cmake_version="$(cmake --version | awk 'NR == 1 { print $3 }')"
cmake_major="${cmake_version%%.*}"
cmake_remainder="${cmake_version#*.}"
cmake_minor="${cmake_remainder%%.*}"
if ((cmake_major < 3 || (cmake_major == 3 && cmake_minor < 16))); then
  echo "CMake ${cmake_version} is too old; Jolt requires CMake 3.16 or newer." >&2
  exit 1
fi

xcrun --find clang >/dev/null
if [[ -z "${LIBCLANG_PATH:-}" ]]; then
  developer_dir="$(xcode-select -p)"
  libclang_candidate="${developer_dir}/usr/lib"
  if [[ ! -f "${libclang_candidate}/libclang.dylib" ]] && command -v brew >/dev/null; then
    brew_llvm="$(brew --prefix llvm 2>/dev/null || true)"
    if [[ -n "${brew_llvm}" ]]; then
      libclang_candidate="${brew_llvm}/lib"
    fi
  fi
  if [[ ! -f "${libclang_candidate}/libclang.dylib" ]]; then
    echo "libclang is required by the Jolt bindings; install Xcode Command Line Tools or Homebrew LLVM." >&2
    exit 1
  fi
  export LIBCLANG_PATH="${libclang_candidate}"
elif [[ ! -f "${LIBCLANG_PATH}/libclang.dylib" ]]; then
  echo "LIBCLANG_PATH does not contain libclang.dylib: ${LIBCLANG_PATH}" >&2
  exit 1
fi

# Apple Silicon Rust targets macOS 11 by default. Propagating the same minimum
# to CMake prevents the static Jolt objects from acquiring the build host's
# newer deployment target.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

mkdir -p "${verse_root}/artifacts/toolchains" "${godot_tool_dir}"
if [[ ! -x "${godot_binary}" ]]; then
  if [[ ! -f "${godot_zip}" ]]; then
    curl -L --fail --show-error --progress-bar -o "${godot_zip}" "${godot_download}"
  fi
  echo "${godot_sha256}  ${godot_zip}" | shasum -a 256 -c -
  ditto -x -k "${godot_zip}" "${godot_tool_dir}"
fi

"${godot_binary}" --version
cmake --version | head -1
cd "${verse_root}"
cargo build -p verse-simulation-worker
echo "VERSE_BOOTSTRAP_OK"
