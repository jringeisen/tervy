# Development and release notes

Run the commands below from the repository root. See [Contributing](../CONTRIBUTING.md) for prerequisites and pull request guidance.

## Build from source

Build with Rust via rustup and Apple's Command Line Tools. Install `cargo-deny`
0.20.2 for the quality gate. Use Xcode only if your toolchain cannot
compile the required Metal shaders. Dependencies are deliberately pinned.

```sh
./scripts/check.sh
cargo run
./scripts/bundle.sh
# Optional separate destination for reviewing a build:
# ./scripts/bundle.sh target/Tervy-review.app
```

The last command creates `target/Tervy.app` with an original icon and
`Info.plist`, without distribution signing or notarisation. Move the app to its
final location **before launching agents**: each CLI receives the absolute path
to that bundle's `Contents/MacOS/tervy` hook helper. Stop agents before moving
or replacing the bundle, then resume them from the new app.

## Performance and architecture

Reproduce the final measurements with `scripts/perf/run-m7.sh`, then compare
records using `scripts/perf/compare.py perf/baseline-m2.json perf/baseline.json`.
Leave benchmark windows visible and untouched. Process memory and CLI memory
are reported separately. A11's upstream GPUI display-link callbacks still prevent
a passing visible idle CPU budget; the review records that limit explicitly.

The architecture and 27 accepted ADRs are in [the architecture notes](architecture.md).
See the [final technical review](m7-review.md) and
[v1 product review](v1-product-review.md) for validation and remaining work.
Vendored Apache/MIT modifications are attributed in [vendor/README.md](../vendor/README.md).
No code was copied from Zed's GPL crates.

## Brand and Mac beta

The public brand is **Tervy**: “Keep your tools together. Keep your agents moving.”
Its modular **T** brings a tool tile and shared bar together with a forward arrow.
The palette uses cobalt, graphite, paper, and pale blue supporting graphics.
System sans-serif is used for interface copy, with Menlo for technical text.
Copy should be clear, capable, and calm.

The [brand sheet](../assets/brand/brand-sheet.svg) shows the logo, tagline, palette,
and applications. The [brand guide](../assets/brand/README.md) documents usage and
exports. The [vector kit](../assets/brand) includes light/dark marks, outlined
wordmarks, tagline lockups, and the app icon. `identity.json` is the source for
geometry and colors; run `python3 scripts/brand.py` to regenerate the SVG assets.
The native icon renderer reads the same source.

## Release builds

Build a local review disk image on an Apple Silicon Mac:

```sh
scripts/release.sh --local
```

The image in `target/releases` includes Tervy, an Applications shortcut, and
installation instructions. A SHA-256 checksum and machine-readable release
manifest accompany it. Local images are **ad hoc signed and not notarized**; they are
review artifacts, not public releases. A manual GitHub Actions workflow builds
these artifacts without publishing a release.

For a public Mac release, configure a Developer ID Application certificate and a
`notarytool` Keychain profile using Apple's
[distribution guidance](https://developer.apple.com/developer-id/), then run:

```sh
export TERVY_SIGNING_IDENTITY='Developer ID Application: Your Name (TEAMID)'
export TERVY_NOTARY_PROFILE='tervy-notary'
export TERVY_UPDATE_FEED_URL='https://raw.githubusercontent.com/jringeisen/tervy/main/appcast.xml'
scripts/release.sh
```

The script checks the workspace, builds the app, signs with the hardened runtime,
notarizes and staples the app, creates and signs a disk image, then notarizes,
staples, and verifies the image. It fails if notarization is not accepted. No
credentials belong in source control. [In-app updates](updates.md) use Sparkle and
require publishing a signed appcast and each versioned disk image. A source-code
push by itself does not release an update. Existing installations need one manual
upgrade to an updater-enabled release.

This beta targets **macOS on Apple Silicon**. Intel Mac, Windows, and Linux builds
are not yet validated. Cross-platform work remains in terminal/process ownership,
IPC, shell discovery, app integration, and packaging. Downloads do not sync local
settings or project files. This brand kit does not establish exclusive rights to
the Tervy name.
