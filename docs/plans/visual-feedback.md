# Thumbnail follow-up

Status: active thumbnail investigations. Bounded/resizable thumbnails and [responsive scopes](completed/vectorscope-and-waveform.md) are implemented and integrated on main; neither is an outstanding implementation step. Historical thumbnail checks remain below. [Roadmap](../../PLAN.md) owns order; follow-up fixes require selection/approval.

## Goal and non-goals

Resolve reported thumbnail layout/invalidation/latency issues without retaining one full RAW per thumbnail, reading back full-resolution frames each repaint or degrading interactive GPU edits. No catalog, automatic correction or new colour-processing model.

## Completed thumbnail slice

1. [x] Add a bounded stable-ID source/image cache and one-outstanding-request worker. Keep the selected `Scan` untouched; develop at most one temporary thumbnail RAW, extract an exact RGB16 nearest-center proxy (maximum dimension 256), then release it. Dispatch visible missing sources only after selected loading/picking/export/refinement is idle; cancel/reject old work across foreground/document transitions.
2. [x] Reuse existing GPU processing/ACES/managed presentation. Without calibration show developed source at zero inspection EV; with calibration show the actual roll/per-photo positive grade. Source/WB/fingerprint/development and output target key images; accepted grade changes only rerun GPU processing, not RAW development. Bounded worker RGB16 and UI GPU caches each retain at most 16 entries (worst-case resident pixel storage ~48 MiB together, excluding driver overhead/one temporary RAW). No whole-roll RAW cache, CPU viewer or new ACES algorithm.
3. [x] Replace filename buttons with thumbnail/filename cells in the **existing resizable bottom panel**, default 140 points, range 52…240 points (also limited by window height). Thumbnail display size follows panel height; horizontally virtualize visible cells, prioritize selected/nearby visible entries, and retain usable placeholders/error tooltips and existing normal/reference-pick selection behavior.
4. [x] Test cache bounds/identity/invalidation/cancellation, view-only size/dirty independence, source/positive/target/photo changes, errors, resize/scroll and worker/foreground isolation. Run focused GPU/editor checks and real-source smoke; update contracts/architecture/roadmap with observed limits. The no-commit restriction belonged to this historical checkpoint, not current delivery status.

Scopes, geometry, relink/recovery/undo and other input formats were not part of that thumbnail slice. The later [completed geometry integration](completed/flip-crop-rotate.md) now paints accepted composition over geometry-independent caches; native-camera model 4 and original-source flat-field correction are integrated. Resizing is transient UI state, not project schema. Cache caps bound this new cache, not a minimum-hardware or application-wide resource policy. Actual scheduling/performance is measured before introducing shared-renderer/atlas machinery.

## Triaged thumbnail follow-up

The bounded thumbnail slice is implemented; these new observations are open follow-up, not scope approval for a cache redesign. Read-only inspection covered `src/editor_thumbnails.rs` and the shared `ImagePaint` callback in `src/viewer.rs`; no new performance or visual run was made.

