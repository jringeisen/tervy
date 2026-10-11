# V1 release checklist

This is the evidence record for a public release. Keep a dated copy with the
release notes and link logs or issues for failed steps. No public distribution
until the signed candidate and data safety checks pass.

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

## Clean machine and upgrade

- [ ] On an Apple Silicon macOS 15 Mac/account, install the DMG, launch a
  terminal, run and edit a process, quit, and reopen. The recording action must
  explain the macOS 26 requirement.
- [ ] On macOS 26, repeat core work and explicitly grant and deny microphone
  and system audio permissions. Record, stop, retry a failed transcription,
  delete audio, and recover an interrupted recording. Check the signed helper.
- [ ] Test no agent CLI, Claude only, Codex only, and multiple installed CLIs.
  Verify install/sign-in guidance and provider capability labels.
- [ ] Back up real beta state after quitting. Replace an older DevDesk build
  with the candidate without changing the data path or bundle identifier.
  Confirm projects, prompts, sessions, and meeting records remain available.
- [ ] Restore a backup to a clean account. Confirm prompts and meeting audio;
  test a corrupt archive and a target that already contains state.
- [ ] Update from the previous candidate, then roll back with an offline
  backup. Stop sessions and quit before replacing the app.

## Product and publication

- [ ] Observe several solo developers on their own projects for at least a
  week. Track first setup, return to work, recovery, crashes, and open P0 bugs.
- [ ] Publish a versioned GitHub Release with DMG, checksum, notes, supported
  macOS versions, experimental meeting caveat, manual update and rollback steps,
  and a support route. Publishing and changing repository visibility require
  the maintainer's final approval.

## Evidence collected on 2026-10-11

- [macOS 15 compatibility smoke](https://github.com/jringeisen/tervy/actions/runs/38156744316)
  passed on an Apple Silicon runner. It built the app bundle on macOS 26,
  transferred it to macOS 15, verified the ad hoc signature and minimum OS
  version, loaded the CLI entry point, and rendered a synthetic first app
  frame. A clean-account install and full core workflow remain open.
- After Tervy quit, an offline backup of existing DevDesk state restored into
  an isolated temporary directory. The check verified the configuration,
  12 non-database files, SQLite integrity, one prompt, one meeting, and that
  restored meeting paths no longer referred to the original state. The current
  core also parsed the restored configuration and loaded one project, one
  agent, one prompt, and one meeting. The archive and restored copy were deleted
  after verification. An actual app upgrade and clean-account restore remain open.
- No Developer ID Application signing identity was available on the build Mac.
  Signing, notarization, Gatekeeper, and signed recording permissions remain
  open. The closed beta protocol is in [closed-beta-guide.md](closed-beta-guide.md).
