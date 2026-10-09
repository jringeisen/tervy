#!/usr/bin/env bash
# Local unsigned bundle; no distribution signing or notarisation.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release -p devdesk-app
bundle="${1:-target/DevDesk.app}"
work=$(mktemp -d "${TMPDIR:-/tmp}/devdesk-icon.XXXXXX")
trap 'rm -rf "$work"' EXIT
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$work/DevDesk.iconset"
# Publish a fresh inode: overwriting a previously launched Mach-O in place can
# leave macOS's cached code signature inconsistent with the new executable.
executable=$(mktemp "$bundle/Contents/MacOS/.devdesk.XXXXXX")
install -m 755 target/release/devdesk "$executable"
mv -f "$executable" "$bundle/Contents/MacOS/devdesk"
swiftc -module-cache-path target/perf-helpers/module-cache scripts/icon.swift -o "$work/icon"
"$work/icon" "$work/full.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/full.png" --out "$work/DevDesk.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$work/full.png" --out "$work/DevDesk.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/DevDesk.iconset" -o "$bundle/Contents/Resources/DevDesk.icns"
python3 - "$bundle" <<'PY'
import plistlib, tomllib, sys
from pathlib import Path
version=tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
info={'CFBundleExecutable':'devdesk','CFBundleName':'DevDesk','CFBundleDisplayName':'DevDesk','CFBundleIdentifier':'com.jringeisen.DevDesk','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'CFBundleIconFile':'DevDesk','NSHighResolutionCapable':True,'NSRequiresAquaSystemAppearance':False}
(Path(sys.argv[1]) / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
PY
plutil -lint "$bundle/Contents/Info.plist"
printf 'Built %s\n' "$bundle"
