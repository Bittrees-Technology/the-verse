#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${verse_root}"
cargo run -p verse-simulation-worker -- \
  --data-directory "${VERSE_DATA_DIR:-data/local-universe}" \
  --bind "${VERSE_BIND:-127.0.0.1:7777}"
