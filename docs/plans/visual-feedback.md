# Thumbnail follow-up

Status: unfinished broader interaction/resource acceptance, embedded-preview design question, clipping and shared loading feedback. [Roadmap](../../PLAN.md) owns priority.

## Goal and boundaries

Resolve thumbnail display/loading issues while preserving source ownership/development compatibility, source-coordinate correction, grade/target behavior and foreground priority. No full RAW retained per thumbnail, full-frame readback on repaint or changes to measurements/export. [Current cache contract](../contracts/output.md#responsiveness-and-optional-caches).

## Triaged thumbnail follow-up

- **Edge squashing:** reproduce partial cells at both scroll edges and trace callback viewport/scissor clipping and UVs. Acceptance: partial thumbnails are cropped, not rescaled, at different strip heights/display scales. Add a focused regression after isolating the cause.
- **Visible-thumbnail responsiveness (broader acceptance):** exercise physical scrolling/grading while cold visible cells develop under the [current admission/cache contract](../contracts/output.md#responsiveness-and-optional-caches). Check foreground control/switching latency and actual OS pressure/aggregate memory; UI-cache timings and current-use feedback do not establish broader presentation or physical interaction acceptance. Cold sources still need development. Parallel-development pools and additional cache tiers are not selected.
- **Embedded previews (open question):** decide whether a clearly navigation-only interim is useful, how to distinguish it from positive thumbnails, and verify JPEG decode/upload, orientation/padding and managed display. [Header/extraction diagnostic](../build-and-probes.md#renderingimport-performance-probes) does not measure those later boundaries. JPEG/display-rendered data cannot enter native-camera RGB16 inversion, correction, picks, scopes or export. No embedded-preview production path is selected.
- **Development spinner:** replace the static loading ellipsis as part of [shared operation feedback](completion.md#operation-feedback), distinguishing queued/developing/error states and stopping animation when idle.

## Next action and acceptance

Next: reproduce partial-cell edge squashing when thumbnail follow-up resumes. A navigation-only embedded interim needs a separate design choice; shared loading presentation remains separate. Verify clipping, resize/scroll, switching, stale-work rejection, pressure behavior and foreground isolation. Physical resize/trackpad and broader device/monitor acceptance remain separate.

[Switching caches/resource assessment](completion.md#rendering-and-thumbnail-performance) · [Scope acceptance](completion.md#scopes)
