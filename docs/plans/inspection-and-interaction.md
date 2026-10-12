# Source inspection and interaction feedback

Status: active interaction/layout/colour follow-up. Navigation, source inspection, thumbnails and the [preview buffer-limit fix](completed/preview-stability.md) are implemented; [thumbnail follow-up](visual-feedback.md) tracks remaining strip defects. The [roadmap](../../PLAN.md) determines priority among the outstanding requests below.

## Goal and boundaries

Make developed-source inspection and Mac image navigation usable without changing calibration, export, response, RAW development or ACES. Preserve full-resolution picker coordinates, atomic rejection/cancellation and working-photo/view restoration.

## Decisions and open questions

- Normal two-finger scrolling should pan; pinch zoom remains pointer-anchored. Retain an explicit modified-wheel zoom path for mouse users without applying gestures twice.
- Outside picker mode, a fitted-image click should zoom to native pixels around that point; zoomed primary drag pans. Distinguish click/drag, provide a clear zoom-out/Fit interaction and matching zoom/grab cursors.
- Picker click patches, rectangle creation/move/corner resize retain priority over ordinary navigation and appropriate cursors.
- `z` should Fit. The reported stale fitted resolution needs reproduction/diagnosis; existing refinement is implemented, so do not assume downsampling is absent.
- The reported **“Settled preview failed: GPU processing: invalid area-average dimensions or GPU budget”** has a [confirmed default-limit cause and adapter-supported fix](completed/preview-stability.md). Strip processing is deferred unless actual memory/performance problems justify it. Other stale-resolution symptoms still need reproduction; do not skip refinement or weaken numerical gates.
- The user explicitly wants consultation for **each existing UI insertion that changes viewer size** before choosing a replacement. Gather the cases/screenshots first. Tab prominence and Output placement feedback also need a current-layout discussion; current pinned Output was previously approved.
- Developed-source codes retain their original linear range and normalize by 65535; development deliberately has no auto-brightness. An **Inspection** popup in the existing viewer controls now supplies zero-default EV adjustment, using the standard −3…3 slider/−16…16 numeric range/reset. The gain scales the developed-source AP1 preview before unchanged ACES. Current source bypass uses native camera→AP1 (and optional original-source flat-field gains), not the superseded AP0 inversion chain. It is transient and never changes RAW development, calibration/fitting, photo/roll edits, project state or exports.
- **DSC04524** is the reported source with inaccurately dark reds; the exact historical grade is unknown. AP0 inversion has since been replaced, which neither reproduces nor proves resolution of that complaint. Reproduce any remaining symptom with a current grade and trace corrected native-camera RGB → native-camera inversion → camera→AP1 → photographic edits → rendered target. Preserve the [accepted native-camera decision](../decisions/native-camera-inversion.md); do not treat a selectable basis as an assumed remedy.
- Toe lift produces rather cold shadows, at least with **Vision3 500T**. The user suspects that the tungsten stock's greater blue sensitivity could leave only blue when other channels fall below a threshold; this is an unverified hypothesis, not an established cause. Capture the RAW, calibration, response and Roll/Photo settings, then trace near-base signed working channels through rendering before proposing a remedy. Do not silently warm shadows, add a pedestal/clamp or reinterpret the accepted response.
- Give the remaining UI a focused UX-polish pass: review unnecessary descriptions/redundant scope wording, hierarchy and action placement across calibration, editing and navigation. Keep essential decision information, actionable errors and concise state indicators visible; move secondary explanations to tooltips. Controls and copy already cleaned up are not a new redesign requirement.

## Triaged interaction follow-up

These requests need focused reproduction or design before implementation. The cursor/New-tab observations below are based on inspection of `src/editor_navigation.rs`, `src/editor_files.rs`, `src/editor_thumbnails.rs`, `src/editor.rs` and `src/editor_controls.rs`, not a runtime acceptance exercise.

### Focused fixes and checks

- **Hand cursor before dragging:** confirmed behavior mismatch. When zoomed primary drag can pan, hover currently shows ZoomOut; Grab appears only with Space and Grabbing during drag. Show the hand whenever the available drag would pan, preserving picker priority, disabled-state behavior and click-to-Fit. Extend the navigation cursor tests.
- **Roll tab for new projects:** startup selects Roll, but successful File → New retains the previous tab because prepared-document installation does not reset `tab`. Verify New from Photo selects Roll; do not reset it on ordinary imports or decide Open behavior implicitly.
- **Save confirmation in the corner:** the dirty-project prompt is an egui Window with no explicit placement. Treat this as a placement/UX question, not a save-state defect. Propose a centered, owner-relative dialog and discuss positioning/modality; preserve Save/Discard/Cancel and failure behavior.

### Camera roll workflow

- **Drag RAWs into the roll:** missing; current import uses File → Add RAWs. Add a roll drop target using the existing staged import/identity validation rather than a second loading path. Settle supported-file feedback, busy-state drops, duplicate handling and all-or-nothing failure behavior before implementation; wider input-format support is not implied.
- **Selection and removal:** single active-photo selection already exists, including keyboard navigation. The request may mean multi-selection or a separate selection for removal; discuss that before choosing interactions. Define removing the active/last photo, next selection, dirty/save behavior, in-flight work and whether reference-only records are retained. Removal must be project-only and never delete RAW files or silently change RGB calibration. Undo integration remains separately scoped.

