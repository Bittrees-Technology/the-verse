#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
  echo "Usage: $0 <staged-package-directory>" >&2
  exit 1
fi

staging_directory="$(cd "$1" && pwd)"
smoke_port="${VERSE_PACKAGE_SMOKE_PORT:-17881}"
smoke_directory="$(mktemp -d)"
server_pid=""

if [[ ! "${smoke_port}" =~ ^[0-9]+$ ]] || (( smoke_port < 1024 || smoke_port > 65535 )); then
  echo "VERSE_PACKAGE_SMOKE_PORT must be an unprivileged TCP port." >&2
  exit 1
fi

cleanup() {
  if [[ -n "${server_pid}" ]] && kill -0 "${server_pid}" 2>/dev/null; then
    kill -TERM "${server_pid}" 2>/dev/null || true
    wait "${server_pid}" 2>/dev/null || true
  fi
  rm -rf "${smoke_directory}"
}
trap cleanup EXIT INT TERM

case "$(uname -s)" in
  Darwin)
    client_binary="${staging_directory}/The Verse.app/Contents/MacOS/The Verse"
    checksum_command=(shasum -a 256 -c)
    ;;
  Linux)
    client_binary="${staging_directory}/TheVerse.x86_64"
    checksum_command=(sha256sum -c)
    ;;
  *)
    echo "Unsupported package-smoke host: $(uname -s)" >&2
    exit 1
    ;;
esac

if [[ ! -x "${client_binary}" || ! -x "${staging_directory}/verse-simulation-worker" ]]; then
  echo "The staged package is missing a native client or authoritative server." >&2
  exit 1
fi

(
  cd "${staging_directory}"
  "${checksum_command[@]}" SHA256SUMS >/dev/null
)

"${staging_directory}/verse-simulation-worker" \
  --data-directory "${smoke_directory}/universe" \
  --bind "127.0.0.1:${smoke_port}" \
  >"${smoke_directory}/server.log" 2>&1 &
server_pid="$!"

for _ in {1..200}; do
  if curl --fail --silent "http://127.0.0.1:${smoke_port}/healthz" >/dev/null; then
    "${client_binary}" \
      --headless \
      -- \
      "--server=ws://127.0.0.1:${smoke_port}/ws" \
      --smoke-test
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
