# Antitone — roadmap

## Current state

The editor supports native-camera model-4 inversion, roll flat-field correction, full-resolution calibration, crop/straightening, Photo contrast/pivot and saturation, responsive GPU previews and scopes, thumbnails, Save/Open and TIFF/AP0 EXR export. [Contracts](docs/README.md#contracts) define behavior; [architecture](docs/architecture.md) describes the implementation.

The workspace uses Roll → Photo → Rendering stages, collapsible inspector sections, independently saved UI preferences and stable viewer-bar tool identity/Apply/Cancel. The visual grouping is accepted; physical input, preference relaunch and broader photographic/device acceptance remain open. [Interaction follow-up](docs/plans/inspection-and-interaction.md).

Memory-256 cached pending selected previews and visible-thumbnail admission/repaint are implemented and accepted for current use. Filmstrip-only embedded camera-preview placeholders provide navigation while proper thumbnails develop, preserving the polished card layout and edge clipping. [Broader interaction/resource and photographic acceptance](docs/plans/visual-feedback.md) remain open. No selected-view JPEG adoption or persistent JPEG cache is selected. Recovery/relink/undo remains lower priority than interaction and photographic acceptance.

## Remaining work, in priority order

1. **Responsiveness and resources:** real-use scrolling/control feedback, actual memory-pressure exercise and aggregate memory assessment. Measure switching caches or source-loading costs only for a concrete need. [Scope](docs/plans/completion.md#rendering-and-thumbnail-performance).
2. **Photographic acceptance:** camera inversion, native/settled straightening agreement, rotated neutral picks, corrected/uncorrected Save/Open and TIFF/AP0 EXR reopen; full-resolution evaluation of the implemented [contrast/saturation tools](docs/plans/completed/contrast-and-saturation.md). [Geometry](docs/plans/completion.md#geometry), [photographic controls](docs/plans/completion.md#photographic-controls-and-bw-handling).
3. **Scopes and interaction:** scope-guide/composed-photo appearance, physical trackpad/preference-relaunch exercise, dialog/native-shell decisions and dark-red/cold-shadow diagnosis. [Scopes](docs/plans/completion.md#scopes), [interaction](docs/plans/inspection-and-interaction.md), [thumbnails](docs/plans/visual-feedback.md).
4. **Completion features:** explicitly accepted/cancellable roll estimates, additional photographic controls/B&W design, shared operation feedback and source readability. [Scope](docs/plans/completion.md).
5. **Export and release:** batch/progress/cancellation/metadata, dependency bundling/licensing, clean-machine testing, macOS 14, broader cameras/devices and physical-monitor matching/transitions. [Scope](docs/plans/completion.md#export-and-release-work).
6. **Persistence:** missing/changed-source UI, recovery and undo/redo. [Scope](docs/plans/durable-projects.md).

## Limits

Numerical and headless checks do not establish photographic, physical-input or monitor acceptance. No minimum chip/RAM target or universal numerical-domain coverage is established. Native runtime paths are checkout-local, not distributable packaging. Lightroom/Resolve interoperability remains unverified. Windows/HDR display output and custom RAW development are outside current scope.

Setup: [README](README.md). Checks: [development](docs/development.md), [specialist probes](docs/build-and-probes.md).
