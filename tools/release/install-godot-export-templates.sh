#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_version="4.7.2"
template_version="${godot_version}.stable"
template_archive="Godot_v${godot_version}-stable_export_templates.tpz"
template_sha256="f298490b8d44d934be425a5a65a51bf15f422428b229a06a6e11d9ffea248011"
template_url="https://github.com/godotengine/godot-builds/releases/download/${godot_version}-stable/${template_archive}"
archive_path="${verse_root}/artifacts/toolchains/${template_archive}"

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

case "$(uname -s)" in
  Darwin)
    template_root="${HOME}/Library/Application Support/Godot/export_templates"
    ;;
  Linux)
    template_root="${XDG_DATA_HOME:-${HOME}/.local/share}/godot/export_templates"
    ;;
  *)
    echo "Unsupported export host: $(uname -s)" >&2
    exit 1
    ;;
esac

template_directory="${template_root}/${template_version}"
if [[ -f "${template_directory}/version.txt" ]]; then
  installed_version="$(tr -d '\r\n' < "${template_directory}/version.txt")"
  if [[ "${installed_version}" == "${template_version}" ]]; then
    echo "Godot ${template_version} export templates are already installed."
    exit 0
  fi
fi

mkdir -p "$(dirname "${archive_path}")" "${template_root}"
if [[ -f "${archive_path}" ]]; then
  actual_sha256="$(sha256_file "${archive_path}")"
  if [[ "${actual_sha256}" != "${template_sha256}" ]]; then
    echo "Removing export-template archive with an invalid checksum." >&2
    rm -f "${archive_path}"
  fi
fi

if [[ ! -f "${archive_path}" ]]; then
  echo "Downloading the pinned Godot export templates (about 1.3 GB)..."
  curl -L --fail --show-error --progress-bar -o "${archive_path}.partial" "${template_url}"
  actual_sha256="$(sha256_file "${archive_path}.partial")"
  if [[ "${actual_sha256}" != "${template_sha256}" ]]; then
    rm -f "${archive_path}.partial"
    echo "Godot export-template checksum mismatch." >&2
    exit 1
  fi
  mv "${archive_path}.partial" "${archive_path}"
fi

temporary_directory="$(mktemp -d)"
cleanup() {
  rm -rf "${temporary_directory}"
}
trap cleanup EXIT

unzip -q "${archive_path}" -d "${temporary_directory}"
source_directory="${temporary_directory}"
if [[ -d "${temporary_directory}/templates" ]]; then
  source_directory="${temporary_directory}/templates"
fi

rm -rf "${template_directory}"
mkdir -p "${template_directory}"
cp -R "${source_directory}/." "${template_directory}/"

if [[ ! -f "${template_directory}/version.txt" ]]; then
  echo "The installed export-template archive is missing version.txt." >&2
  exit 1
fi

installed_version="$(tr -d '\r\n' < "${template_directory}/version.txt")"
if [[ "${installed_version}" != "${template_version}" ]]; then
  echo "Installed template version '${installed_version}' does not match '${template_version}'." >&2
  exit 1
fi

echo "Installed Godot ${template_version} export templates."
