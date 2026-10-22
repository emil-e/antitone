# Continuous integration

## Agreed scope

Run the maintained local pipeline as GitHub Actions on PRs, pushes to main and manual dispatch, using the same commands contributors run locally. Keep coverage claims honest about hosted-hardware limits (Metal). No release publishing, GitHub deployment rules, branch protection automation or Windows/Linux jobs in this milestone; pushing the task branch to run Actions is in scope.

## Design

- `.github/workflows/ci.yml`, two jobs on `macos-15` (Apple Silicon): `checks` (formatting, docs gate, script tests, numerical core) and `editor` (vendored native build, DNG adapter tests, editor/all-features clippy, package and relocated verification, ZIP artifact upload).
- `cargo xtask tools` subcommand: installs only pinned formatters and warms Cargo downloads, so the checks job never builds native dependencies. `setup` = native setup + tools.
- Actions pinned to commit SHAs; minimal permissions; per-run concurrency cancellation.
- Cache pinned native dependencies and format tools keyed on the setup scripts, and Cargo downloads keyed on the lockfile.
- Hosted runners have no Metal device: editor library tests, kittest ui-tests and gpu tests are local/self-hosted coverage and are documented as such.

## Results

- PR #1 (`build/ci`): CI run #1 failed at Swift formatting — the macos-15 image's default Xcode 16.4 ships a `swift-format` too old for the checked-in `.swift-format` schema. Fixed by selecting Xcode 26.3 via `DEVELOPER_DIR` in the workflow (documented with the re-pin caveat).
- CI run #2 on `7081a75`: **success**. `format, docs and core` 1m17s (tools install, format/docs gates, 13 script tests incl. real signed-bundle fixtures, numerical core); `native build, editor and package` 8m34s cold (OpenMP/LibRaw/OCIO setup, vendored-SDK DNG adapter tests, editor + all-features clippy, package, relocated ZIP verification, ZIP artifact upload).
- Caches saved from the green run (native deps + format tools; Cargo registry) so subsequent runs skip the long cold setup.
- Local gates re-run on the final tree: YAML parse, formatting, docs (28 files, 0 broken links), script tests, xtask check/help.

## Remaining steps

1. Integration on request: the branch is verified green in Actions (run #39743819523, both jobs, ~8.5 min editor / ~1.2 min checks). GitHub `main` is 12 commits behind local main; integrating and pushing main will shrink PR #1's diff to the CI commit alone.
2. Optional follow-ups: branch-protection settings on GitHub (repo settings, not files); self-hosted/labelled Metal runner for the hosted-out suites; release workflow (out of scope).

## Current constraints and next action

- Hosted macOS images give no Metal device; do not add Metal-dependent suites to hosted jobs, and do not present CI green as full local `cargo xtask check` coverage.
- Action SHAs resolved: checkout v5.1.0 `fbc6f39…`, cache v5.1.0 `caa29612…`, upload-artifact v5.0.0 `330a01c4…` (lightweight tags).
- Next action: write the xtask subcommand and workflow.