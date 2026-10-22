# License completion and release workflow

## Agreed scope

Two follow-ups from the CI milestone, on `origin/main` at `447921f`:

1. **License inventory completion.** The packaging notice collector reports 13+ upstream crate records with no collected notice files (published archives omit them). Resolve each the way the earlier batch was resolved: pin the crate's `.cargo_vcs_info.json` commit, fetch the actual upstream license text at that commit, record provenance in `licenses/rust-notices.json`; crates whose upstream genuinely lacks license text stay explicitly marked unresolved rather than guessed. Keep the packaged `Licenses/` inventory and `NOTICE.md` consistent with what ships.
2. **Tag-triggered release workflow.** `.github/workflows/release.yml` on `v*` tags: run the maintained `cargo xtask package`/`verify-package` stages, then publish a GitHub Release with the ZIP artifact plus SHA-256 checksums. Validate tag/version agreement with `Cargo.toml`. Publication form (draft vs. published-on-tag) pending the user's answer.

No Metal-dependent suites in hosted jobs (no self-hosted runner — user decision: agents/local run the Metal suites manually before merges, as already documented). No clean-machine/macOS-14 acceptance or public visibility flip in this task.

## Acceptance

- `cargo xtask package` produces a notice inventory with no silently missing records: every dependency either carries collected upstream license files or an explicitly pinned supplemental notice, or is listed as unresolved with a reason.
- The release workflow is syntactically valid and observed green end-to-end on a hosted runner from a test tag, producing a Release with ZIP + checksum.
- Docs (`docs/build-and-delivery.md`, `NOTICE.md` as needed) describe the release procedure and its boundaries accurately.

## Remaining steps

1. ~~Enumerate unresolved notice records~~ — done: the 13 records were the objc2 0.6.x family, dispatch/dispatch2, and zune-inflate.
2. ~~Extend supplements + tests~~ — done: `text` mode pins upstream notice bytes verbatim (pointer-style `LICENSE.md` files where upstream ships no full text), provenance records the rewritten-away VCS commits; tests cover both modes. Packaging now emits **zero unresolved warnings**.
3. ~~Write release.yml~~ — done: tag-triggered, validates tag vs crate version, runs hosted verification, publishes Release with ZIP + checksums directly.
4. Open the PR, watch CI green, hand to the user for merge.

## Verification record

- All 14 script tests pass; formatting and docs gates pass.
- Real `cargo xtask package` run: zero unresolved upstream-notice warnings (was 13); every crate directory in `Licenses/Rust/` carries collected files, a supplement, or a verbatim pinned notice with `notice-provenance.json`.
- `cargo xtask verify-package` passes on the produced ZIP.
- Release YAML parses; tag-validation snippet exercised locally against `cargo metadata`.

## Current constraints and notes

- Root checkout still carries the other agent's session (27 dirty files); untouched.
- objc2 family discovery: crates.io-pinned VCS commits `b4167b5`, `8852b42`, `82d6c7`, `7b1abf` no longer exist upstream (force-push); the repo ships only pointer-style `LICENSE.md` at reachable commits. `dispatch` 0.2.0's tree has no license file at all. These facts are recorded in the entries' `note` fields.
- Before any real `v*` tag: license review and clean-machine acceptance are still on the user (documented in `docs/build-and-delivery.md`).