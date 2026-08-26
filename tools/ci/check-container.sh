#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image_name="the-verse:p0-ci"
container_name="verse-p0-ci-${GITHUB_RUN_ID:-$$}"
host_port="${VERSE_CONTAINER_TEST_PORT:-17999}"

cleanup() {
  docker stop "${container_name}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

cd "${verse_root}"
docker build \
  --file infra/containers/simulation-worker.Dockerfile \
  --tag "${image_name}" \
  .
docker run \
  --rm \
  --detach \
  --name "${container_name}" \
  --publish "127.0.0.1:${host_port}:7777" \
  "${image_name}" >/dev/null

for _ in {1..200}; do
  if curl --fail --silent "http://127.0.0.1:${host_port}/healthz" >/dev/null; then
    status="$(curl --fail --silent "http://127.0.0.1:${host_port}/api/v1/status")"
    [[ "$(jq -r '.content_manifest_version' <<<"${status}")" == "p0.1.0" ]]
    [[ "$(jq -r '.conservation_valid' <<<"${status}")" == "true" ]]
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
