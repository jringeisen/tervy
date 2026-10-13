#!/usr/bin/env bash
# Generate and sign the Sparkle appcast for the current version's GitHub Release.
# Publish the DMG first, then commit and push the generated root appcast.xml.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${TERVY_UPDATE_FEED_URL:?Set the HTTPS appcast URL embedded in the app}"
[[ "$TERVY_UPDATE_FEED_URL" == https://* ]] || { echo 'Update feed must use HTTPS' >&2; exit 1; }
directory="${1:-target/update-feed}"
[[ -d "$directory" ]] || { echo "No update directory: $directory" >&2; exit 1; }
if compgen -G "$directory/*-local.dmg" >/dev/null; then
    echo 'Remove local review images from the update directory' >&2
    exit 1
fi
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
if [[ ! -f "$directory/Tervy-$version-macos-arm64.dmg" ]]; then
    echo "Missing public Tervy-$version-macos-arm64.dmg in $directory" >&2
    exit 1
fi
sparkle_dir=$(scripts/fetch-sparkle.sh)
feed_name="${TERVY_UPDATE_FEED_URL##*/}"
[[ "$feed_name" == *.xml ]] || { echo 'Update feed URL must end in .xml' >&2; exit 1; }
download_prefix="${TERVY_UPDATE_DOWNLOAD_PREFIX:-https://github.com/jringeisen/tervy/releases/download/v$version/}"
[[ "$download_prefix" == https://*/ ]] || { echo 'Download prefix must be an HTTPS URL ending in /' >&2; exit 1; }
key_args=(--account "${TERVY_SPARKLE_KEY_ACCOUNT:-tervy}")
if [[ -n "${TERVY_SPARKLE_KEY_FILE:-}" ]]; then
    key_args=(--ed-key-file "$TERVY_SPARKLE_KEY_FILE")
else
    actual_key=$("$sparkle_dir/bin/generate_keys" --account "${TERVY_SPARKLE_KEY_ACCOUNT:-tervy}" -p)
    expected_key=$(<assets/update-public-key.txt)
    [[ "$actual_key" == "$expected_key" ]] || { echo 'The Keychain signing key does not match the bundled public key' >&2; exit 1; }
fi
work=$(mktemp -d "${TMPDIR:-/tmp}/tervy-appcast.XXXXXX")
trap 'rm -rf "$work"' EXIT
# Sparkle rewrites URLs for every archive it sees. Give it only this version's
# archive so older feed items keep their original versioned Release URLs.
cp "$directory/Tervy-$version-macos-arm64.dmg" "$work/"
for extension in md html txt; do
    notes="$directory/Tervy-$version-macos-arm64.$extension"
    if [[ -f "$notes" ]]; then cp "$notes" "$work/"; fi
done
if [[ -f "$feed_name" ]]; then cp "$feed_name" "$work/$feed_name"; fi
"$sparkle_dir/bin/generate_appcast" \
    "${key_args[@]}" \
    --download-url-prefix "$download_prefix" \
    --embed-release-notes \
    --versions "$version" \
    --maximum-versions 0 \
    --maximum-deltas 0 \
    -o "$work/$feed_name" "$work" 2>&1 | tee "$work/log"
if [[ "$(<"$work/log")" == *'SUPublicEDKey in the app'*'does not match key EdDSA'* ]]; then
    echo 'The app public key does not match the feed signing key' >&2
    exit 1
fi
"$sparkle_dir/bin/sign_update" "${key_args[@]}" --verify "$work/$feed_name"
python3 - "$work/$feed_name" "$download_prefix" "$version" <<'PY'
from xml.etree import ElementTree
import sys

feed, prefix, version = sys.argv[1:]
expected = f'{prefix}Tervy-{version}-macos-arm64.dmg'
urls = [node.get('url') for node in ElementTree.parse(feed).iter('enclosure')]
if expected not in urls:
    raise SystemExit(f'Generated appcast has no update for {expected}')
PY
cp "$work/$feed_name" "$directory/$feed_name"
cp "$work/$feed_name" "$feed_name"
printf 'Generated %s and %s\n' "$directory/$feed_name" "$feed_name"
