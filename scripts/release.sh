#!/usr/bin/env bash
# Build a reviewable disk image. Public releases require Developer ID + notarization.
set -euo pipefail
cd "$(dirname "$0")/.."
mode="public"
output_dir="target/releases"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --local) mode="local"; shift ;;
        --output) output_dir="${2:?Missing output directory}"; shift 2 ;;
        --help) printf 'Usage: scripts/release.sh [--local] [--output DIRECTORY]\nPublic: set DEVDESK_SIGNING_IDENTITY and DEVDESK_NOTARY_PROFILE.\nLocal: ad hoc signed review image; not a public download.\n'; exit 0 ;;
        *) printf 'Unknown argument: %s\n' "$1" >&2; exit 2 ;;
    esac
done
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'This beta release requires an Apple Silicon Mac build host.' >&2; exit 1; }
if [[ "$mode" == public ]]; then
    : "${DEVDESK_SIGNING_IDENTITY:?Set a Developer ID Application signing identity}"
    : "${DEVDESK_NOTARY_PROFILE:?Set a notarytool Keychain profile}"
    [[ "$DEVDESK_SIGNING_IDENTITY" == 'Developer ID Application:'* ]] || { echo 'Use a Developer ID Application identity.' >&2; exit 1; }
fi
version=$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')
mkdir -p "$output_dir"
output_dir=$(cd "$output_dir" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/devdesk-release.XXXXXX")
trap 'rm -rf "$work"' EXIT
app="$work/staging/Tervy.app"
./scripts/check.sh
./scripts/bundle.sh "$app"
[[ "$(lipo -archs "$app/Contents/MacOS/devdesk")" == arm64 ]] || { echo 'Unexpected binary architecture.' >&2; exit 1; }
notarize() {
    local artifact="$1" log="$2"
    xcrun notarytool submit "$artifact" --keychain-profile "$DEVDESK_NOTARY_PROFILE" --wait --output-format json > "$log"
    python3 - "$log" <<'PY'
import json, sys
result = json.load(open(sys.argv[1]))
if result.get('status') != 'Accepted':
    raise SystemExit('Notarization did not accept this artifact. Review the saved notary log.')
PY
}
if [[ "$mode" == public ]]; then
    codesign --force --options runtime --timestamp --sign "$DEVDESK_SIGNING_IDENTITY" "$app/Contents/MacOS/meeting-audio"
    codesign --force --options runtime --timestamp --sign "$DEVDESK_SIGNING_IDENTITY" "$app"
    codesign --verify --deep --strict --verbose=2 "$app"
    ditto -c -k --keepParent "$app" "$work/Tervy.zip"
    notarize "$work/Tervy.zip" "$output_dir/Tervy-$version-app-notary.json"
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    spctl --assess --type execute --verbose=2 "$app"
fi
ln -s /Applications "$work/staging/Applications"
cp assets/brand/logo.svg "$work/staging/Tervy-logo.svg"
cat > "$work/staging/Start here.txt" <<'TXT'
Tervy
Keep your tools together. Keep your agents moving.

1. Drag Tervy to Applications, then open it from there.
2. Choose Add project and select a local project folder.
3. Use Terminals + for a shell, Processes + for project commands,
   and Agents + for Claude Code or Codex.

Coding agents are optional. Install and sign in to their CLIs separately,
then restart Tervy so it can pick up your shell environment.
Press Cmd-P and choose Setup and Prerequisites to check available tools.

Cmd-P searches commands. Cmd-T opens a terminal. Cmd-O adds a project.

This beta supports Apple Silicon Macs. Settings and sessions stay local;
installing on another computer does not sync them.
Stop running sessions and quit Tervy before replacing an installed build.
TXT
suffix=""
if [[ "$mode" == local ]]; then
    suffix="-local"
    printf '\nLOCAL REVIEW BUILD: ad hoc signed and not notarized. Public distribution requires a Developer ID signed release.\n' >> "$work/staging/Start here.txt"
fi
filename="Tervy-$version-macos-arm64$suffix.dmg"
# Work on a fresh image and replace only once packaging succeeds.
hdiutil create -volname Tervy -srcfolder "$work/staging" -format UDZO -ov "$work/$filename"
if [[ "$mode" == public ]]; then
    codesign --timestamp --sign "$DEVDESK_SIGNING_IDENTITY" "$work/$filename"
    notarize "$work/$filename" "$output_dir/Tervy-$version-dmg-notary.json"
    xcrun stapler staple "$work/$filename"
    xcrun stapler validate "$work/$filename"
fi
hdiutil verify "$work/$filename"
mv -f "$work/$filename" "$output_dir/$filename"
(cd "$output_dir" && shasum -a 256 "$filename" > "$filename.sha256")
python3 - "$output_dir/$filename" "$version" "$mode" <<'PY'
import datetime, hashlib, json, sys
from pathlib import Path
artifact = Path(sys.argv[1])
manifest = dict(product='Tervy', version=sys.argv[2], platform='macos', architecture='arm64',
    filename=artifact.name, sha256=hashlib.sha256(artifact.read_bytes()).hexdigest(),
    signed=sys.argv[3] == 'public', notarized=sys.argv[3] == 'public',
    built_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
artifact.with_suffix('.json').write_text(json.dumps(manifest, indent=2) + '\n')
PY
printf 'Built %s\n' "$output_dir/$filename"
