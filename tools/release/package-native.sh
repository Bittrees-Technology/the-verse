#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
set -euo pipefail

verse_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
godot_binary="${GODOT_BIN:-}"
release_version="${VERSE_RELEASE_VERSION:-p1.7.0-dev}"
release_root="${verse_root}/artifacts/release"
staging_root="${release_root}/staging"

if [[ ! "${release_version}" =~ ^[0-9A-Za-z._-]+$ ]]; then
  echo "VERSE_RELEASE_VERSION may contain only letters, digits, periods, underscores, and hyphens." >&2
  exit 1
fi

sha256_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)
    release_platform="macos-arm64"
    export_preset="macOS Universal"
    default_godot="${verse_root}/artifacts/toolchains/godot-4.7.2/Godot.app/Contents/MacOS/Godot"
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
    ;;
  Linux-x86_64)
    release_platform="linux-x86_64"
    export_preset="Linux x86_64"
    default_godot=""
    ;;
  *)
    echo "Unsupported packaging host: $(uname -s) $(uname -m)" >&2
    exit 1
    ;;
esac

if [[ -z "${godot_binary}" ]]; then
  godot_binary="${default_godot}"
fi
if [[ -z "${godot_binary}" || ! -x "${godot_binary}" ]]; then
  echo "Set GODOT_BIN to the pinned Godot 4.7.2 executable." >&2
  exit 1
fi
if [[ "$("${godot_binary}" --headless --version)" != 4.7.2.stable.* ]]; then
  echo "Native packages require Godot 4.7.2 stable." >&2
  exit 1
fi

package_name="the-verse-${release_version}-${release_platform}"
staging_directory="${staging_root}/${package_name}"
rm -rf "${staging_directory}"
mkdir -p "${staging_directory}/browser-verifier" "${staging_directory}/licenses"

cd "${verse_root}"
commit_sha="$(git rev-parse HEAD)"
if [[ -n "$(git status --porcelain --untracked-files=normal)" ]]; then
  if [[ "${VERSE_ALLOW_DIRTY_PACKAGE:-0}" != "1" ]]; then
    echo "Refusing to package an uncommitted worktree. Commit the release inputs first." >&2
    exit 1
  fi
  source_revision="${commit_sha}-dirty"
else
  source_revision="${commit_sha}"
fi
cargo build --locked --release -p verse-simulation-worker
tools/ci/build-native-verifier.sh release
for browser_asset in \
  verse_interest_verifier.js \
  verse_interest_verifier_bg.wasm; do
  if [[ ! -s "apps/web-command-center/generated/${browser_asset}" ]]; then
    echo "Missing committed browser verifier asset: ${browser_asset}" >&2
    exit 1
  fi
  cp \
    "apps/web-command-center/generated/${browser_asset}" \
    "${staging_directory}/browser-verifier/${browser_asset}"
done

if [[ "${release_platform}" == "macos-arm64" ]]; then
  "${godot_binary}" \
    --headless \
    --path apps/native-client \
    --export-release "${export_preset}" "${staging_directory}/The Verse.app"
  cp target/release/verse-simulation-worker "${staging_directory}/verse-simulation-worker"
  cp tools/release/runtime/launch-macos.command "${staging_directory}/Launch The Verse.command"
  chmod 755 \
    "${staging_directory}/verse-simulation-worker" \
    "${staging_directory}/Launch The Verse.command"
else
  "${godot_binary}" \
    --headless \
    --path apps/native-client \
    --export-release "${export_preset}" "${staging_directory}/TheVerse.x86_64"
  cp target/release/verse-simulation-worker "${staging_directory}/verse-simulation-worker"
  cp tools/release/runtime/launch-linux.sh "${staging_directory}/the-verse"
  chmod 755 \
    "${staging_directory}/TheVerse.x86_64" \
    "${staging_directory}/verse-simulation-worker" \
    "${staging_directory}/the-verse"
fi

cp LICENSE "${staging_directory}/LICENSE"
cp -R LICENSES/. "${staging_directory}/licenses/"
cp tools/release/runtime/README.txt "${staging_directory}/README.txt"

printf '%s\n' \
  "The Verse ${release_version}" \
  "Source revision: ${source_revision}" \
  "Content manifest: p1.5.0" \
  "Content schema: 11" \
  "Protocol: 18" \
  "Projection schema: 4" \
  "World schema: 20" \
  "Event schema: 16" \
  "Celestial registry schema: 1" \
  "Universe manifest schema: 4" \
  "Interest schema: 2" \
  "Operation fingerprint schema: 2" \
  "Cell directory schema: 2" \
  "Transfer package schema: 1" \
  "Interest verifier encoding: 1" \
  "Browser verifier generator: wasm-bindgen 0.2.127" \
  "Package: ${release_platform}" \
  "Channel: development" \
  > "${staging_directory}/VERSION.txt"

(
  cd "${staging_directory}"
  while IFS= read -r packaged_file; do
    printf '%s  %s\n' "$(sha256_file "${packaged_file}")" "${packaged_file}"
  done < <(find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort) \
    > SHA256SUMS
)

tools/release/smoke-native-package.sh "${staging_directory}"

mkdir -p "${release_root}"
if [[ "${release_platform}" == "macos-arm64" ]]; then
  archive_path="${release_root}/${package_name}.zip"
  rm -f "${archive_path}"
  ditto -c -k --sequesterRsrc --keepParent "${staging_directory}" "${archive_path}"
else
  archive_path="${release_root}/${package_name}.tar.gz"
  rm -f "${archive_path}"
  tar -C "${staging_root}" -czf "${archive_path}" "${package_name}"
fi

printf '%s  %s\n' "$(sha256_file "${archive_path}")" "$(basename "${archive_path}")" \
  > "${archive_path}.sha256"
echo "VERSE_NATIVE_PACKAGE_OK ${archive_path}"
