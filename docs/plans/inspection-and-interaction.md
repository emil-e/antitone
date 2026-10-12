# Interaction and colour diagnosis

Status: unfinished fixes and UX/design questions. [Roadmap](../../PLAN.md) owns priority. Preserve full-resolution picks, source anchors, atomic rejection/cancellation and independence from calibration/export. Current behavior: [controls](../contracts/editor-controls.md), [geometry](../contracts/geometry.md).

## Decisions and open questions

- Reproduce DSC04524 dark reds using the exact RAW, calibration, response and Roll/Photo settings. Trace corrected camera RGB → inversion → camera→AP1 → photographic edits → rendered target. A cause is not established; [native-camera inversion](../decisions/native-camera-inversion.md) remains the processing basis.
- Investigate cold near-base shadows with Vision3 500T. Greater blue sensitivity is a hypothesis, not an established cause. Trace signed working channels and rendering with a reproducible current grade; do not silently warm shadows or add a pedestal/clamp.
- Reproduce stale fitted-resolution symptoms across Fit/native/resize/rapid switching. Validate dimensions, adapter limits and stale-result rejection without skipping refinement, weakening gates or adding perpetual redraw.
- Consult on each existing UI insertion that changes viewer size before choosing a replacement. Gather current screenshots and discuss tab prominence, Output placement and action hierarchy.

## Triaged interaction follow-up

### Focused fixes

- **Hand cursor:** zoomed primary drag can pan, but hover shows ZoomOut except with Space. Show Grab when dragging would pan, preserving picker priority, disabled state and click-to-Fit. Extend navigation cursor tests.
- **Roll tab on New:** successful File → New retains the selected tab. New from Photo should select Roll; ordinary imports must not reset it. Open behavior needs an explicit choice.
- **Save prompt:** discuss centered owner-relative placement/modality; preserve Save/Discard/Cancel and failure behavior.

### Roll workflow

- **RAW drag/drop:** use staged import/identity validation. Settle supported-file feedback, busy-state drops, duplicates and all-or-nothing failure; wider format support is not implied.
- **Selection/removal:** clarify multi-selection versus removal selection. Define active/last-photo removal, next selection, dirty state, in-flight work and reference-only records. Removal is project-only: never delete RAWs or change RGB calibration. Undo is separately scoped.

### Layout and controls

- **Bottom preview toolbar:** discuss grouping/placement of Fit/100%/zoom, source preview and inspection EV relative to the filmstrip. Consider tooltips/accessibility names rather than assuming icon-only controls; consult on viewer-size changes.
- **UX guidelines/copy:** agree visible information versus tooltips, hierarchy, icons, dialog placement, progress and viewer-space stability. Reduce redundant descriptions while keeping actionable errors and essential state visible. Record agreed rules in the control contract, not implementation narration.
- **Wheel mapping:** identify the wheel/gesture that appears asymmetric before changing it. Equal artwork sectors do not imply equal processing response: gamma controls camera-channel log gamma; WB uses the local AP1 response to Bradford log-cone gains. [Mapping](../contracts/editor-controls.md#resolve-style-wheel-orientation-and-mapping).

## Recorded support/architecture questions

Current input support and the CPU/GPU boundary should be clear to users: exposed inputs are ARW RAWs and schema-5 JSON projects, not developed JPEG/TIFF/EXR. CPU LibRaw development, full-resolution f64 sampling/fitting/validation and export/reference paths coexist with GPU-only interactive processing. [Product](../contracts/product.md), [architecture](../architecture.md#ownership-and-execution).

## Next action and acceptance

Select one fix or design question. Reproduce symptoms against current composed views, then implement only that slice with focused event/geometry/state checks. Preserve picker/crop priority and view restoration; physical trackpad feel, popup appearance and monitor matching need real-use checks.

[Thumbnails](visual-feedback.md) · [Progress, photographic controls and release](completion.md)
