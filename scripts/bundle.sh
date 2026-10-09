#!/usr/bin/env bash
# Local unsigned bundle; no distribution signing or notarisation.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p devdesk-app
bundle="target/DevDesk.app"
work=$(mktemp -d "${TMPDIR:-/tmp}/devdesk-icon.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$work/DevDesk.iconset"
cp target/release/devdesk "$bundle/Contents/MacOS/devdesk"
swiftc -module-cache-path target/perf-helpers/module-cache scripts/icon.swift -o "$work/icon"
"$work/icon" "$work/full.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/full.png" --out "$work/DevDesk.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$work/full.png" --out "$work/DevDesk.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/DevDesk.iconset" -o "$bundle/Contents/Resources/DevDesk.icns"
python3 - <<'PY'
import plistlib, tomllib
from pathlib import Path
version=tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
info={'CFBundleExecutable':'devdesk','CFBundleName':'DevDesk','CFBundleDisplayName':'DevDesk','CFBundleIdentifier':'com.jringeisen.DevDesk','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'CFBundleIconFile':'DevDesk','NSHighResolutionCapable':True,'NSRequiresAquaSystemAppearance':False}
Path('target/DevDesk.app/Contents/Info.plist').write_bytes(plistlib.dumps(info))
PY
plutil -lint "$bundle/Contents/Info.plist"
printf 'Built %s\n' "$bundle"
