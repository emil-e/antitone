# Antitone — roadmap

## Current state

The editor supports native-camera model-4 inversion, roll flat-field correction, Analyze roll, full-resolution calibration, crop/straightening, Photo contrast/pivot and saturation, responsive GPU previews and scopes, thumbnails, Save/Open and TIFF/AP0 EXR export. [Topic guides](docs/README.md#understand-the-system) explain the models and important boundaries; [architecture](docs/architecture.md) maps the implementation.

The workspace uses Roll → Photo → Rendering stages, collapsible inspector sections, independently saved UI preferences and stable viewer-bar tool identity/Apply/Cancel. The visual grouping is accepted; physical input, preference relaunch and broader photographic/device acceptance remain open. [Interaction follow-up](docs/plans/inspection-and-interaction.md). Shared presentation theme/typography/metrics/actions, viewer tool groups, inspector sections/tabs/labelled rows, contact-sheet cards/captions, blocking decision dialogs, acknowledged flat-field/save/export failures and agent reuse rules are implemented. Working feedback uses one typed foreground task lifecycle with concise modal/inline/activity-strip presentation, completed-file Import/Open progress and status-bar-only export, with Analyze progress/Cancel available across stages and optional background feedback kept local. [Broader failure coverage, additional native/export progress and cancellation, and native acceptance](docs/plans/completion.md#operation-feedback) remain unfinished.

Memory-256 cached pending selected previews and visible-thumbnail admission/repaint are implemented and accepted for current use. Filmstrip-only embedded camera-preview placeholders provide navigation while proper thumbnails develop, preserving card layout and edge clipping. [Broader interaction/resource and photographic acceptance](docs/plans/visual-feedback.md) remain open. No selected-view JPEG adoption or persistent JPEG cache is selected.

Immutable full-resolution source reuse, persistent CPU/GPU optional byte limits, eager proper CPU/disk thumbnails and one native priority worker are implemented. Lazy support hints and navigation during pending photo loads pass authored/controller/composed tests and a bounded retained-hit exercise. Broader normal-budget residency, pressure/aggregate-memory and hardware acceptance remain [resource follow-up](docs/plans/visual-feedback.md); the dedicated Preferences dialog is deferred.

Photo-only full-display inspection uses current-Space borderless window coverage, zoom-group entry and auto-hiding shared controls. Controller/composed checks and the bounded native display-frame/active-Space/paint/window-restoration gate pass. [Broader native lifecycle, physical input and display acceptance](docs/plans/inspection-and-interaction.md#layout-and-controls) remain open.

Current focus is [Analyze photographic acceptance](docs/plans/roll-assistance.md). Analyze uses the shared UI components with full Photo-grade reset and conditional confirmation. Recovery/relink/undo remains lower priority than interaction and photographic acceptance.

## Remaining work, in priority order

1. **Responsiveness and resources:** real-use scrolling/control feedback, actual memory-pressure exercise and aggregate memory assessment. Measure switching caches or source-loading costs only for a concrete need. [Scope](docs/plans/completion.md#rendering-and-thumbnail-performance).
2. **Photographic acceptance:** camera inversion, native/settled straightening agreement, rotated neutral picks, corrected/uncorrected Save/Open and TIFF/AP0 EXR reopen; full-resolution evaluation of the implemented [contrast/saturation tools](docs/processing.md#photo-contrast-and-saturation). [Geometry](docs/plans/completion.md#geometry), [photographic controls](docs/plans/completion.md#photographic-controls-and-bw-handling).
3. **UI polish, scopes and interaction:** scope-guide/composed-photo appearance, wheel accessibility, physical trackpad/preference-relaunch exercise, decision-dialog acceptance/native-shell design and dark-red/cold-shadow diagnosis. [Scopes](docs/plans/completion.md#scopes), [interaction](docs/plans/inspection-and-interaction.md), [thumbnails](docs/plans/visual-feedback.md).
4. **Completion features:** Analyze photographic acceptance, additional photographic controls/B&W design, shared operation feedback and source readability. [Scope](docs/plans/completion.md).
5. **Export and release:** batch/progress/cancellation/metadata, dependency bundling/licensing, clean-machine testing, macOS 14, broader cameras/devices and physical-monitor matching/transitions. [Scope](docs/plans/completion.md#export-and-release-work).
6. **Persistence:** missing/changed-source UI, recovery and undo/redo. [Scope](docs/plans/durable-projects.md).

## Limits

Numerical and headless checks do not establish photographic, physical-input or monitor acceptance. No minimum chip/RAM target or universal numerical-domain coverage is established. Native runtime paths are checkout-local, not distributable packaging. Lightroom/Resolve interoperability remains unverified. Windows/HDR display output and custom RAW development are outside current scope.

Setup: [README](README.md). Checks: [development](docs/development.md), [diagnostics](docs/diagnostics.md).
