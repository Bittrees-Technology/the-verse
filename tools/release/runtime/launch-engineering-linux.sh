#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail
package_directory="$(cd "$(dirname "$0")" && pwd)"
export VERSE_DATA_DIR="${VERSE_DATA_DIR:-${XDG_DATA_HOME:-${HOME}/.local/share}/the-verse-engineering/universe}"
exec "${package_directory}/the-verse"
