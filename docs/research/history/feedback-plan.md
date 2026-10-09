# Feedback priorities — historical approved amendment

Archived to preserve original scope and approval history, not an active task queue. Dmin diagnosis and film response are now complete. Current order/status: [roadmap](../../../PLAN.md); remaining packages: [plans](../../plans/README.md). Historical control group ordering, test counts and pending-diagnosis statements below are superseded.

Status: approved; after the ProPhoto and SDR Rec.2020 gamut-boundary failures, the user explicitly approved **sRGB + P3-D65**. [That output update](../p3-output-update.md) is implemented and verified without changing ACES mathematics. Black clipping/dark negative diagnosis and the remaining numbered work are pending. Based on [feedback.md](../../feedback.md). This changes the next implementation priority, not the numerical/export correctness requirements. The previous [execution plan](execution-plan.md) and [project-state foundation](../../../src/project.rs) remain recorded.

## Priority and scope

First complete a focused rendering/interaction feedback pass (steps 1–4). Then resume project save/recovery (step 5), using the simpler reference model below. Larger completion features (steps 6–7) remain explicitly scheduled rather than being silently deferred forever. Reassess that ordering after the feedback pass; do not build all the larger features before making the app saveable by default.

The user's completion list supersedes the original blanket deferral of scopes, transforms and selectable inversion bases as product goals. Approval of the immediate feedback pass is not an instruction to implement every later feature in one change set.

## Plan

### 1. Diagnose color and tonal behavior before changing processing

- Original sRGB/ProPhoto diagnosis is recorded in `docs/research/prophoto-gamut-diagnosis.md`; the failed SDR Rec.2020 candidate and explicitly approved P3 replacement are recorded separately. Confirm the user's current-image appearance using the same source, calibration, edits, view and settled-preview state. Compare GPU and CPU results in a common adapted XYZ/linear display basis, not the two targets' raw RGB codes. Check neutral ramps, near-black ramps, saturated colors and representative camera pixels; use the existing ColorSync/Core Image references where useful. No Lightroom/Resolve automation.
- The original ProPhoto/D50 and sRGB/D65 targets exposed a gamut-boundary failure. The approved current pair, sRGB/P3, has different ACES limiting gamuts but the same D65 white; it is not merely two ICC encodings of one rendered image. Some differences are expected; equality of every pixel would be the wrong acceptance test. Diagnose large common-gamut/neutral/tonal differences and distinguish intended target behavior from errors. Do not hide narrower rendering in a wider container or replace ACES mathematics.
- Trace apparent black clipping through RAW development, scan/base ordering, signed inversion output, AP0→AP1 conversion, the ACES input clamp, ACES rendering, target limits and presentation. The existing signed working/EXR contract does not mean the rendered image has no black limiting. Add focused near-base/near-black tests and temporary developer diagnostics, not permanent UI prose.
- Check why the developed negative is dark. If its normalization is correct but inspection is inconvenient, add an explicit negative-view brightness setting that affects presentation only. Picker measurements, inversion parameters and exports must remain unchanged. Do not brighten RAW development silently.
- **Accept:** document which differences are expected and which are defects; fix verified defects at their source. No unexplained black-level fudge factor, hidden exposure or approximate output transform. A genuine change to inversion/rendering policy is a separate processing decision with regressions/versioning.
- **Files:** `src/scan.rs`, `src/processing.rs`, `src/gpu.rs`, `src/shaders/processing.wgsl`, `src/rendering.rs`, `src/viewer.rs`, `native/rendering_transform.h`, `tests/processing.rs`, `tests/viewer.rs`, relevant probes. Regenerate pinned OCIO assets only if a justified transform-definition change requires it.

### 2. Simplify references and build consistent editing controls

**2a. Reference semantics and terminology — implemented**

Editor and project references are now single RGB values. Domain identifiers, processing/GPU code, probe usage/output and tests use `dmin`/`dmax`, not merely renamed UI labels. Project schema 3 stores RGB arrays without provenance; photo identity/relink and roll-level compatibility remain. Current-photo re-picking, atomic rejection/cancellation, calibration without source records and 58 all-feature tests pass; strict Clippy and formatting pass. Real-window smoke verifies current-photo initialization, deliberate source switching and working-photo/view return: **11.40 s / 478 submitted frames** on M5 Pro (`.cache/probes/editor-dmin-dmax-smoke.txt`), not M1 validation. The following 2b cleanup is also implemented.

