# Continuous integration

## Agreed scope

Run the maintained local pipeline as GitHub Actions on PRs, pushes to main and manual dispatch, using the same commands contributors run locally. Keep coverage claims honest about hosted-hardware limits (Metal). No release publishing, GitHub deployment rules, branch protection automation or Windows/Linux jobs in this milestone; pushing the task branch to run Actions is in scope.

## Design

- `.github/workflows/ci.yml`, two jobs on `macos-15` (Apple Silicon): `checks` (formatting, docs gate, script tests, numerical core) and `editor` (vendored native build, DNG adapter tests, editor/all-features clippy, package and relocated verification, ZIP artifact upload).
- `cargo xtask tools` subcommand: installs only pinned formatters and warms Cargo downloads, so the checks job never builds native dependencies. `setup` = native setup + tools.
- Actions pinned to commit SHAs; minimal permissions; per-run concurrency cancellation.
- Cache pinned native dependencies and format tools keyed on the setup scripts, and Cargo downloads keyed on the lockfile.
- Hosted runners have no Metal device: editor library tests, kittest ui-tests and gpu tests are local/self-hosted coverage and are documented as such.

## Acceptance

- Workflows syntactically valid and running green on the hosted runner from a cold environment.
- CI mirrors the documented local commands; docs state which suites CI does not cover.
- xtask `tools` verified locally and via CI.

## Remaining steps

1. Implement xtask `tools`, ci.yml and docs; run local checks.
2. Push `build/ci`, watch the Actions run, iterate on hosted-only failures.
3. Report results; integration only on request.

## Current constraints and next action

- Hosted macOS images give no Metal device; do not add Metal-dependent suites to hosted jobs, and do not present CI green as full local `cargo xtask check` coverage.
- Action SHAs resolved: checkout v5.1.0 `fbc6f39…`, cache v5.1.0 `caa29612…`, upload-artifact v5.0.0 `330a01c4…` (lightweight tags).
- Next action: write the xtask subcommand and workflow.