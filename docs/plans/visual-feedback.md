# Bounded thumbnails and whole-image scopes

Scope basis: scheduled in the approved feedback amendment; implementation and detailed scope basis are not started. [Roadmap](../../PLAN.md) owns order/status.

## Goal and non-goals

Add useful filmstrip thumbnails and whole-image vectorscope/waveform feedback without retaining one full RAW per thumbnail, reading back full-resolution frames each repaint, degrading interactive GPU edits or implying that a zoomed regional scope represents the whole frame. No catalog, automatic correction or new colour-processing model.

## Design to settle first

- Thumbnail source/positive behavior before calibration, processing/development invalidation key, selected/adjacent prioritization and explicit memory budget.
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

Filename-only filmstrip is implemented; thumbnail cache and scopes are not. Existing settled refinement covers the visible source region, which cannot silently be reused as a whole-frame scope.

Next action when selected: inspect scheduling/resource ownership and propose the smallest bounded thumbnail and explicit scope-basis design. Do not implement every later completion feature before returning to Save/Open.

Context: [historical feedback scope](../research/history/feedback-plan.md#4-add-visual-feedback-thumbnails-and-scopes), [architecture](../architecture.md#preview-resources-and-caches).
