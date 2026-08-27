#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${GODOT_BIN:-}"

if [[ -z "${godot_binary}" ]] && [[ -x "${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot" ]]; then
  godot_binary="${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
fi
if [[ -z "${godot_binary}" || ! -x "${godot_binary}" ]]; then
  echo "Set GODOT_BIN to Godot 4.7.2." >&2
  exit 1
fi

cd "${verse_root}"
tools/ci/build-native-verifier.sh debug
"${godot_binary}" --headless --editor --path apps/native-client --quit
"${godot_binary}" \
  --headless \
  --path apps/native-client \
  --script res://tests/native_interest_verifier_smoke.gd

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    verifier_library="apps/native-client/bin/libverse_godot_interest_adapter.dylib"
    ;;
  Linux-x86_64)
    verifier_library="apps/native-client/bin/libverse_godot_interest_adapter.so"
    ;;
  *)
    echo "Unsupported native verifier host." >&2
    exit 1
    ;;
esac

held_library="${verifier_library}.missing-test"
missing_startup_log="$(mktemp "${TMPDIR:-/tmp}/the-verse-native-missing.XXXXXX")"
restore_library() {
  if [[ -f "${held_library}" ]]; then
    mv "${held_library}" "${verifier_library}"
  fi
  rm -f "${missing_startup_log}"
}
trap restore_library EXIT INT TERM
mv "${verifier_library}" "${held_library}"
"${godot_binary}" \
  --headless \
  --path apps/native-client \
  --script res://tests/native_interest_verifier_missing_smoke.gd
if "${godot_binary}" \
  --headless \
  --path apps/native-client \
  -- \
  --smoke-test \
  >"${missing_startup_log}" 2>&1; then
  sed -n '1,200p' "${missing_startup_log}" >&2
  echo "The native client started without its mandatory verifier." >&2
  exit 1
fi
if ! rg -q \
  'VERSE_SMOKE_CLIENT_FATAL NATIVE INTEREST VERIFIER EXTENSION UNAVAILABLE' \
  "${missing_startup_log}"; then
  sed -n '1,200p' "${missing_startup_log}" >&2
  echo "The missing-verifier startup did not fail through the expected guard." >&2
  exit 1
fi
restore_library
trap - EXIT INT TERM
echo "VERSE_NATIVE_VERIFIER_TESTS_OK"
