# Contributing to Tervy

Tervy is an open source Mac workspace for solo developers. Start with the
[product review and v1 priorities](docs/v1-product-review.md) if you are
considering a feature. For a substantial change, open an issue first so its
user problem and scope can be discussed.

## Build and check

Development currently requires an Apple Silicon Mac, Rust 1.99.0, Python 3.11
or later, and Apple tools with the macOS 26 Speech framework SDK. The core
app targets macOS 15 or later; meetings require macOS 26 or later. Install `cargo-deny`
0.20.2, then run:

```sh
./scripts/check.sh
cargo run
./scripts/bundle.sh
```

`scripts/check.sh` runs formatting, Clippy, ordinary tests, and dependency
license checks. The paid, interactive agent probes are ignored by default;
do not run them against personal projects or accounts as part of a routine PR.
Native UI and signed-package checks have separate acceptance steps in the
[v1 plan](docs/v1-product-review.md).

## Pull requests

- Describe the user problem and the visible behavior change. Link an issue for
  larger work.
- Add meaningful tests for lifecycle, persistence, or permission changes. For
  UI changes, include a screenshot or a short recording and note keyboard
  behavior and window size tested.
- Keep provider-specific flags and capabilities explicit. Avoid assuming a CLI
  version is stable; update contract probes when an integration changes.
- Do not include credentials, personal transcripts, agent rollouts, or captured
  terminal output in fixtures, screenshots, logs, or issues.
- Keep third-party license files and notices with vendored code. See
  [vendor/README.md](vendor/README.md) before modifying a pinned dependency.

Contributions to Tervy's code and documentation are accepted under the
[Apache-2.0 license](LICENSE). The official [brand assets](assets/brand/README.md#asset-rights)
have separate usage terms. Forks should use their own name and visual identity.

For a bug, use a GitHub issue with reproduction steps and expected behavior.
For a potential security vulnerability, follow [SECURITY.md](SECURITY.md).