- Model each calibration reference as one RGB value. Remove stored source/path/rectangle/fingerprint relationships from calibration references, not just their visible labels. Sampling rectangles and source-return context are transient UI state.
- Starting a reference pick uses the currently selected image. Re-picking never automatically navigates to the previous reference source or reinstates its old rectangle. Explicitly choosing a different source remains possible, and finishing/cancelling can restore the working image when the picker deliberately changed it.
- Use **Dmax** instead of “leader”; use **Dmin (film base)** initially for the other reference. Explain the physical meaning in a tooltip, not a persistent paragraph. These labels do not turn the measured RGB values into density numbers.
- Keep reference units/inversion-basis interpretation and development/model compatibility at roll level. Retain photo-source identities/fingerprints for future relinking, but changing/removing a photo must not invalidate a standalone measured reference through an obsolete source association.
- **Accept:** re-pick from a different current photo without an unexpected jump; invalid/cancelled picks change no calibration or edits; references survive source removal; new project records contain no per-reference provenance. Existing project-foundation provenance tests/schema will be revised before save UI integration, not preserved as an accidental product requirement.

**2b. Structured, standardized controls and less UI clutter — implemented**

Shared `editor_controls.rs` sections/rows/calibration/color inputs now replace improvised controls. Roll/Photo tabs have only meaningful groups; Output remains visible below scrollable edits. Full-width sliders, numeric/unit/reset alignment, scoped actual-default resets, ±3-EV exposure interaction with ±16-EV entry, Shift-fine control, and RGB wheels with the user's Resolve orientation (blue 3 o'clock/yellow 9) are implemented. Artwork/input/markers use one orientation transform; gamma and Bradford WB keep distinct display adapters and unchanged processing. A reproduced egui disabled-focus-loss commit is prevented by read-only row snapshots. [Control contract](../../contracts/editor-controls.md) records exact mappings and behavior.

**Verification:** 70 all-feature tests, strict Clippy and formatting pass. Headless events cover focus/entry/reset/disabled/layout/state scopes. Fresh window-only screenshots of both tabs at 1100×800/800×600 confirm orientation, fitting and fixed Output; narrow edit lists scroll rather than overflow. Real-image layout smoke: **25.84 s / 1,031 submitted frames** on M5 Pro. The optional harness requests ordinary window focus for each layout; no occlusion bypass or Resolve app automation. Evidence: `.cache/probes/controls-{all-tests,clippy,roll-normal,photo-normal,roll-narrow,photo-narrow}.*`, `editor-controls-smoke.txt`. Trackpad interaction in step 3 remains next.

- Establish a three-level hierarchy: **section/container → parameter row → specialized input**. Use a small shared set of section headings, spacing, alignment, widths and state styles, not individually improvised widget layouts or a general-purpose UI framework.
- Group the roll-pre controls into Calibration (Dmin/Dmax), Inversion (gamma/balance; basis when implemented) and Exposure. Group photo controls into Exposure, White balance and Geometry when implemented. Keep output settings in a consistent separate group; avoid an empty Roll-post section/tab that suggests unavailable controls. Only show groups with meaningful actions.
- Standardize parameter rows: label above, full-width slider, right-aligned editable numeric value/unit, and one consistent reset affordance. Define precision/fine adjustment, keyboard focus, tooltip behavior and disabled/error states centrally. A section reset affects only that section's edits; it does not erase calibration or reset other scopes.
- Standardize two-coordinate color controls as a grouped component: color wheel, warmth/tint parameter rows, neutral-pick action and reset, all in predictable positions. Gamma balance and WB reuse the structure while retaining their distinct processing/mapping contracts. Dmin/Dmax rows likewise share a calibration-row structure with a concise availability state and pick action.
- Use the same state for wheel, sliders, numeric input and neutral fitting. Layout/hierarchy—not additional descriptions—should communicate what belongs together.
- Give every parameter a consistent reset affordance (a small reset button; optional double-click reset), including gamma's actual default of 0.65. Reset a two-coordinate color control to its real identity; remove the separate “Neutral” button. Do not reset unrelated settings.
- Narrow the normal exposure slider span (initial proposal ±3 EV, 0.05-EV interaction), with finer adjustment and numeric access to the existing validated ±16-EV range. A narrower slider must not clamp a legal saved/manual value merely by drawing it.
- Draw RGB-oriented color-direction wheels, with consistent primary/secondary directions for the future vectorscope, while retaining warmth/tint sliders. Specify/test the wheel↔coordinate mapping for gamma balance and photographic WB separately. RGB presentation is not permission to replace Bradford WB with RGB channel gains or introduce another hidden state.
- Remove redundant “shared/global” labels and implementation descriptions. Move necessary explanations into tooltips/help; keep actionable errors and concise approximate/refining status. Keep an honest unsaved-state indication until saving exists, but not a wall of disclaimers.
- **Accept:** consistent section boundaries, parameter-row alignment and action placement across tabs; wheel/slider/numeric/reset round-trips; independent roll/photo/section-reset scopes; out-of-slider-range values preserved; keyboard/focus and disabled/error-state checks; screenshots/layout check at normal and narrow window sizes. Review the whole calibration→edit→navigation flow for hierarchy and consistency after these changes.
- **Files:** `src/editor.rs`, a small reusable editor-controls module, `src/color.rs` only if a documented coordinate adapter is needed, `src/editor_tests.rs`, `src/project.rs`, `tests/project.rs`, processing documentation/tests.

