# Source inspection and interaction feedback

Scope basis: remaining approved feedback pass. The user selected navigation polish, then source inspection, then thumbnails. The [preview buffer-limit fix](completed/preview-stability.md) is complete; navigation, source-inspection controls and bounded thumbnails in the approved resizable filmstrip are implemented. [Roadmap](../../PLAN.md) owns order/status. Completed Dmin/toe and output work are not to be reopened by default.

## Goal and boundaries

Make developed-source inspection and Mac image navigation usable without changing calibration, export, response, RAW development or ACES. Preserve full-resolution picker coordinates, atomic rejection/cancellation and working-photo/view restoration.

## Decisions and open questions

- Normal two-finger scrolling should pan; pinch zoom remains pointer-anchored. Retain an explicit modified-wheel zoom path for mouse users without applying gestures twice.
- Outside picker mode, a fitted-image click should zoom to native pixels around that point; zoomed primary drag pans. Distinguish click/drag, provide a clear zoom-out/Fit interaction and matching zoom/grab cursors.
- Picker click patches, rectangle creation/move/corner resize retain priority over ordinary navigation and appropriate cursors.
- `z` should Fit. The reported stale fitted resolution needs reproduction/diagnosis; existing refinement is implemented, so do not assume downsampling is absent.
- The reported **“Settled preview failed: GPU processing: invalid area-average dimensions or GPU budget”** has a [confirmed default-limit cause and adapter-supported fix](completed/preview-stability.md). Strip processing is deferred unless actual memory/performance problems justify it. Other stale-resolution symptoms still need reproduction; do not skip refinement or weaken numerical gates.
- The user explicitly wants consultation for **each existing UI insertion that changes viewer size** before choosing a replacement. Gather the cases/screenshots first. Tab prominence and Output placement feedback also need a current-layout discussion; current pinned Output was previously approved.
- Developed-source codes retain their original linear range and normalize by 65535; development deliberately has no auto-brightness. An **Inspection** popup in the existing viewer controls now supplies zero-default EV adjustment, using the standard −3…3 slider/−16…16 numeric range/reset. The gain applies only to the source-preview AP0→AP1 post matrix, before unchanged ACES. It is transient and never changes RAW development, calibration/fitting, photo/roll edits, project state or exports.
- **DSC04524** is the reported source with inaccurately dark reds; the exact calibration, response and Roll/Photo settings are still needed. Capture a reproducible grade and trace developed camera RGB → AP0 inversion → AP1 working values → rendered target before attributing it to the inversion space. Compare with the [accepted colour-space boundaries](../decisions/colour-space-boundaries.md); selectable bases remain a separate design, not an assumed remedy.
- Toe lift produces rather cold shadows, at least with **Vision3 500T**. The user suspects that the tungsten stock's greater blue sensitivity could leave only blue when other channels fall below a threshold; this is an unverified hypothesis, not an established cause. Capture the RAW, calibration, response and Roll/Photo settings, then trace near-base signed working channels through rendering before proposing a remedy. Do not silently warm shadows, add a pedestal/clamp or reinterpret the accepted response.
- Give the remaining UI a focused UX-polish pass: review unnecessary descriptions/redundant scope wording, hierarchy and action placement across calibration, editing and navigation. Keep essential decision information, actionable errors and concise state indicators visible; move secondary explanations to tooltips. Controls and copy already cleaned up are not a new redesign requirement.

## Triaged interaction follow-up

