# V1 release checklist

This is the evidence record for a v1 launch. Keep a dated copy with the release
notes and link logs or issues for failed steps. The public beta evidence below
does not complete every v1 product and data safety check.

## Public beta 0.1.7 — 2026-10-13

- [Tervy 0.1.7](https://github.com/jringeisen/tervy/releases/tag/v0.1.7)
  is public with a Developer ID signed, notarized, and stapled Apple Silicon DMG.
  Apple accepted the app submission `82da5fa0-5169-46eb-bff3-b493a28a372d`
  and DMG submission `a8827573-62ff-4a07-b6f8-ad29f114886a`.
- `./scripts/check.sh` passed locally. The packaged app passed deep signature
  verification and Gatekeeper assessment. The public DMG's SHA-256 is
  `b0dc17ffb5f055c083dca7fab4cad724963f45c0ce0453182deb57cf26945e67`.
- The public [signed update feed](https://raw.githubusercontent.com/jringeisen/tervy/main/appcast.xml)
  and DMG were downloaded without authentication; the feed signature and DMG
  checksum verified. Version 0.1.7 is the first updater-enabled build.
- Manual clean-account installation and an in-app upgrade from 0.1.7 to a later
  release remain to be observed. The product checks below remain the v1 gates.

## Candidate

- [ ] Pin a version and commit; run `./scripts/check.sh` and PR CI.
- [ ] Check vendored notices, Apache-2.0 license, reserved brand terms,
  contribution, support, and private security reporting paths.
- [ ] Confirm GitHub repository visibility and enable private vulnerability
  reporting before inviting public security reports.
- [ ] Build with `scripts/release.sh` using a Developer ID Application identity
  and notarytool profile. Record notarization acceptance, `codesign --verify`,
  `spctl --assess`, DMG checksum, and manifest.
- [ ] Check that `LSMinimumSystemVersion` is 15.0 and the signed recording
  helper has its microphone entitlement.
- [ ] Confirm Sparkle, its helpers, and the Swift bridge are signed by the
  release identity. Verify the embedded HTTPS feed URL and Ed25519 public key.

## Clean machine and upgrade

- [ ] On an Apple Silicon macOS 15 Mac/account, install the DMG, launch a
  terminal, run and edit a process, quit, and reopen. The recording action must
  explain the macOS 26 requirement.
- [ ] On macOS 26, repeat core work and explicitly grant and deny microphone
  and system audio permissions. Record, stop, retry a failed transcription,
  delete audio, and recover an interrupted recording. Check the signed helper.
- [ ] Test no agent CLI, Claude only, Codex only, and multiple installed CLIs.
  Verify install/sign-in guidance and provider capability labels.
- [ ] Back up real beta state after quitting. Replace an older build with the
  candidate and verify the previous application data directory migrates to Tervy.
  Confirm projects, prompts, sessions, meeting records and audio remain available;
  verify recording permissions with the new bundle identifier.
- [ ] Restore a backup to a clean account. Confirm prompts and meeting audio;
  test a corrupt archive and a target that already contains state.
- [ ] Update from the previous candidate, then roll back with an offline
  backup. Test the in-app update with no sessions, then with a running session
  and recording so the stop-and-update confirmation is exercised.

## Product and publication

- [ ] Observe several solo developers on their own projects for at least a
  week. Track first setup, return to work, recovery, crashes, and open P0 bugs.
- [ ] Publish a versioned GitHub Release with DMG, checksum, notes, supported
  macOS versions, experimental meeting caveat, update and rollback steps, and
  a support route. Upload the signed DMG before publishing the signed appcast.
  Publishing and changing repository visibility require the maintainer's final
  approval.

## Evidence collected on 2026-10-11

- [macOS 15 compatibility smoke](https://github.com/jringeisen/tervy/actions/runs/38156744316)
  passed on an Apple Silicon runner. It built the app bundle on macOS 26,
  transferred it to macOS 15, verified the ad hoc signature and minimum OS
  version, loaded the CLI entry point, and rendered a synthetic first app
  frame. A clean-account install and full core workflow remain open.
- After Tervy quit, an offline backup of existing Tervy state restored into
  an isolated temporary directory. The check verified the configuration,
  12 non-database files, SQLite integrity, one prompt, one meeting, and that
  restored meeting paths no longer referred to the original state. The current
  core also parsed the restored configuration and loaded one project, one
  agent, one prompt, and one meeting. The archive and restored copy were deleted
  after verification. An actual app upgrade and clean-account restore remain open.
- No Developer ID Application signing identity was available on the build Mac.
  Signing, notarization, Gatekeeper, and signed recording permissions remain
  open. The closed beta protocol is in [closed-beta-guide.md](closed-beta-guide.md).
