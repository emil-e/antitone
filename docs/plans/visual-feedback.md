# Thumbnail follow-up

Status: active layout/operation-feedback investigations. Resizable thumbnails, responsive scopes and the [rendering/cache performance package](completed/rendering-performance.md) are implemented. [The roadmap](../../PLAN.md) determines priority.

## Goal and boundaries

Resolve reported thumbnail display/loading issues without retaining full RAWs per thumbnail, reading back full-resolution frames each repaint or degrading interactive GPU edits. Preserve source identity, original-source correction, grade/target behavior and independence from measurements/exports. No catalog, automatic correction or new colour model.

## Implementation status

The existing bottom strip uses aspect-preserving thumbnail cells, horizontal virtualization, normal/reference selection and transient height adjustment: default 140 points, range 52…240 points, also window-limited. The available panel height is explicitly consumed, so idle frames do not shrink a resized strip.

Current thumbnail work uses verified uncorrected RGB16 proxies, shared GPU resources, protected byte-budgeted caches and optional bounded disk proxies—not the historical 16-entry cache policy. Accepted composition is applied during painting, while flat-field gains use original source coordinates. Selected-image work has priority; one outstanding request prevents a backlog. Cancellation occurs between phases and cannot interrupt an executing LibRaw call or already submitted GPU work. Missing/changed/development-error sources show an error marker/tooltip rather than a fallback image.

Current resource/identity requirements are in [architecture](../architecture.md#preview-resources-and-caches) and [the output contract](../contracts/output.md#responsiveness-and-optional-caches).

## Triaged thumbnail follow-up

- **Partially visible thumbnails squish at scroll edges — reported defect.** Existing aspect-preserving geometry does not disprove the observation. Reproduce at both edges and trace callback viewport/scissor clipping and UVs. Acceptance: partial thumbnails are cropped, not rescaled, at different strip heights and display scales; add a focused regression once the cause is isolated.
- **Eviction/invalidation — performance follow-up implemented.** Visible-set protection, recent retention, selected proxy reuse and narrower cancellation replace the old fixed-entry behavior. Any remaining sequence should distinguish parameter rendering, cache eviction and cancelled development; new source content still requires explicit identity verification.
- **Cold development — measured and cached.** The reported cache-miss latency was end-to-end, not GPU-only. Full pinned development dominates cold requests after renderer warm-up; shared/prewarmed resources and verified persistent proxies reuse completed work. A never-developed source without a disk proxy still needs development. [Measurements/limits](../research/evidence/rendering-performance.txt).
- **Development spinner — missing.** The loading placeholder remains a static ellipsis. Coordinate animation with [operation feedback](completion.md#operation-feedback), distinguishing queued, developing and error states without perpetual idle repaint.

## Remaining steps and acceptance

1. Reproduce the edge-squashing case and isolate its viewport/scissor/UV cause; preserve the current cache/source path.
2. Agree thumbnail loading presentation as part of shared operation feedback.
3. Verify clipping, resize/scroll, switching, stale-work rejection, pressure behavior and foreground isolation for the selected change. Physical resize/trackpad exercise and broad device/monitor acceptance remain separate from headless tests.

Higher-resolution switching caches and broader real-use/resource feedback are in [completion work](completion.md#rendering-and-thumbnail-performance). Scopes independently use the accepted whole-frame proxy, not regional refinement; [scope acceptance/full-source work](completion.md#scopes) is separate.

[Archived thumbnail checkpoints](../research/history/implementation/visual-feedback.txt) retain the original bounded-cache tests, resize-defect diagnosis, latency measurements and locked-session visible-frame failure. They are superseded implementation history, not instructions to rebuild the old caches.
