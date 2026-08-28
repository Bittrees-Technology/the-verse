#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
world_dir="${verse_root}/data/local-universe"
backup_root="${verse_root}/artifacts/world-backups"
timestamp="$(date -u +%Y%m%dT%H%M%SZ)"

if [[ ! -d "${world_dir}" ]]; then
  echo "No local universe exists; the next launch will create one."
  exit 0
fi
mkdir -p "${backup_root}"
destination="${backup_root}/local-universe-${timestamp}"
mv "${world_dir}" "${destination}"
echo "Local universe moved to ${destination}"