### 3. Make image interaction natural on a Mac trackpad

- Treat two-finger scrolling as pan and pinch as pointer-anchored zoom; avoid applying one gesture twice or treating every vertical scroll as zoom. Retain an explicit modified-wheel zoom path for mouse users.
- Outside picker mode, click a fitted image to zoom to native-pixel view around that point; provide a clear way back to Fit (including a zoom-out click behavior). When zoomed, primary drag pans. Distinguish a click from a drag before changing zoom.
- Use zoom-in/zoom-out and grab/grabbing cursors as appropriate. Picker mode retains click patches, rectangle creation/move/corner resize, with matching selection cursors; ordinary zoom/pan must not accidentally sample or move a patch.
- **Accept:** actual Mac trackpad exercise plus event/geometry regressions for pan, pinch, click-vs-drag, pointer anchors, fit/100%, bounds and picker interaction. Measurement coordinates remain full resolution and export processing is unaffected.
- **Files:** `src/editor.rs`, `src/viewport.rs`, `src/platform/host.rs` only if event translation needs adjustment, `src/editor_tests.rs`, viewport tests and real-window smoke.

### 4. Add visual feedback: thumbnails and scopes

**4a. Filmstrip thumbnails**

- Replace filename-only buttons with bounded cached thumbnails and short labels, a clear selection indicator and missing/loading/error states. Decode/generate off the UI thread, prioritize the selected image, and reject stale work.
- Calibration thumbnails must remain usable before both references exist. Specify source/positive thumbnail behavior and processing-key invalidation; never retain a full-resolution RAW per thumbnail. Label/use an approximate thumbnail policy rather than passing it off as an authoritative settled preview.
- **Accept:** responsive navigation and shared-edit invalidation, independent photo edits, missing-source handling, bounded cache/memory and no redecode caused merely by repainting controls.

**4b. Vectorscope and waveform**

- Build GPU scope reductions from the processed/rendered image, before OS monitor conversion, in one explicitly documented common RGB/scopes basis rather than comparing sRGB and P3 channel codes directly. Start with an RGB/luma waveform and a conventional vectorscope graticule; define transfer, white, coefficients, scaling and out-of-range display together.
- Declare scope coverage/resolution. Do not accumulate a settled waveform only from the visible zoomed region and imply it represents the whole frame. During interaction an approximate whole-frame scope is acceptable if identified; full-source-derived validation remains separate from the viewer's regional refinement.
- Reuse processing/rendering where possible; do not read back full-resolution images per frame. Verify neutral placement, primary/secondary directions, ramps, bin counts and signed/HDR boundary handling against scalar fixtures. The color-wheel directions should be coherent with the chosen scope convention without promising a scene-independent color correction response.
- **Accept:** useful whole-image scopes without degrading interaction or violating the memory budget; a clear relationship between selected rendering target and scope values.
- **Files:** `src/editor.rs`, worker scheduling, `src/viewer.rs`, `src/refinement.rs`, thumbnail/cache and GPU scope modules/shaders, synthetic scope tests and performance probes.

### 5. Resume durable projects — retained, not forgotten

