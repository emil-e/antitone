# Shared repair-pipeline integration

## Agreement and workspace

Implementation, proportionate verification, checkpoint commits, rebase onto current main and creative-look reconciliation are authorized. Advancing main, final delivery and push remain unauthorized.

- Branch: `feat/repair-pipeline`
- Worktree: `.worktrees/repair-pipeline`
- Base: main `b4ae51a`; implementation checkpoint rebased as `2232021`.
- Preserve main's unrelated contributor/setup edits, other worktrees and private samples.

The owning contracts are in Processing, Colour/output, Projects and Editor. [The Dust plan](docs/plans/dust-repair.md) owns durable unfinished acceptance.

## Implemented acceptance

- Immutable source-owned accepted recipes capture upstream interpretation, enabled repairs, photographic/local grades, composition and the accepted numeric look. Neutral healing excludes downstream grade/look/target changes; the frozen solver/input contract and schema 6/model 4 are unchanged.
- Viewer/reference/scopes, TIFF/AP0 EXR and proper positive cards share accepted repair semantics. Cold export resolves applicable repairs independently of viewer warmth; exclusion requires zero exact output coverage. Applicable failures preserve destinations rather than producing unrepaired success.
- Ordinary/compensated repair grading preserves signed/HDR data, untouched words/flags and independent paired coordinates. TIFF applies the captured look after repair/grading; EXR bypasses it in pixels and metadata. Logical sparse results upload only demanded original-unit pixels.
- Proper cards use original-derived neutral AP1 with and without repairs, retaining existing source leases/native scheduling. Compatible neutral/graded prefixes survive look changes and visible zero-budget handoffs. CPU LUTs, GPU look allocations, recipe keys, failures and retained products participate in accounting/admission.
- Card look validation uses the displayed footprint. Explicit tool-preview framing participates in the consumer key without changing the accepted recipe. Embedded-look persistence and main's renderer/frame/canvas ownership are retained.
- Shared contextual activity/warning/error components preserve concurrent reasons and accessibility; completed cards have no permanent repair badge. Camera/JPEG/RGB16 navigation products do not silently become authoritative repaired positives.

No new persistent image cache, general pipeline framework, undo, detection/proposals, solver changes, batch/general cancellation UI, recovery/relinking, RAW hashing or broad photographic/device acceptance was added.

## Verification on `b4ae51a`

Passed:

- Default numerical/core suites; project Dust/recipe/look suites.
- Original-export reference suite: 30 passed, two explicit diagnostics ignored. Includes repaired-plus-look serial/cached/forecasted/paired complete files, correction/locals, signed/HDR precision and independent coordinates.
- External look/export tests: five passed, including repaired TIFF against scalar/OCIO and exact half/full EXR look bypass. Three production export-worker tests pass without OCIO, including cold Save/New/Open, captured look retirement during dispatch, exact repair exclusion and destination preservation.
- Thumbnail suite: 82 passed, three diagnostics ignored. Added repaired look/None neutral and graded-prefix reuse, transient framing and zero-budget payload/admission coverage.
- Composed look suite: six passed; Auto Crop: nine passed; Dust: four passed. Release Dust: four passed; release constant-scene repaired viewer/card/scope oracle: one passed. Normal/narrow authored looked-Dust captures were visually inspected.
- Shared presentation suite: 56 passed. Native look/sampling/storage components and idle projected-admission checks pass.
- Strict `ui-tests` all-target Clippy and production library/binary Clippy pass. Production release build passes; linkage contains no OCIO. Headless `raw,gpu,export` compilation passes.
- Complete authored source formatting, local Markdown file targets and diff whitespace checks pass.

Baseline received one independent review; its key accounting, zero-budget handoff and offscreen admission findings have regression coverage. No repeat review was performed. Current-main look interactions were verified with targeted component/composed tests, not a full editor-library rerun. The earlier full pre-rebase run was not wholly green; affected failures were fixed and rerun in focused suites, not hidden by claiming a full-suite pass.

Artifacts are ignored and worktree-local under `.cache/` and `target/`; reconciliation logs use `.cache/reconcile-*`, captures `.cache/ui-captures/reconciled-looks/`. Native setup uses the documented shared dependencies with absolute `LIBRAW_SOURCE`/`OCIO_PREFIX` overrides. No setup/download, private-sample study, native application automation or system installation was performed.

## Remaining limits and next action

- Production **all-target** Clippy still rejects main's pre-existing test-only `ScopeInput::frame` helper: it is gated on `test`, but all callers are `ui-tests`-only. `src/editor_scopes.rs` is unchanged from main. Production code linting and reference/UI all-target linting pass; the narrower configuration issue was not silently fixed or suppressed. Headless compilation also retains existing unused configuration-specific helper warnings.
- Full-resolution thumbnail startup/whole-roll throughput, aggregate/in-flight memory, photographic correspondence, physical input, VoiceOver and native-shell/monitor/device acceptance remain in the owning plans. Authored fixtures and declared payload bounds are not broader acceptance.
- Agreed implementation/reconciliation is ready at the task branch checkpoint. Await explicit integration/wrap-up authorization before advancing main or removing this worktree; push requires separate permission. Preserve this plan until final delivery, then transfer any lasting remaining scope and remove it according to repository guidance.