These are recorded requests, not an approved implementation sequence. Current behavior was inspected in `src/editor_navigation.rs`, `src/editor_files.rs`, `src/editor_thumbnails.rs`, `src/editor.rs` and `src/editor_controls.rs`; no fresh runtime verification was performed for this triage.

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
- **Wheel asymmetry/explanation:** the six displayed hue sectors are equally spaced, while WB coordinates use the local AP1 neutral response to Bradford log-cone gains and legal warmth/tint limits. Gamma balance instead controls AP0 log gamma. Mapping and input/output round trips are documented in the [wheel contract](../contracts/editor-controls.md#resolve-style-wheel-orientation-and-mapping) and current tooltips. Explain this distinction to the user and identify which wheel/gesture looks asymmetric before declaring a bug or changing the adapter.

Thumbnail defects/performance are tracked in [visual feedback](visual-feedback.md#triaged-thumbnail-follow-up). Shared progress/spinners, common photographic controls, B&W handling and code readability are tracked in [completion work](completion.md).

## Recorded support/architecture questions

- “Do we need the CPU path? Is it just for export? Why?” Explain the boundary rather than silently removing it: CPU LibRaw development, full-resolution f64 measurements/fitting/domain validation, reference tests and full-source export; interactive processing/presentation remains GPU-only. [Architecture](../architecture.md#ownership-and-execution).
- “What files can it open?” Clarify current exposed input support: ARW RAW import/reference selection and schema-3 JSON projects (default `.antitone.json`). Sony ILCE-1M2 lossless-compressed ARW is the verified camera input; LibRaw's wider decoding capability is not an application-wide support claim. JPEG/TIFF/EXR developed-image import is not implemented. [Product](../contracts/product.md), [project state](../contracts/project-state.md).

## Current execution plan

1. [x] Implement unmodified scroll pan, independent anchored pinch/modified-wheel zoom, click-native/click-Fit, primary drag pan, `z` Fit and picker/navigation cursors without adding UI rows. Verify egui events, geometry, text-focus gating and picker priority.
2. [x] Add presentation-only source-inspection EV in an existing-row popup. Verify zero-default equivalence, GPU output, transient state and pick/settings independence. No RAW/calibration/fitting/export change.
3. [x] Implement bounded thumbnails in the existing enlarged/resizable filmstrip with approved layout, stable-ID/development cache keys, foreground priority and stale-work rejection. [Implementation/checks](visual-feedback.md#progress--handoff). Scopes and larger completion packages still require their own design.

## Steps and acceptance

1. Reproduce remaining source/navigation/refinement/layout and colour symptoms against current code; separate already-fixed issues from outstanding ones, discuss viewer-intrusion cases and review remaining UX clutter. Distinguish the Vision3 cold-shadow observation from its proposed cause; any processing-policy change still requires separate agreement.
2. Specify presentation-only source inspection state and implement only the agreed behavior. Verify bypass, calibration/fitting/export independence and invalid/cancelled picks.
3. Implement gesture/click/cursor/Fit behavior with event/geometry tests and actual Mac trackpad exercise. Verify anchors, bounds, physical-pixel 100%, click-vs-drag and picker isolation.
4. Diagnose any remaining reproduced stale-resolution failures at their source. Verify fit/native/resize/rapid-switch behavior, valid dimensions/adapter limits and stale-result rejection, without perpetual idle redraw or weakened numerical checks.

Likely files: `src/{editor,viewport,viewer,refinement,resample,editor_tests,editor_smoke}.rs`; `src/platform/host.rs` only for a demonstrated event translation issue. Numeric/processing policy changes require separate agreement and regression gates.

## Progress / handoff

Navigation/source inspection are implemented in `src/editor_navigation.rs`, `src/editor.rs` and `ProcessingParameters::source_inspection`. Ordinary scroll pans both axes, Ctrl/Cmd-wheel/native pinch use egui's zoom factor once, multitouch prefers its own center/translation rather than synthetic duplicates. Fit click selects physical 100% at the pointer; an explicit-zoom click returns to Fit; zoomed primary/middle/Space drag pans. `z` ignores text focus; picker primary interactions win and use matching resize/move/crosshair cursors. Picker drag state clears between gestures. No host/event translation or new toolbar row was needed.

Verification snapshot: **73 focused tests passed** (59 library, 7 project, 3 film-project, 4 viewer), strict all-target Clippy, formatting and release editor build. Seven new egui-event navigation tests cover wheel/pinch/touch, click/drag, cursor/picker priority and `z` text focus. Source-inspection tests verify zero-default parameter bytes, post-matrix-only changes, range rejection, CPU/GPU working agreement at unchanged tolerances, zero diagnostic flags and unchanged persisted settings/pick commands. The real-source Save/Open smoke, now exercising +2 EV source inspection, passes **722 onscreen frames** on the unlocked M5 Pro; Fit/native/reopened refinement and exact saved state/WB/output remain valid. Actual physical trackpad feel and popup visual inspection were not externally driven; no universal-device/monitor acceptance claim. No fresh full export integration suite.

Bounded thumbnails are now implemented in the existing bottom strip; the user approved enlargement and requested adjustable size. Drag its top edge to resize (52…240 points, also window-limited); image display follows height, while source/GPU caches stay bounded. Remaining layout/colour investigations, physical interaction exercise and scopes remain open; no further slice is automatically authorized.

Evidence/context: remaining symptoms were consolidated from `docs/feedback.md`; it is now an inbox, not the persistent requirement record. See [historical approved pass](../research/history/feedback-plan.md), [architecture](../architecture.md) and [control contract](../contracts/editor-controls.md).
