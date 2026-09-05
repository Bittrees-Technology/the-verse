#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail
package_directory="$(cd "$(dirname "$0")" && pwd)"
export VERSE_DATA_DIR="${VERSE_DATA_DIR:-${HOME}/Library/Application Support/The Verse Engineering Playtest/universe}"
export VERSE_GENESIS_PROFILE=orbital
exec "${package_directory}/Launch The Verse.command"