- Wire Save/Open to the existing versioned model after adapting it to RGB-only references. Preserve stable photo IDs/order, roll-fixed development metadata, roll/photo settings, output presets and photo-source fingerprints. Keep reference values independent of photo availability/content changes.
- Add atomic explicit saves, separate autosave/recovery that never silently overwrites the saved project, missing-photo relinking and explicit changed-source acceptance. Keep view/navigation state separate from processing state.
- Add undo/redo with one drag or pick per operation and the correct shared/photo/reset scope. Include any implemented scope/thumbnail-independent settings; derived images and caches never belong in the project.
- **Accept:** reopen reproduces settings/output; removed/missing photos preserve calibration; schema/model incompatibilities are explicit; failed save/recovery leaves the prior document intact.
- **Preserved follow-ups:** export progress/cancellation and batch snapshots, metadata propagation, aggregate memory/cache budgets, M1 Pro/16 GB validation and distributable native dependencies remain in the original execution plan.

### 6. Add roll/photo orientation, then crop/rotation

**6a. Discrete transforms**

- Add independent roll-level and photo-level horizontal/vertical flips plus photo-level quarter-turn rotation. Specify composition order and transformed dimensions. Apply identically to the viewer, full-resolution picker coordinates, thumbnails, scopes and exports; combine flips predictably rather than changing the stored source.
- **Accept:** asymmetric corner fixtures at every orientation/composition, exact picker mapping, correct TIFF/EXR dimensions and unchanged calibration RGB values.

**6b. Arbitrary rotation and crop**

- Build on the shared geometry transform; keep edits non-destructive. Specify source-coordinate crop storage, rotation pivot/order, resampling/filtering and edge/transparent-area policy. Export the selected geometry, not the uncropped source behind it.
- **Accept:** preview/export geometry agreement, valid transformed pick regions, scope coverage matching the exported frame, undo/reopen, no accidental write to RAW files.
- **Files:** viewport/geometry, editor/project state, full-resolution scan sampling, refinement/resampling, export, thumbnail/scope integrations and geometry fixtures. This is a separate larger feature, not UI polish.

### 7. Selectable inversion spaces and assisted roll analysis

**7a. Inversion bases**

- Implement a small declared set of linear inversion bases as an independent processing choice. Keep AP1 photography and AP0 EXR interchange separate. Reference RGB values have one declared calibration basis; convert them and each scan into the selected inversion basis before nonlinear inversion.
- Recompute the applicable floor/domain checks and test anchors, neutral fitting and CPU/GPU agreement in every supported basis. Reject an unusable basis/reference combination rather than silently flooring/clamping reference values. Specify what happens to existing gamma-balance settings/fits when the basis changes; do not silently reinterpret versioned calibration units.
- **Accept:** independently selectable inversion basis with explicit/versioned behavior, no change to the TIFF/profile or EXR interchange contract, deterministic reopen/export.

**7b. Whole-roll estimation**

- Start with a cancellable **Dmax estimate** when an exposed sample is unavailable. Measure full-source candidates using robust statistics, clipping/floor masks and outlier rejection, rather than choosing the darkest individual pixel/channel extrema. Use inexpensive sampling for candidate discovery and full-resolution confirmation; bound worker/memory costs across the roll.
- Report a proposed RGB value and uncertainty/coverage; require explicit acceptance. A roll without a sufficiently exposed region does not identify true film Dmax uniquely, so the tool must be able to say that the evidence is insufficient.
- Extend to suggested global/base gamma and gamma/WB balance only after the Dmax estimator is useful. These are model/scene-dependent estimates, not automatic physically known corrections. Compare against explicitly measured diagnostic references and synthetic rolls before exposing automatic application.
- **Files:** processing/basis definitions, calibration/neutral fitting, GPU kernels, project versioning, RAW sampling/statistics, cancellable workers, roll-analysis UI and independent synthetic/real-roll diagnostics.

## Validation and implementation boundaries

- The plan was approved after the planning-only turn. Execute incrementally, starting with diagnosis; material processing-policy changes still require the explicit decision described in step 1.
- Preserve the established-library/GPU policy, signed/HDR working values, full-resolution sampling, managed SDR presentation and exact pinned ACES/profile contracts. New findings can justify a discussion, not silent weakening.
- Use focused tests per step and real UI evidence where event/layout behavior matters. Existing 52 tests and prior smoke are a baseline, not proof that the reported visual issues are absent.
- Leave samples untouched; no commits/pushes, global installations or Lightroom/Resolve automation. Hardware-specific monitor/M1 checks remain explicitly pending.
