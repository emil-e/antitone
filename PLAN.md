# Antitone — roadmap

## Current state

The editor supports native-camera model-4 inversion, roll flat-field correction, Analyze roll, full-resolution calibration, crop/straightening, Photo contrast/pivot and saturation, GPU previews and visible-image scopes that follow pan/zoom, thumbnails, Save/Open and TIFF/AP0 EXR export. [Topic guides](docs/README.md#understand-the-system) explain the models and important boundaries; [architecture](docs/architecture.md) maps the implementation.

[Local gradient adjustments](docs/plans/local-adjustments.md) and the [physical working-view prototype](docs/plans/view-working-set.md) are implemented, including source inspection and shared viewer/scope output. Bounded RAW/resource and normal-worker checks support the baseline. Photographic feel, preview/export correspondence, integrated physical latency, supported-domain usability and aggregate/in-flight memory under broader devices/pressure remain open. [Architecture](docs/architecture.md#ownership-and-execution) and [editor](docs/editor.md#a-stable-working-view-while-grading) own the lifecycle and resource contracts.

Project-wide creative looks are implemented through one embedded scene-referred LUT stage shared by preview, thumbnails, scopes, original references and TIFF. None, EXR and measurement/repair inputs retain their explicit bypasses. Built-ins resolve natively; imports remain available for session comparison. [Colour/output](docs/colour-and-output.md#creative-looks-precede-output-rendering) and [the catalog](assets/looks/README.md) own the contract. Final LUT density/smoothness, broader photographic/device acceptance and source/asset release clearance remain open.

The [provisional manual Dust workflow](docs/editor.md#provisional-dust-cleanup) is implemented and accepted for initial native use. [Source-owned repair instructions](docs/projects.md#source-owned-dust-repairs) save/reopen with deterministic reconstruction. TIFF/EXR and thumbnails remain unrepaired; [repaired output and further photographic, latency/resource and physical-input acceptance](docs/plans/dust-repair.md) remain open. Undo and automatic proposals are separate.

The Roll → Photo → Rendering workspace's visual grouping is accepted. [Physical input, preference relaunch and broader photographic/device acceptance](docs/plans/inspection-and-interaction.md) remain open. Shared presentation and operation feedback are implemented; [broader failure coverage, additional native/export progress and cancellation, and native acceptance](docs/plans/completion.md#operation-feedback) remain unfinished.

Selected previews, visible thumbnails and filmstrip-only camera-preview placeholders are accepted for current use. Source reuse, optional CPU/GPU byte limits, eager CPU/disk thumbnails and native scheduling are implemented. [Broader interaction, photographic, normal-budget residency, pressure/aggregate-memory and hardware acceptance](docs/plans/visual-feedback.md) remain open; the dedicated Preferences dialog is deferred.

Photo-only full-display inspection uses current-Space borderless window coverage, zoom-group entry and auto-hiding shared controls. Controller/composed checks and the bounded native display-frame/active-Space/paint/window-restoration gate pass. [Broader native lifecycle, physical input and display acceptance](docs/plans/inspection-and-interaction.md#layout-and-controls) remain open.

Roll multi-selection and confirmed project-only removal preserve RAWs, retained source grades and Roll calibration/correction. The active photo has distinct card framing, and empty rolls can be saved/reopened and repopulated. [Physical-input and accessibility acceptance](docs/plans/inspection-and-interaction.md#roll-workflow) remains open; undo is separate.

Analyze's [initializer and sampling-area review](docs/roll-analysis.md) are accepted for current use. [Photographic acceptance and deferred sampling/dense-tail work](docs/plans/roll-assistance.md) remain follow-ups. Explicit [Auto-crop](docs/geometry.md#automatic-framing-consumes-measurements-not-sampling-support) is implemented for Photo/Roll/selection drafts using strong retained frame evidence. [Bare-light correction support, independent evidence requests and broader framing acceptance](docs/plans/region-detection.md) remain separate: sampling acceptance alone does not install crops or prove physical border accuracy. Recovery/relink/undo remains lower priority than interaction and photographic acceptance.

## Remaining work, in priority order

1. **Remaining region consumers:** scope physical bare-light/reference support, feature-neutral lazy measurement requests and broader photographic crop acceptance separately from the delivered Analyze review and first retained-evidence Auto-crop consumer. Do not treat statistical support as a physical boundary or install correction/crops as an implicit analysis side effect. [Scope](docs/plans/region-detection.md).
2. **Responsiveness and resources:** real-use scrolling/control feedback, actual memory-pressure exercise and aggregate memory assessment. Measure switching caches or source-loading costs only for a concrete need. [Scope](docs/plans/completion.md#rendering-and-thumbnail-performance).
3. **Photographic acceptance:** camera inversion, native/settled straightening agreement, rotated neutral picks, corrected/uncorrected Save/Open and TIFF/AP0 EXR reopen; full-resolution evaluation of the implemented [contrast/saturation tools](docs/processing.md#photo-contrast-and-saturation). [Geometry](docs/plans/completion.md#geometry), [photographic controls](docs/plans/completion.md#photographic-controls-and-bw-handling).
4. **UI polish, scopes and interaction:** scope-guide/composed-photo appearance, wheel accessibility, physical trackpad/preference-relaunch exercise, decision-dialog acceptance/native-shell design and dark-red/cold-shadow diagnosis. [Scopes](docs/plans/completion.md#scopes), [interaction](docs/plans/inspection-and-interaction.md), [thumbnails](docs/plans/visual-feedback.md).
5. **Completion features:** additional photographic controls/B&W design and shared operation feedback. [Scope](docs/plans/completion.md); [local gradient adjustment acceptance](docs/plans/local-adjustments.md).
6. **Export and release:** batch/progress/cancellation/metadata, dependency bundling/licensing, clean-machine testing, macOS 14, broader cameras/devices and physical-monitor matching/transitions. [Scope](docs/plans/completion.md#export-and-release-work).
7. **Persistence:** missing/changed-source UI, recovery and undo/redo. [Scope](docs/plans/durable-projects.md).

## Limits

Numerical and headless checks do not establish photographic, physical-input or monitor acceptance. No minimum chip/RAM target or universal numerical-domain coverage is established. Native runtime paths are checkout-local, not distributable packaging. Lightroom/Resolve interoperability remains unverified. Windows/HDR display output and custom RAW development are outside current scope.

Setup: [README](README.md). Checks: [development](docs/development.md), [diagnostics](docs/diagnostics.md).
