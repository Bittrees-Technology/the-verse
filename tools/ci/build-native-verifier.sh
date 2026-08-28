#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
profile="${1:-debug}"

case "${profile}" in
  debug)
    target_directory="debug"
    ;;
  release)
    target_directory="release"
    ;;
  *)
    echo "Usage: $0 [debug|release]" >&2
    exit 1
    ;;
esac

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    library_name="libverse_godot_interest_adapter.dylib"
    ;;
  Linux-x86_64)
    library_name="libverse_godot_interest_adapter.so"
    ;;
  *)
    echo "Unsupported native verifier host: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

cd "${verse_root}"
if [[ "${profile}" == "release" ]]; then
  cargo build --locked -p verse-godot-interest-adapter --release
else
  cargo build --locked -p verse-godot-interest-adapter
fi
mkdir -p apps/native-client/bin
cp "target/${target_directory}/${library_name}" "apps/native-client/bin/${library_name}"
echo "VERSE_NATIVE_VERIFIER_BUILT apps/native-client/bin/${library_name}"
