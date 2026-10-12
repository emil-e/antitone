# Thumbnail follow-up

Status: unfinished clipping and loading-feedback work. [Roadmap](../../PLAN.md) owns priority.

## Goal and boundaries

Resolve thumbnail display/loading issues while preserving source identity, source-coordinate correction, grade/target behavior and foreground priority. No full RAW retained per thumbnail, full-frame readback on repaint or changes to measurements/export. [Current cache contract](../contracts/output.md#responsiveness-and-optional-caches).

## Triaged thumbnail follow-up

- **Edge squashing:** reproduce partial cells at both scroll edges and trace callback viewport/scissor clipping and UVs. Acceptance: partial thumbnails are cropped, not rescaled, at different strip heights/display scales. Add a focused regression after isolating the cause.
- **Invalidation/latency:** reproduce any slow sequence and distinguish grade rendering, cache eviction, cancelled development and a genuinely cold source. A source without a verified proxy needs development; identity verification cannot be skipped. Measure current behavior before changing retention or scheduling.
- **Development spinner:** replace the static loading ellipsis as part of [shared operation feedback](completion.md#operation-feedback), distinguishing queued/developing/error states and stopping animation when idle.

## Next action and acceptance

Reproduce edge squashing first, or agree loading presentation within the shared feedback design. Verify clipping, resize/scroll, switching, stale-work rejection, pressure behavior and foreground isolation. Physical resize/trackpad and broader device/monitor acceptance remain separate.

[Switching caches/resource assessment](completion.md#rendering-and-thumbnail-performance) · [Scope acceptance](completion.md#scopes)
