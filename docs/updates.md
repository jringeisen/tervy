# In-app updates

Tervy embeds Sparkle 2.9.6 in Mac bundles. `scripts/fetch-sparkle.sh` downloads
the pinned distribution and checks its SHA-256 digest. The bundle includes a
small Swift bridge so the GPUI Settings button and gear badge use Sparkle's
update checks and installer. Local review builds have no feed or key by default,
so they show no update controls.

## One-time setup

1. Obtain a Developer ID Application signing identity and configure a
   `notarytool` Keychain profile as described in the README.
2. The `tervy` Sparkle signing key has been generated in the maintainer's
   Keychain. Its public half is in `assets/update-public-key.txt`. Back up the
   private key securely; it must remain available to sign future updates.
3. Make `github.com/jringeisen/tervy` public before the first updater release.
   Use `https://raw.githubusercontent.com/jringeisen/tervy/main/appcast.xml` as
   `TERVY_UPDATE_FEED_URL`. This stable URL serves the signed feed from `main`.
   Each versioned DMG needs a stable public HTTPS download URL. GitHub Release
   assets are the usual location.
   The feed URL and public key are embedded in each public app bundle.

## Every release

1. Increment `workspace.package.version` in `Cargo.toml` and update
   `Cargo.lock`. A Git push alone does not create a user update.
2. Export `TERVY_SIGNING_IDENTITY`, `TERVY_NOTARY_PROFILE`, and
   `TERVY_UPDATE_FEED_URL`, then run `scripts/release.sh`. The public path
   requires all three values and embeds the checked-in public key.
3. Copy `target/releases/Tervy-VERSION-macos-arm64.dmg` to the directory of
   public update archives, for example `target/update-feed/`. Optionally add
   `Tervy-VERSION-macos-arm64.md` in the same directory for release notes.
   Publish the DMG and checksum at a stable public HTTPS URL before publishing
   the feed. If the files are GitHub Release assets, the default download URL
   works. For another host, set `TERVY_UPDATE_DOWNLOAD_PREFIX` to the HTTPS
   directory URL ending in `/` when running `scripts/update-feed.sh`. You may
   keep older archives locally for rollback. The feed generator reads only the
   current archive and preserves prior feed entries.
4. Run `scripts/update-feed.sh target/update-feed`. This invokes Sparkle's
   `generate_appcast` using the private key in the `tervy` Keychain account and
   signs the update and feed. It preserves prior items from the root
   `appcast.xml`, points the new item to its public download location, verifies
   the signature, and copies the feed to the repo root. A secure CI job may
   instead supply an ephemeral private-key file through
   `TERVY_SPARKLE_KEY_FILE`; delete it after signing.
5. Commit and push `appcast.xml` after the public archive is available.
   Verify the public appcast URL and an older signed Tervy build can download
   and install the new version. Check that active sessions receive the
   stop-and-update confirmation and saved state reopens.

Sparkle's feed and archives use Ed25519 signatures in addition to Apple's
Developer ID signing and notarization. Never upload the private Sparkle key or
put it in the repository. The appcast is the release switch: clients only see a
version after its signed entry is published. Installed builds from before the
updater need one manual replacement.