### Layout and UX decisions

- **Bottom preview toolbar:** requested for Fit/100%/zoom and presentation-only controls, including developed-source preview and inspection EV. Discuss grouping, placement relative to the resizable filmstrip and discoverability; consider icons with tooltips/accessibility names rather than assuming icon-only controls. Consult before any viewer-size change; no relocation is implemented by this triage.
- **Durable UX guidelines:** discuss first, then record agreed user-facing rules alongside the [editor control contract](../contracts/editor-controls.md), keeping code-style conventions in `AGENTS.md`. Candidate topics are visible information versus tooltips, action hierarchy, icons, dialog placement, progress feedback and viewer-space stability. Existing requirements remain in force; these candidates are not yet accepted guidelines.
- **Wheel asymmetry/explanation:** the six displayed hue sectors are equally spaced, while WB coordinates use the local AP1 neutral response to Bradford log-cone gains and legal warmth/tint limits. Gamma balance instead controls native-camera log gamma. Mapping and input/output round trips are documented in the [wheel contract](../contracts/editor-controls.md#resolve-style-wheel-orientation-and-mapping) and current tooltips. Explain this distinction to the user and identify which wheel/gesture looks asymmetric before declaring a bug or changing the adapter.

Thumbnail defects/performance are tracked in [visual feedback](visual-feedback.md#triaged-thumbnail-follow-up). Shared progress/spinners, common photographic controls, B&W handling and code readability are tracked in [completion work](completion.md).

## Recorded support/architecture questions

- “Do we need the CPU path? Is it just for export? Why?” Explain the boundary rather than silently removing it: CPU LibRaw development, full-resolution f64 measurements/fitting/domain validation, reference tests and full-source export; interactive processing/presentation remains GPU-only. [Architecture](../architecture.md#ownership-and-execution).
- “What files can it open?” Clarify current exposed input support: ARW RAW import/reference selection and schema-4 native-camera JSON projects (default `.antitone.json`). Sony ILCE-1M2 lossless-compressed ARW is the verified camera input; LibRaw's wider decoding capability is not an application-wide support claim. JPEG/TIFF/EXR developed-image import is not implemented. [Product](../contracts/product.md), [project state](../contracts/project-state.md).

## Remaining steps and acceptance

1. Select a concrete interaction fix or UX-design question with the user; the triaged hand cursor/New-tab cases are code-confirmed, while dialog/toolbar/roll-workflow choices need agreement. Do not repeat the implemented navigation/inspection/thumbnail slices.
2. Reproduce remaining source/navigation/refinement/layout and colour symptoms against current model-4/composed views. Separate fixed issues from outstanding ones, consult on viewer-space changes and distinguish the Vision3 observation from its proposed cause. Processing-policy changes require separate agreement.
3. Implement only the selected slice with event/geometry/state tests. Preserve bypass/calibration/fitting/export independence, physical-pixel scale/source anchors, picker/crop priority and invalid/cancelled state. Actual trackpad feel and popup inspection remain separate checks, not inferred from headless events.
4. Diagnose remaining reproduced stale-resolution failures at their source. Verify Fit/native/resize/rapid-switch behavior, valid dimensions/adapter limits and stale-result rejection without perpetual idle redraw or weakened numerical gates.

Likely files: `src/{editor,viewport,viewer,refinement,resample,editor_tests,editor_smoke}.rs`; `src/platform/host.rs` only for a demonstrated event translation issue. Numeric/processing policy changes require separate agreement and regression gates.

## Implementation status

Navigation/source inspection are implemented in `src/editor_navigation.rs`, `src/editor.rs` and `ProcessingParameters::source_inspection`. Ordinary scroll pans both axes, Ctrl/Cmd-wheel/native pinch use egui's zoom factor once, multitouch prefers its own center/translation rather than synthetic duplicates. Fit click selects physical 100% at the pointer; an explicit-zoom click returns to Fit; zoomed primary/middle/Space drag pans. `z` ignores text focus; picker primary interactions win and use matching resize/move/crosshair cursors. Picker drag state clears between gestures. No host/event translation or new toolbar row was needed.

Event tests cover wheel/pinch/touch, click/drag, picker/cursor priority and text-focus handling. Source-inspection checks cover zero-default state, post-matrix-only effects, range rejection, CPU/GPU agreement and unchanged persistent settings/pick commands. [Archived interaction checkpoints](../research/history/implementation/inspection-and-interaction.txt) preserve the original tests and real-source smoke results. Physical trackpad feel, popup appearance and universal device/monitor acceptance remain unverified; the navigation checkpoint did not rerun full export integration.

Thumbnails use the existing resizable bottom strip (52…240 points, also window-limited), with current cache/pressure behavior in the [performance closure](completed/rendering-performance.md). Remaining layout/colour symptoms, physical interaction and photographic scope acceptance are separate from those implemented features.

Next action: select one remaining interaction/UX slice and agree its scope before implementation. Camera/geometry photographic checks and dense guides are tracked in [completion work](completion.md#geometry), not reopened navigation work.

Evidence/context: remaining symptoms were consolidated from `docs/feedback.md`; it is now an inbox, not the persistent requirement record. See [historical approved pass](../research/history/feedback-plan.md), [architecture](../architecture.md) and [control contract](../contracts/editor-controls.md).
