#!/usr/bin/env bash
# Fetch the pinned Sparkle distribution used by bundle.sh and update-feed.sh.
set -euo pipefail
cd "$(dirname "$0")/.."
version=2.9.6
checksum=8d5fb41d960b43f4a68aa14126bf62b098544ec8d191cdcc73eb14e63a8e7606
directory="target/sparkle/$version"
archive="$directory/Sparkle.zip"
framework="$directory/Sparkle.xcframework/macos-arm64_x86_64/Sparkle.framework"
mkdir -p "$directory"
if [[ ! -f "$archive" ]]; then
    curl -fL --retry 3 "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-for-Swift-Package-Manager.zip" -o "$archive.partial"
    mv "$archive.partial" "$archive"
fi
printf '%s  %s\n' "$checksum" "$archive" | shasum -a 256 -c - >&2
if [[ ! -d "$framework" ]]; then
    ditto -x -k "$archive" "$directory"
fi
[[ -f "$framework/Sparkle" && -x "$directory/bin/generate_appcast" ]] || {
    echo 'Sparkle distribution is incomplete' >&2
    exit 1
}
codesign --verify --deep --strict "$framework"
printf '%s\n' "$directory"
