# Thumbnail loading and resource follow-up

Status: unfinished broader interaction/resource and camera-preview photographic acceptance, plus shared loading feedback. [Roadmap](../../PLAN.md) owns priority.

## Goal and boundaries

Assess thumbnail responsiveness and resolve loading feedback while preserving source ownership/development compatibility, source-coordinate correction, grade/target behavior and foreground priority. No full RAW retained per thumbnail, full-frame readback on repaint or changes to measurements/export. [Current cache contract](../contracts/output.md#responsiveness-and-optional-caches).

## Triaged thumbnail follow-up

- **Visible-thumbnail responsiveness (broader acceptance):** exercise physical scrolling/grading while cold visible cells develop under the [current admission/cache contract](../contracts/output.md#responsiveness-and-optional-caches). Check foreground control/switching latency and actual OS pressure/aggregate memory; UI-cache timings and current-use feedback do not establish broader presentation or physical interaction acceptance. Cold sources still need development. Parallel-development pools and additional cache tiers are not selected.
- **Embedded previews (acceptance):** inspect camera-preview appearance/readability and camera-framing/positive-composition replacement on new uncalibrated rolls and saved calibrated projects, including rotation/padding and both monitor targets. [Current contract](../contracts/editor-controls.md#filmstrip-thumbnails) and [readiness/bridge probes](../build-and-probes.md#renderingimport-performance-probes) describe the implemented filmstrip-only path. Focused decode/ICC/EXIF, stale/missing/cancellation/pressure, uncalibrated/calibrated isolation and proper/error replacement tests plus sampled native-development and scripted Save/Open checks do not establish physical gestures, photographic framing/colour acceptance or broader-camera coverage. No selected-view JPEG adoption, persistent cache or alternate inversion is selected.
- **Development spinner:** replace the static loading ellipsis as part of [shared operation feedback](completion.md#operation-feedback), distinguishing queued/developing/error states and stopping animation when idle.

## Next action and acceptance

Next: exercise physical scrolling/grading with cold visible camera previews and developed thumbnails, assess foreground latency and actual memory pressure, and inspect photographic framing/colour replacement on new and saved calibrated rolls. Shared loading presentation remains separate. Verify resize/scroll, switching, stale-work rejection, pressure behavior and foreground isolation. Physical resize/trackpad and broader device/monitor acceptance remain separate.

[Switching caches/resource assessment](completion.md#rendering-and-thumbnail-performance) · [Scope acceptance](completion.md#scopes)
