# Shared repair-pipeline integration

## Agreement and workspace

Implementation and proportionate verification are authorized for shared accepted recipes, repaired original-sample TIFF/AP0 EXR, proper positive thumbnails and composed pending/failure behavior, including the bounded shared contextual status-indicator pattern. Checkpoint commits, rebase onto current main and look/repair reconciliation are now authorized. Advancing main, final delivery and push remain unauthorized.

- Branch: `feat/repair-pipeline`
- Worktree: `.worktrees/repair-pipeline`
- Base: main `5a6c6df`
- Preserve main's unrelated work, every other worktree and private samples.

The owning contracts are now in Processing, Colour/output, Projects and Editor. [The Dust plan](docs/plans/dust-repair.md) owns broader unfinished acceptance; this file is task-local handoff bookkeeping.

## Delivered implementation

- Source-owned immutable image-recipe capture and semantic neutral/repair identity, including explicit viewer/candidate overlays. Shared RAW evaluation is usable without Editor; the frozen f64-to-f32 neutral sampler, solver, schema 6 and model 4 are unchanged.
- Cold asynchronous export resolves captured applicable repairs on its own worker. Exclusion requires zero exact composition coverage. Failures block successful output and retain destination safety; no viewer-admitted subset or unrepaired fallback is used.
- Shared ordinary/compensated repair grading preserves original floor flags, untouched words, independent paired-member coordinates and EXR high/low/tail precision. Offscreen logical snapshots upload contributing pixels per bounded original unit rather than retaining every patch on the GPU.
- Proper calibrated cards use original-derived neutral AP1 with and without repairs; source inspection keeps camera-proxy semantics. RGB16 CPU/disk products stay source-owned and unrepaired. Recipe invalidation, stale replies, native scheduling and source leases remain with their existing owners; no new persistent cache or RAW worker was added.
- Compatible visible neutral products survive grade/target changes and zero-budget replacement/failure handoff. Cache charges include recipe-key masks, error entries, neutral/graded products and sparse evaluations; offscreen GPU admission accounts for positive products rather than the old camera proxy.
- Shared contextual activity/warning/error presentation, combined priority reasons, tooltip/accessibility descriptions and controller-owned retirement. Pending uses a static clock to avoid animation-driven hot loops for queued/suspended work. Completed cards have no permanent repair badge; Auto-crop/Analyze warnings are not hidden by activity. Contributor rules/examples are in `docs/editor.md`.
- Obsolete output-omission copy is removed, and owning guides/roadmap describe the implemented consumers.

Undo, detection/proposals, solver changes, general pipeline graphs, batch/general export-cancellation UI, recovery/relinking, RAW hashing/identity and broad photographic/device/resource acceptance remain outside scope.

## Verification and review

Passed:

- Default/core tests and project/camera/film/flat-field/Dust/recipe suites.
- Production worker cold Save/New/Open/export checks, TIFF/half/full AP0 EXR reopening and failed preparation destination preservation.
- Original-sample reference export suite: 28 passed, one diagnostic ignored; includes repaired serial/cached/forecasted/paired complete files, corrected sources, signed/HDR compensation, untouched words/flags, tile boundaries and large integer origins.
- Dust/controller/GPU/composed debug slice: 58 passed, one diagnostic ignored. Updated composed Save/New/Open/viewer/reference/scopes/proper-card case and pending/failure/disable status case pass; release Dust composed slice passes all four cases.
- Auto-crop compact warning and Analyze missing-area composed debug cases pass.
- Final thumbnail suite: 80 passed, three explicit diagnostics ignored; presentation suite: 56 passed. Authored checks cover retained neutral reuse through local/target changes and a failed-grade handoff at zero optional retention, key/error payload accounting, and offscreen positive admission.
- External export/encoder/geometry/local integration suites: 12 passed. Cold export also proves that a genuine no-donor repair outside exact cropped coverage does not block that output, while an applicable one preserves an existing destination on failure.
- Debug and release composed pending/no-donor/disable feedback pass at normal/narrow widths. Generated authored captures were visually inspected; they are in `.cache/ui-captures/`.
- Final production editor release build, strict UI/reference-feature Clippy, complete authored source-format checks, local Markdown link targets and diff whitespace checks pass. Production linkage was inspected and does not include OCIO.

One independent fresh-context review found three cache issues: recipe-key accounting, visible zero-budget neutral handoff, and offscreen positive-product admission. Each is addressed with targeted regression coverage. No repeat review is planned. Initial broad production editor run had 664 passes and nine thumbnail failures; their causes were investigated (an unintended filmstrip state sync, obsolete camera-proxy repaint expectations, and exact accessibility descriptions). The sync was removed, the obsolete test contract was replaced with proper-neutral reuse coverage, and descriptions now assert active identity while permitting contextual detail. Focused reruns cover the affected boundaries; the full 23-minute library run is not being repeated by default.

All artifacts stay in checkout-local ignored `.cache/` and `target/`. Shared native setup uses the documented `.cache/native-deps` link and absolute `LIBRAW_SOURCE`/`OCIO_PREFIX` overrides. No setup/download, private-sample study, native application automation or system installation was performed.

## Remaining steps

The planned baseline is implemented and verified on `5a6c6df`, uncommitted in this worktree. It is not yet verified or ready for current main.

Final refresh found main at `b4ae51a` (project-wide creative looks), after `aa3f29e` (editor frame/canvas extraction) and read-only crop diagnostics. The look commit changes the same processing input, working-view/rendering, export, thumbnail and LRU boundaries. This is a semantic integration requirement, not just a textual rebase:

1. Capture the accepted immutable look in the shared recipe/rendering intent. Look/target changes remain downstream of neutral healing; source inspection/measurements and EXR preserve main's bypasses.
2. Place repaired originals before main's look/ACES `RenderBinding`, retaining look failure markers/used-footprint checks and TIFF-only look metadata. Adapt both serial/paired exports and captured worker dispatch.
3. Adapt proper-thumbnail `RenderedView`/paint APIs, include look content in derived keys and account for look-result buffers/retained products in cache/admission without rehealing.
4. Preserve main's embedded-look persistence, current contributor docs, frame/canvas ownership and unrelated changes. Verify repaired-plus-look viewer/reference/scopes/TIFF/cards, look-free EXR/repair inputs and both None/selected-look routes.

User authorized checkpoint commits, rebase onto current main and reconciliation/verification. Start by checkpointing this verified baseline, then resolve the rebase and implement the semantic interactions above. Main fast-forward, final delivery and push remain separately unauthorized. Do not use destructive resets or overwrite main's uncommitted contributor/setup changes to bypass this checkpoint boundary.

## Limits

Authored fixtures establish actual native reuse, source/recipe fencing and bounded scheduling, not representative full-resolution thumbnail startup/whole-roll throughput. A cold calibrated positive needs original samples even if a disk RGB16 proxy exists. Larger acquisition/cache tradeoffs require evidence and an explicit decision; no second appearance definition or persistent cache was substituted.

Broader photographic correspondence, aggregate/in-flight memory, physical input, VoiceOver, native-shell/monitor and additional device acceptance remain in the owning topic plans. Look/repair reconciliation and verification on current main are in progress. Current main's newly accepted creative look is not yet represented in this worktree's recipe.