- **Partially visible thumbnails squish at scroll edges — reported rendering defect.** Cell/image geometry already preserves aspect ratio, so that alone does not disprove the observation. Reproduce at both edges and trace callback viewport/scissor clipping and UVs. Acceptance: partially visible thumbnails are cropped, not rescaled, at different strip heights and display scales. Add a focused clipping regression once the cause is isolated.
- **Thumbnails invalidate too easily — investigation.** Distinguish GPU parameter reprocessing, UI-image cache misses, proxy cache eviction and cancelled in-flight work. Source changes/document work cancel generations but do not directly clear the caches; roll/photo edits are not source-cache keys. Capture the triggering interaction, keys, cancellation and development counts for photo switching, grade changes, target changes, import/Save and scrolling beyond 16 sources. Reuse valid work without weakening source/development identity or stale-result rejection.
- **Cold thumbnail latency — measured concern, not a diagnosed GPU bottleneck.** Reported log: `thumbnail source_id=35 target=DisplayP3 source_cache_hit=false seconds=1.516`. This is end-to-end worker time for a proxy-cache miss, including source validation, full RAW development, proxy extraction and GPU image creation. It is consistent with the earlier 1.40–1.51 s cold-source checkpoint; cached proxies previously took 0.002–0.003 s. Instrument phases and cold/warm/rapid-switch cases before choosing an optimization. Preserve pinned development, selected-image priority and bounded RGB16 storage; do not introduce a full-roll RAW cache or speculative renderer machinery.
- **Thumbnail development spinner — missing.** The loading placeholder is a static ellipsis. Coordinate animated loading indicators with the [shared operation-feedback work](completion.md#operation-feedback); distinguish queued, developing and error states without perpetual idle repaint.

Next action when selected: reproduce the edge-squashing case, then capture the reported invalidation sequence and phase timings. Keep layout correctness, wasted work and cold development cost separate; scopes do not block these investigations.

## Remaining steps and acceptance

1. Select a reproduced thumbnail follow-up; existing bounded caches are not to be rebuilt from this checklist.
2. Isolate clipping, invalidation and cold-development cost using the cases above. Preserve selected-image priority, cache bounds/identity, stale-work rejection, source/positive/target behavior and calibration/measurement/export independence.

Scope conventions, GPU implementation and synthetic composition integration are closed in the [completed scope plan](completed/vectorscope-and-waveform.md). [Remaining scope acceptance/full-source work](completion.md#scopes) is separate.

Likely boundaries: editor/backend scheduling, `src/{viewer,refinement}.rs`, dedicated thumbnail/scope resources and shaders, synthetic tests and targeted performance probes. Production colour/measurement/export contracts remain unchanged.

## Progress / handoff

Thumbnails are implemented in `src/editor_thumbnails.rs`: one background request, two 16-entry caches, verified source identity and transient resizable strip. Cell width keeps a visible page within cache capacity even at short strip heights; image aspect is preserved. Current classic Original and Film shapes both use native-camera model 4 and their actual GPU response, with accepted composition applied during painting and gains evaluated in original source coordinates. No thumbnail influences measurements/exports. Cancellation is between phases: LibRaw development and already submitted GPU work are not interruptible. A cancelled request remains outstanding until completion, preventing a queued backlog. Missing/changed/development-error sources retain a visible error marker and tooltip, not a fallback image.

Historical thumbnail verification snapshot on the pre-camera/geometry baseline (not rerun for this plan audit): **79 focused tests passed** (65 library, 7 project, 3 film-project, 4 viewer), including cache/identity/cancellation/disconnection, parameter source/roll/photo behavior and egui resize/selection tests; strict all-target Clippy, formatting and release editor build pass. The real-source Save/Open smoke now waits for all four thumbnails; its initial run submitted **906 onscreen frames** and preserved exact model-2/model-3/WB/output/document checks. The final repeat also passed persistence/GPU/four-thumbnail assertions but failed **`editor smoke acquired no visible frame`**; macOS reported **`CGSSessionScreenIsLocked=true`**. The visible-frame gate was retained, not bypassed. New RAW thumbnails took **1.40–1.51 s** on M5 Pro, cached-source GPU images **0.002–0.003 s**. This is a four-source checkpoint, not large-roll, whole-process memory, broader-device or physical-monitor acceptance. No full export rerun or externally driven physical resize/trackpad exercise.

User follow-up exposed a resize defect: content used six points less than the panel height, so egui persisted a six-point shrink on every idle frame. The panel now explicitly consumes its available height; default height is increased to **140 points**. The resize regression first reproduced **144→138 points** after release, then passed with the fix, checking the fresh default and 20 idle frames. At that correction checkpoint, all 51 focused editor tests (including six thumbnail tests), strict all-target Clippy, formatting and release build passed; no new real-RAW/physical resize smoke is claimed. Thumbnail worker/development behavior and latency remain unchanged.

Responsive scopes independently sample the accepted whole composition from the whole-source proxy; regional settled refinement is never their input. No thumbnail optimization or full-source scope analysis is presumed next.

Context: [historical feedback scope](../research/history/feedback-plan.md#4-add-visual-feedback-thumbnails-and-scopes), [architecture](../architecture.md#preview-resources-and-caches).
