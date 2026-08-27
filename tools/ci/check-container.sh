#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image_name="the-verse:p0-ci"
container_name="verse-p0-ci-${GITHUB_RUN_ID:-$$}"
host_port="${VERSE_CONTAINER_TEST_PORT:-17999}"
expected_manifest="$(jq -r '.manifest_version' "${verse_root}/content/definitions/p0-content.json")"

cleanup() {
  docker rm --force "${container_name}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

container_request_ok() {
  local request_path="$1"
  docker exec "${container_name}" bash -c '
    exec 3<>"/dev/tcp/127.0.0.1/${1}" || exit 1
    printf "GET %s HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n" "${2}" >&3
    IFS= read -r status_line <&3
    [[ "${status_line}" =~ ^HTTP/[0-9.]+[[:space:]]2[0-9][0-9][[:space:]] ]]
  ' _ "${host_port}" "${request_path}"
}

container_request_body() {
  local request_path="$1"
  docker exec "${container_name}" bash -c '
    exec 3<>"/dev/tcp/127.0.0.1/${1}" || exit 1
    printf "GET %s HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n" "${2}" >&3
    carriage_return="$(printf "\r")"
    while IFS= read -r header <&3; do
      [[ "${header}" == "${carriage_return}" ]] && break
    done
    cat <&3
  ' _ "${host_port}" "${request_path}"
}

cd "${verse_root}"
docker build \
  --file infra/containers/simulation-worker.Dockerfile \
  --tag "${image_name}" \
  .
docker run \
  --detach \
  --name "${container_name}" \
  --network host \
  "${image_name}" \
  --bind "127.0.0.1:${host_port}" \
  --data-directory /home/verse/data >/dev/null

for _ in {1..200}; do
  if container_request_ok "/healthz" 2>/dev/null; then
    status="$(container_request_body "/api/v1/status")"
    [[ "$(jq -r '.content_manifest_version' <<<"${status}")" == "${expected_manifest}" ]]
    [[ "$(jq -r '.conservation_valid' <<<"${status}")" == "true" ]]
    container_request_ok "/generated/verse_interest_verifier.js"
    container_request_ok "/generated/verse_interest_verifier_bg.wasm"
    echo "VERSE_CONTAINER_CHECK_OK"
    exit 0
  fi
  if ! docker ps --format '{{.Names}}' | grep -Fx "${container_name}" >/dev/null; then
    docker logs "${container_name}" 2>&1 || true
    exit 1
  fi
  sleep 0.05
done

docker logs "${container_name}" 2>&1 || true
echo "container did not become ready" >&2
exit 1
