#!/usr/bin/env bash
# Local ad hoc signed bundle. scripts/release.sh handles distribution signing.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked -p devdesk-app
python3 scripts/brand.py
bundle="${1:-target/Tervy.app}"
work=$(mktemp -d "${TMPDIR:-/tmp}/devdesk-icon.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p target/perf-helpers "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$work/Tervy.iconset"
# Publish a fresh inode: overwriting a previously launched Mach-O in place can
# leave macOS's cached code signature inconsistent with the new executable.
executable=$(mktemp "$bundle/Contents/MacOS/.devdesk.XXXXXX")
install -m 755 target/release/devdesk "$executable"
mv -f "$executable" "$bundle/Contents/MacOS/devdesk"
install -m 755 target/meeting-audio "$bundle/Contents/MacOS/meeting-audio"
swiftc -module-cache-path "$work/module-cache" scripts/icon.swift -o "$work/icon"
"$work/icon" assets/brand/identity.json "$work/full.png"
install -m 644 "$work/full.png" "$bundle/Contents/Resources/Tervy.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/full.png" --out "$work/Tervy.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$work/full.png" --out "$work/Tervy.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/Tervy.iconset" -o "$bundle/Contents/Resources/Tervy.icns"
python3 - "$bundle" <<'PY'
import plistlib, tomllib, sys
from pathlib import Path
version=tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
info={'CFBundleExecutable':'devdesk','CFBundleName':'Tervy','CFBundleDisplayName':'Tervy','CFBundleIdentifier':'com.jringeisen.DevDesk','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'CFBundleIconFile':'Tervy','LSMinimumSystemVersion':'15.0','NSHighResolutionCapable':True,'NSRequiresAquaSystemAppearance':False,'NSMicrophoneUsageDescription':'Tervy records your microphone when you start a meeting recording.','NSAudioCaptureUsageDescription':'Tervy records meeting audio playing on your Mac when you start a recording.','NSScreenCaptureUsageDescription':'Tervy captures Mac audio when you start an experimental meeting recording.'}
(Path(sys.argv[1]) / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
PY
plutil -lint "$bundle/Contents/Info.plist"
codesign --force --sign - "$bundle/Contents/MacOS/meeting-audio"
codesign --force --sign - "$bundle"
codesign --verify --deep --strict "$bundle"
printf 'Built %s\n' "$bundle"
