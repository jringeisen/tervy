#!/usr/bin/env bash
# Local ad hoc signed bundle. scripts/release.sh handles distribution signing.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked -p tervy-app
python3 scripts/brand.py
bundle="${1:-target/Tervy.app}"
work=$(mktemp -d "${TMPDIR:-/tmp}/tervy-icon.XXXXXX")
trap 'rm -rf "$work"' EXIT
helper_app="$bundle/Contents/Helpers/Tervy Meeting Audio.app"
mkdir -p target/perf-helpers "$bundle/Contents/MacOS" "$bundle/Contents/Resources" "$bundle/Contents/Frameworks" "$helper_app/Contents/MacOS" "$work/Tervy.iconset"
# Publish a fresh inode: overwriting a previously launched Mach-O in place can
# leave macOS's cached code signature inconsistent with the new executable.
executable=$(mktemp "$bundle/Contents/MacOS/.tervy.XXXXXX")
install -m 755 target/release/tervy "$executable"
mv -f "$executable" "$bundle/Contents/MacOS/tervy"
rm -f "$bundle/Contents/MacOS/meeting-audio"
install -m 755 target/meeting-audio "$helper_app/Contents/MacOS/meeting-audio"
sparkle_dir=$(scripts/fetch-sparkle.sh)
sparkle_framework="$sparkle_dir/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework"
ditto "$sparkle_framework" "$bundle/Contents/Frameworks/Sparkle.framework"
install -m 644 "$sparkle_dir/LICENSE" "$bundle/Contents/Resources/Sparkle-LICENSE.txt"
swiftc -target arm64-apple-macosx15.0 -module-cache-path "$work/module-cache" \
    -parse-as-library -emit-library -F "$bundle/Contents/Frameworks" -framework Sparkle \
    -Xlinker -rpath -Xlinker @loader_path \
    scripts/updater.swift -o "$bundle/Contents/Frameworks/TervyUpdater.dylib"
swiftc -module-cache-path "$work/module-cache" scripts/icon.swift -o "$work/icon"
"$work/icon" assets/brand/identity.json "$work/full.png"
install -m 644 "$work/full.png" "$bundle/Contents/Resources/Tervy.png"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$work/full.png" --out "$work/Tervy.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$work/full.png" --out "$work/Tervy.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$work/Tervy.iconset" -o "$bundle/Contents/Resources/Tervy.icns"
python3 - "$bundle" "$helper_app" <<'PY'
import base64, os, plistlib, tomllib, sys
from pathlib import Path
version=tomllib.loads(Path('Cargo.toml').read_text())['workspace']['package']['version']
info={'CFBundleExecutable':'tervy','CFBundleName':'Tervy','CFBundleDisplayName':'Tervy','CFBundleIdentifier':'com.jringeisen.Tervy','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'CFBundleIconFile':'Tervy','LSMinimumSystemVersion':'15.0','NSHighResolutionCapable':True,'NSRequiresAquaSystemAppearance':False,'NSMicrophoneUsageDescription':'Tervy records your microphone when you start a meeting recording.','NSAudioCaptureUsageDescription':'Tervy records meeting audio playing on your Mac when you start a recording.','NSScreenCaptureUsageDescription':'Tervy captures Mac audio when you start an experimental meeting recording.'}
feed=os.environ.get('TERVY_UPDATE_FEED_URL','')
key=os.environ.get('TERVY_SPARKLE_PUBLIC_KEY','')
if feed and not key:
    key=Path('assets/update-public-key.txt').read_text().strip()
if feed or key:
    if not feed.startswith('https://') or not key:
        raise SystemExit('Set both TERVY_UPDATE_FEED_URL (HTTPS) and TERVY_SPARKLE_PUBLIC_KEY')
    try:
        if len(base64.b64decode(key,validate=True)) != 32:
            raise ValueError('Ed25519 public key must be 32 bytes')
    except ValueError as error:
        raise SystemExit(f'Invalid TERVY_SPARKLE_PUBLIC_KEY: {error}')
    info.update(SUFeedURL=feed,SUPublicEDKey=key,SUEnableAutomaticChecks=True,
                SUAutomaticallyUpdate=False,SUVerifyUpdateBeforeExtraction=True,
                SURequireSignedFeed=True)
(Path(sys.argv[1]) / 'Contents/Info.plist').write_bytes(plistlib.dumps(info))
helper_info={'CFBundleExecutable':'meeting-audio','CFBundleName':'Tervy Meeting Audio','CFBundleDisplayName':'Tervy Meeting Audio','CFBundleIdentifier':'com.jringeisen.Tervy.MeetingAudio','CFBundlePackageType':'APPL','CFBundleShortVersionString':version,'CFBundleVersion':version,'LSMinimumSystemVersion':'26.0','LSUIElement':True,'NSMicrophoneUsageDescription':'Tervy Meeting Audio records your microphone when you start a meeting recording.','NSAudioCaptureUsageDescription':'Tervy Meeting Audio records meeting audio playing on your Mac when you start a recording.','NSScreenCaptureUsageDescription':'Tervy Meeting Audio captures Mac audio when you start a meeting recording.'}
(Path(sys.argv[2]) / 'Contents/Info.plist').write_bytes(plistlib.dumps(helper_info))
PY
plutil -lint "$bundle/Contents/Info.plist"
plutil -lint "$helper_app/Contents/Info.plist"
sparkle_bundle="$bundle/Contents/Frameworks/Sparkle.framework"
codesign --force --sign - "$sparkle_bundle/Versions/B/XPCServices/Installer.xpc"
codesign --force --sign - --preserve-metadata=entitlements "$sparkle_bundle/Versions/B/XPCServices/Downloader.xpc"
codesign --force --sign - "$sparkle_bundle/Versions/B/Autoupdate"
codesign --force --sign - "$sparkle_bundle/Versions/B/Updater.app"
codesign --force --sign - "$sparkle_bundle"
codesign --force --sign - "$bundle/Contents/Frameworks/TervyUpdater.dylib"
codesign --force --sign - --entitlements scripts/meeting-audio.entitlements.plist "$helper_app"
codesign --force --sign - --entitlements scripts/meeting-audio.entitlements.plist "$bundle"
codesign --verify --deep --strict "$bundle"
printf 'Built %s\n' "$bundle"
