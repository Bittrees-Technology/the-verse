#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "Usage: $0 <staged-package-directory>" >&2
  exit 1
fi

staging_directory="$(cd "$1" && pwd)"
smoke_port="${VERSE_PACKAGE_SMOKE_PORT:-17881}"
client_timeout_seconds="${VERSE_PACKAGE_CLIENT_TIMEOUT_SECONDS:-120}"
smoke_directory="$(mktemp -d)"
server_pid=""
client_pid=""

if [[ ! "${smoke_port}" =~ ^[0-9]+$ ]] || (( smoke_port < 1024 || smoke_port > 65535 )); then
  echo "VERSE_PACKAGE_SMOKE_PORT must be an unprivileged TCP port." >&2
  exit 1
fi
if [[ ! "${client_timeout_seconds}" =~ ^[0-9]+$ ]] || (( client_timeout_seconds < 1 )); then
  echo "VERSE_PACKAGE_CLIENT_TIMEOUT_SECONDS must be a positive integer." >&2
  exit 1
fi

cleanup() {
  if [[ -n "${client_pid}" ]] && kill -0 "${client_pid}" 2>/dev/null; then
    kill -TERM "${client_pid}" 2>/dev/null || true
    wait "${client_pid}" 2>/dev/null || true
  fi
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill -TERM "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  rm -rf "${smoke_directory}"
}
trap cleanup EXIT INT TERM

run_client_smoke() {
  "${client_command[@]}" \
    --headless \
    -- \
    "--server=ws://127.0.0.1:${smoke_port}/ws" \
    --smoke-test &
  client_pid="$!"
  local deadline=$((SECONDS + client_timeout_seconds))
  while kill -0 "${client_pid}" 2>/dev/null; do
    if (( SECONDS >= deadline )); then
      echo "The packaged native client smoke exceeded ${client_timeout_seconds}s." >&2
      kill -TERM "${client_pid}" 2>/dev/null || true
      wait "${client_pid}" 2>/dev/null || true
      client_pid=""
      return 124
    fi
    sleep 0.1
  done
  local status=0
  wait "${client_pid}" || status="$?"
  client_pid=""
  return "${status}"
}

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    client_binary="${staging_directory}/The Verse.app/Contents/MacOS/The Verse"
    client_command=(/usr/bin/arch -arm64 "${client_binary}")
    checksum_command=(shasum -a 256 -c)
    ;;
  Linux-x86_64)
    client_binary="${staging_directory}/TheVerse.x86_64"
    client_command=("${client_binary}")
    checksum_command=(sha256sum -c)
    ;;
  *)
    echo "Unsupported package-smoke host: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

if [[ ! -x "${client_binary}" || ! -x "${staging_directory}/verse-simulation-worker" ]]; then
  echo "The staged package is missing a native client or authoritative server." >&2
  exit 1
fi

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    verifier_library="${staging_directory}/The Verse.app/Contents/Frameworks/libverse_godot_interest_adapter.dylib"
    ;;
  Linux-x86_64)
    verifier_library="${staging_directory}/libverse_godot_interest_adapter.so"
    ;;
esac
if [[ ! -f "${verifier_library}" ]]; then
  echo "The staged package is missing the mandatory native interest verifier." >&2
  exit 1
fi
if [[ "$(uname -s)-$(uname -m)" == "Darwin-arm64" ]]; then
  for arm64_binary in \
    "${client_binary}" \
    "${staging_directory}/verse-simulation-worker" \
    "${verifier_library}"; do
    architectures="$(lipo -archs "${arm64_binary}")"
    case " ${architectures} " in
      *" arm64 "*) ;;
      *)
        echo "The staged macOS package contains a binary without an arm64 slice: ${arm64_binary}" >&2
        exit 1
        ;;
    esac
  done
  client_architectures="$(lipo -archs "${client_binary}")"
  if [[ " ${client_architectures} " == *" x86_64 "* ]]; then
    echo "The staged macOS client may select x86_64, but the mandatory verifier is arm64-only." >&2
    exit 1
  fi
fi
if [[ ! -s "${staging_directory}/licenses/MPL-2.0.txt" ]]; then
  echo "The staged package is missing the godot-rust MPL-2.0 notice." >&2
  exit 1
fi
for browser_asset in \
  verse_interest_verifier.js \
  verse_interest_verifier_bg.wasm; do
  if [[ ! -s "${staging_directory}/browser-verifier/${browser_asset}" ]]; then
    echo "The staged package is missing browser verifier asset ${browser_asset}." >&2
    exit 1
  fi
done

(
  cd "${staging_directory}"
  "${checksum_command[@]}" SHA256SUMS >/dev/null
)

VERSE_BROWSER_VERIFIER_ASSET_DIR="${staging_directory}/browser-verifier" \
  "${staging_directory}/verse-simulation-worker" \
  --data-directory "${smoke_directory}/universe" \
  --bind "127.0.0.1:${smoke_port}" \
  >"${smoke_directory}/server.log" 2>&1 &
server_pid="$!"

for _ in {1..200}; do
  if curl --fail --silent "http://127.0.0.1:${smoke_port}/healthz" >/dev/null; then
    curl --fail --silent \
      "http://127.0.0.1:${smoke_port}/generated/verse_interest_verifier.js" \
      >/dev/null
    curl --fail --silent \
      "http://127.0.0.1:${smoke_port}/generated/verse_interest_verifier_bg.wasm" \
      >/dev/null
    if run_client_smoke; then
      :
    else
      client_status="$?"
      sed -n '1,240p' "${smoke_directory}/server.log" >&2
      echo "The packaged native client smoke failed with status ${client_status}." >&2
      exit "${client_status}"
    fi
    echo "VERSE_NATIVE_PACKAGE_SMOKE_OK $(basename "${staging_directory}")"
    exit 0
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    sed -n '1,200p' "${smoke_directory}/server.log" >&2
    exit 1
  fi
  sleep 0.05
done

echo "The packaged authoritative server did not become ready." >&2
exit 1
