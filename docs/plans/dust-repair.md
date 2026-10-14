# Dust-repair rendering continuity

Dust cleanup is repetitive canvas work. Already healed defects must remain healed while navigating or creating the next repair; falling back to the original defects interrupts inspection and makes completed work appear lost.

The [editor guide](../editor.md#provisional-dust-repair) describes the implemented direct-selection workflow. The [research guide](../research/dust-removal.md) records evidence for the frozen `ap1-boundary-ranked-score10-grow4-v1` candidate and its photographic limits. This remains an experimental prototype; main integration is deferred pending rendering continuity and explicit user-facing disclosure.

## Current limits

Evaluated AP1 patches are cached on the CPU. Settled refinement uploads them into temporary GPU tiles and retains only the resulting view-dependent GPU texture. A viewport or repair revision invalidates that texture; the interim image is an unrepaired provisional preview. Unchanged repairs need not be healed again, but their visible result disappears while the view rebuilds.

The worker also uses the viewer generation to reject queued repair evaluations. A view-only invalidation can therefore cancel otherwise valid repair work; generic failure memoization then suppresses retries for that same repair state. The Dust workflow test has intermittently timed out with `dust repair became stale before evaluation` in the parallel UI suite, while the seven workflow tests pass serially. Cancellation must be distinguished from a genuine unchanged evaluation failure.

Repairs remain transient, keyed by `SourceId`, and do not mark the project dirty. Save omits them; New/Open and application exit discard them. TIFF/EXR exports, thumbnails, scopes and approximate/cached previews remain unrepaired. The current inspector lacks an explicit warning that repairs are unsaved and unexported.

Automatic detector proposals remain unwired. Automatic donor search stays asynchronous and has no normal-use latency acceptance. Local undo remains deferred until its relationship with global undo is designed.

## Next slice

Separate repair-content validity from view-dependent reconstruction. Retain valid repair content in GPU-accessible storage and make both interactive and settled viewer paths consume it before ACES, without replacing the frozen healer.

The exact representation is open: sparse uploaded patches, repaired tiles or another bounded representation must be compared against the current renderer's reconstruction needs. A full-resolution retained AP1 image is not an approved requirement. GPU-only production output rendering is a possible later simplification, not a prerequisite or selected migration in this slice; keep the scalar numerical reference.

Add compact user-facing disclosure that the prototype is preview-only and repairs are not saved or exported. The limitation must remain discoverable after leaving cleanup, not only while its inspector is expanded.

### Preserve

- Immutable unrepaired signed/HDR AP1 is the input to every independent repair; repairs never consume one another's pixels.
- Manual and detected masks share the same engine, with manual support authoritative and separate supports non-overlapping.
- Preserve exact f32 bits outside repair support and introduce no clamp or pedestal.
- Keep the RGB16 retained source path and bounded asynchronous evaluation, including stale-source/settings/repair rejection and unchanged-failure handling.
- Cache validity includes source development, calibration, correction, grade and exact repair content. Pan/zoom changes view reconstruction, not repair content.
- Source changes and incompatible processing state must never display another source's or grade's patches.
- Keep canvas selection, unmodified creation, Shift-add, Option-remove, donor dragging and release-to-commit behavior. Delete/Backspace removes the selection; the inspector reset removes the photo's repairs.

### Acceptance before main integration

- Previously evaluated, unchanged repairs stay visible during pan, zoom, brush resizing and creation/editing of another repair, including while evaluation or settled refinement is pending.
- Interactive and settled views agree on repair placement through crop, orientation and straightening; repair composition respects existing process-before-reconstruction behavior rather than painting into a downsampled proxy.
- Editing one repair updates only its content; deletion and reset remove it without leaving stale GPU pixels.
- Changes to source, grade or correction reject incompatible caches; late results cannot restore deleted/reset repairs or mutate another source. View-only cancellation does not permanently suppress a still-current repair, while genuine unchanged failures still avoid endless retries.
- Resource retention and update costs are measured during a representative cleanup session; do not infer a universal memory or latency budget from one machine.
- Composed UI and GPU tests cover continuity, invalidation and unchanged-pixel behavior, followed by a real-photo navigation/cleanup pass.
- The UI clearly identifies unsaved/unexported cleanup, with no persistence or repaired-output claim.

## Deferred scope

Persistence/schema changes, global/local undo design, TIFF/EXR export, thumbnails and scopes need separate integration work. A shared repaired-AP1 boundary may help those consumers, but caching alone does not connect them. Automatic proposal presentation must preserve editable masks without coupling detector scores to healing. Do not reopen Lightroom comparison, add neural downloads or retune the retained corpus.

Collect residuals, donor failures, support rejections and latency through normal use before changing the frozen healer. Clean-truth accuracy, real defects crossing fine in-focus wires, detector precision/recall, diagnostic calibration and physical-monitor acceptance remain unverified.

## Verification boundary

The default numerical tests, composed Dust workflow and UI tests, editor/UI Clippy and experiment tests cover implementation consistency, not photographic or physical-input acceptance. Private numerical reproduction covers 69 retained cases; the donor rectangles and known-pixel bits agree, with three one-ULP repaired-channel differences. Those generated inputs/results remain ignored and are not distributed with the repository.

The editor/UI library suite has a known unrelated failure in `idle_roll_prefetch_reaches_unshown_photos_after_visible_work`. The broader editor integration suite also has a pre-existing diagnostic-wording failure in `gpu_range_gate_covers_above_base_continuation_after_camera_mapping`. Keep those failures explicit rather than weakening gates or silently repairing them in this slice.

Use the [development checks](../development.md#framework-ui-regressions) and [experiment checks](../../scripts/dust-experiments/README.md#inspect-and-check). GPU continuity acceptance is not implemented or verified yet.

## Exact next action

Reproduce the Dust stale-evaluation timeout and trace repair-content validity separately from viewer generations, source/patch ownership and both reconstruction paths. Then propose the smallest bounded GPU representation and cancellation/invalidation model that satisfies continuity. Settle that design before implementing it; do not expand into output or persistence migration as part of the cache work.
