# Bounded thumbnails and whole-image scopes

Scope basis: the user selected bounded thumbnails after navigation/source inspection, which are now implemented. The user approved enlarging the existing filmstrip and requested adjustable size. The bounded thumbnail slice below is implemented; scopes remain separate. [Roadmap](../../PLAN.md) owns order/status.

## Goal and non-goals

Add useful filmstrip thumbnails and whole-image vectorscope/waveform feedback without retaining one full RAW per thumbnail, reading back full-resolution frames each repaint, degrading interactive GPU edits or implying that a zoomed regional scope represents the whole frame. No catalog, automatic correction or new colour-processing model.

## Thumbnail execution slice

1. [x] Add a bounded stable-ID source/image cache and one-outstanding-request worker. Keep the selected `Scan` untouched; develop at most one temporary thumbnail RAW, extract an exact RGB16 nearest-center proxy (maximum dimension 256), then release it. Dispatch visible missing sources only after selected loading/picking/export/refinement is idle; cancel/reject old work across foreground/document transitions.
2. [x] Reuse existing GPU processing/ACES/managed presentation. Without calibration show developed source at zero inspection EV; with calibration show the actual roll/per-photo positive grade. Source/WB/fingerprint/development and output target key images; accepted grade changes only rerun GPU processing, not RAW development. Bounded worker RGB16 and UI GPU caches each retain at most 16 entries (worst-case resident pixel storage ~48 MiB together, excluding driver overhead/one temporary RAW). No whole-roll RAW cache, CPU viewer or new ACES algorithm.
3. [x] Replace filename buttons with thumbnail/filename cells in the **existing resizable bottom panel**, default 140 points, range 52…240 points (also limited by window height). Thumbnail display size follows panel height; horizontally virtualize visible cells, prioritize selected/nearby visible entries, and retain usable placeholders/error tooltips and existing normal/reference-pick selection behavior.
4. [x] Test cache bounds/identity/invalidation/cancellation, view-only size/dirty independence, source/positive/target/photo changes, errors, resize/scroll and worker/foreground isolation. Run focused GPU/editor checks and real-source smoke; update contracts/architecture/roadmap with observed limits. No commit/push.

Scopes, geometry, relink/recovery/undo and other input formats are not included. Resizing is transient UI state, not project schema. Cache caps bound this new cache, not a minimum-hardware or application-wide resource policy. Actual scheduling/performance is measured before introducing shared-renderer/atlas machinery.

## Scope design to settle later

- Scope stage, common RGB basis, white/transfer, luma coefficients, bin scaling and signed/HDR boundary display. Different target RGB codes are not directly comparable.
- Coverage/resolution, provisional whole-frame versus full-source-derived scope states, relationship to rendering target, and graticule/wheel directions.
- Scheduling/cancellation so thumbnails/scopes do not displace selected-image refinement. Layout must respect the user's consultation request for viewer-intrusion cases.

## Steps and acceptance

1. Agree the above small design before introducing caches/shaders.
2. Generate/cache bounded thumbnails off the UI thread with selected priority and stale-work rejection. Test calibration-unavailable, loading/missing/error states, shared/photo invalidation and responsive navigation; no redecode merely from repaint.
3. Implement GPU scope reductions from the declared whole-image processing/rendering stage before monitor conversion. Reuse work where possible; avoid full-frame per-repaint CPU readback.
4. Compare synthetic neutrals, primaries/secondaries, ramps, bin counts and boundary cases against scalar fixtures. Verify whole-frame coverage under zoom/crop, target convention and measured memory/latency.

Likely boundaries: editor/backend scheduling, `src/{viewer,refinement}.rs`, dedicated thumbnail/scope resources and shaders, synthetic tests and targeted performance probes. Production colour/measurement/export contracts remain unchanged.

## Progress / handoff

Thumbnails are implemented in `src/editor_thumbnails.rs`: one background request, two 16-entry caches, verified source identity and transient resizable strip. Cell width keeps a visible page within cache capacity even at short strip heights; image aspect is preserved. Original/model-2 and Film/model-3 both use their actual GPU response; no thumbnail influences measurements/exports. Cancellation is between phases: LibRaw development and already submitted GPU work are not interruptible. A cancelled request remains outstanding until completion, preventing a queued backlog. Missing/changed/development-error sources retain a visible error marker and tooltip, not a fallback image.

Verification: **79 focused tests pass** (65 library, 7 project, 3 film-project, 4 viewer), including cache/identity/cancellation/disconnection, parameter source/roll/photo behavior and egui resize/selection tests; strict all-target Clippy, formatting and release editor build pass. The real-source Save/Open smoke now waits for all four thumbnails; its initial run submitted **906 onscreen frames** and preserved exact model-2/model-3/WB/output/document checks. The final repeat also passed persistence/GPU/four-thumbnail assertions but failed **`editor smoke acquired no visible frame`**; macOS reported **`CGSSessionScreenIsLocked=true`**. The visible-frame gate was retained, not bypassed. New RAW thumbnails took **1.40–1.51 s** on M5 Pro, cached-source GPU images **0.002–0.003 s**. This is a four-source checkpoint, not large-roll, whole-process memory, broader-device or physical-monitor acceptance. No full export rerun or externally driven physical resize/trackpad exercise.

User follow-up exposed a resize defect: content used six points less than the panel height, so egui persisted a six-point shrink on every idle frame. The panel now explicitly consumes its available height; default height is increased to **140 points**. The resize regression first reproduced **144→138 points** after release, then passed with the fix, checking the fresh default and 20 idle frames. All 51 focused editor tests (including six thumbnail tests), strict all-target Clippy, formatting and release build pass for this correction; no new real-RAW/physical resize smoke is claimed. Thumbnail worker/development behavior and latency remain unchanged.

Scopes remain unimplemented. Existing settled refinement covers the visible source region, which cannot silently become a whole-frame scope. Scope basis/coverage/design needs separate agreement; no additional implementation is presumed next.

Context: [historical feedback scope](../research/history/feedback-plan.md#4-add-visual-feedback-thumbnails-and-scopes), [architecture](../architecture.md#preview-resources-and-caches).
