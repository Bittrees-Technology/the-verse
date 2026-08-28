#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
verse_test_dir="$(mktemp -d "${TMPDIR:-/tmp}/the-verse-two-cell-e2e.XXXXXX")"
verse_server_pid=""
verse_port="${VERSE_TWO_CELL_E2E_PORT:-17778}"
verse_seed="${VERSE_TWO_CELL_E2E_SEED:-8031}"
verse_origin_cell_id=""

cleanup() {
  if [[ -n "${verse_server_pid}" ]] && kill -0 "${verse_server_pid}" 2>/dev/null; then
    kill -TERM "${verse_server_pid}" 2>/dev/null || true
    wait "${verse_server_pid}" 2>/dev/null || true
  fi
  rm -rf "${verse_test_dir}"
}
trap cleanup EXIT

cd "${verse_root}"
cargo build --locked \
  -p verse-simulation-worker \
  -p verse-simulation --example two_cell_boundary_fixture
target/debug/examples/two_cell_boundary_fixture \
  "${verse_test_dir}/universe" "${verse_seed}"

start_server() {
  local log_name="$1"
  target/debug/verse-simulation-worker \
    --two-cell-universe \
    --data-directory "${verse_test_dir}/universe" \
    --world-seed "${verse_seed}" \
    --bind "127.0.0.1:${verse_port}" \
    --snapshot-every 5 \
    --idle-drain-seconds 300 \
    >"${verse_test_dir}/${log_name}" 2>&1 &
  verse_server_pid="$!"
  for _ in {1..200}; do
    if curl --fail --silent "http://127.0.0.1:${verse_port}/healthz" >/dev/null; then
      return
    fi
    if ! kill -0 "${verse_server_pid}" 2>/dev/null; then
      break
    fi
    sleep 0.05
  done
  sed -n '1,260p' "${verse_test_dir}/${log_name}" >&2
  return 1
}

stop_server() {
  kill -TERM "${verse_server_pid}"
  wait "${verse_server_pid}"
  verse_server_pid=""
}

assert_durable_destination_placement() {
  local description="$1"
  local evidence_path="${verse_test_dir}/handoff-evidence.json"
  local directory_path="${verse_test_dir}/universe/cell-directory.json"
  local transfer_id

  transfer_id="$(jq -r '.transfer_id' "${evidence_path}")"
  jq --exit-status \
    --slurpfile evidence "${evidence_path}" \
    --arg transfer_id "${transfer_id}" \
    '
      .placements["player-local"] as $placement
      | .transfers[$transfer_id] as $transfer
      | ($placement != null)
        and ($placement.aggregate_id == "player-local")
        and ($placement.aggregate_kind == "player")
        and ($placement.cell_id == $evidence[0].destination_cell_id)
        and ($placement.cell_key == $evidence[0].destination_cell_key)
        and ($placement.placement_generation == $evidence[0].placement_generation)
        and ($placement.state == "resident")
        and ($placement.active_transfer_id == null)
        and ($transfer != null)
        and ($transfer.aggregate_id == "player-local")
        and ($transfer.destination_cell_id == $evidence[0].destination_cell_id)
        and ($transfer.resulting_placement_generation == $evidence[0].placement_generation)
        and ($transfer.phase == "finalized")
    ' "${directory_path}" >/dev/null
  echo "VERSE_TWO_CELL_DIRECTORY_${description} transfer=${transfer_id} placement=resident"
}

assert_origin_status() {
  local description="$1"
  local status
  local cell_id
  status="$(curl --fail --silent "http://127.0.0.1:${verse_port}/api/v1/status")"
  cell_id="$(jq -r '.cell_id' <<<"${status}")"
  if [[ -z "${verse_origin_cell_id}" ]]; then
    verse_origin_cell_id="${cell_id}"
  elif [[ "${cell_id}" != "${verse_origin_cell_id}" ]]; then
    echo "${description}: public status left the origin cell" >&2
    return 1
  fi
  if [[ "$(jq -r '.authoritative_halted' <<<"${status}")" != "false" ]]; then
    echo "${description}: the two-cell authority halted" >&2
    return 1
  fi
  echo "VERSE_TWO_CELL_STATUS_${description} cell=${cell_id} sequence=$(jq -r '.event_sequence' <<<"${status}") hash=$(jq -r '.world_hash' <<<"${status}")"
}

start_server "server-before-restart.log"
assert_origin_status "BEFORE_HANDOFF"
if ! node tools/e2e/two-cell-handoff-smoke.mjs \
  "ws://127.0.0.1:${verse_port}/ws" \
  --exercise "${verse_test_dir}/handoff-evidence.json"; then
  sed -n '1,300p' "${verse_test_dir}/server-before-restart.log" >&2
  exit 1
fi
assert_origin_status "AFTER_HANDOFF"
stop_server
assert_durable_destination_placement "AFTER_HANDOFF"

start_server "server-after-restart.log"
if ! node tools/e2e/two-cell-handoff-smoke.mjs \
  "ws://127.0.0.1:${verse_port}/ws" \
  --verify-recovery "${verse_test_dir}/handoff-evidence.json"; then
  sed -n '1,300p' "${verse_test_dir}/server-after-restart.log" >&2
  exit 1
fi
assert_origin_status "AFTER_RESTART"
stop_server
assert_durable_destination_placement "AFTER_RESTART"

echo "VERSE_TWO_CELL_E2E_OK"
