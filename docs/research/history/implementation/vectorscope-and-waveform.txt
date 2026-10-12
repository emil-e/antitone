# Responsive vectorscope and waveform — completed

Status: implementation closed and delivered locally to main after rebasing onto `bbc9afd` (native-camera model/schema 4 and accepted geometry). The user requested wrap-up under the updated repository procedure. No push. [Roadmap](../../../PLAN.md) owns priorities; [remaining scope acceptance/full-source work](../completion.md#scopes) is separate.

## Goal, approval and scope

Provide responsive feedback for the rendered positive photograph without changing processing, calibration, picks or exports. The user agreed selected-output-native encoded sRGB/P3, whole-frame approximate proxy coverage, one collapsible switchable slot above Roll/Photo controls, independent vector amplification and bounded GPU scheduling. Subsequent feedback approved width-responsive sizing/divider, retained coherent grading traces, distinct trace/guide colours, vectorscope default, brighter zoomed vector traces and persistent colour/approximate Skin directions. The last refinement dims only the Skin line, not its label or waveform.

No histogram, automatic correction, full-source analysis, extra RAW/wide retained image, CPU interactive fallback, new colour transform or project state. Canonical formulas/approximation/lifecycle are in the [scope contract](../../contracts/scopes.md); rationale in the [accepted decision](../../decisions/output-native-scopes.md).

## Completed implementation and integration

- `scopes`, `gpu_scopes`, two shaders and `editor_scopes` provide target-derived scalar conventions, GPU bins/display, transient UI and positive-only suspension. RGB/luma reuse four planes; vector amplification re-bins existing rendered samples. One reduction/16-byte diagnostic readback is outstanding. Compatible grading traces stay Updating; incompatible source/revision/composition/target/projection/scale traces cannot paint.
- `GpuImage::prepare` shares exact parameter-keyed native-camera processing/ACES with the viewer, whichever callback prepares first. Original-source flat-field coordinates and model-4 camera identity remain unchanged; regional settled images never supply scopes.
- `Editor::scope_input` declares the whole accepted export composition through `Composition`/`FrameRaster`; canonical affine mapping samples the existing whole-source proxy in final-frame order. Crop/straightening is not its source bounding rectangle. Sampling pitch remains approximate; this is not full-source coverage-weighted export reconstruction. Accepted geometry joins scope identity; crop drafts suspend/invalidate until Apply/Cancel.
- The existing short-window scope cap now reserves main's pinned Scan orientation/Composition panel as well as grading space. Output and viewer dimensions remain unchanged; pending numeric Save and scroll reachability regressions pass.
- Contracts, architecture, roadmap, decision/indexes and mixed thumbnail plan reflect the delivered feature. Deferred work/acceptance lives in completion work, not an unfinished implementation checklist.

## Verification and limits

Fresh integrated snapshot: **155 tests passed** (140 library, 8 scope GPU, 7 viewer); two unrelated/manual tests remain ignored. Strict editor-feature all-target Clippy, formatting, release editor build, diff whitespace and local documentation links passed. Reproduction/environment and selected results: [integration evidence](../../research/evidence/scopes-integration.txt). Tests cover both targets, nonzero source origins, turns/flips/exact and angled crops, independently mapped frame-centre bins/counts, native-camera parameter propagation, geometry invalidation/reuse, crop cancellation/suspension, retained traces and combined layout/Save behavior. Existing native-camera/flat-field/rotated-reconstruction viewer gates pass without weakened numerical tolerances.

Historical pre-integration measurements and onscreen checks: [initial checkpoint](../../research/evidence/scopes-checkpoint.txt), [layout/flicker follow-up](../../research/evidence/scopes-layout-and-flicker.txt). Those are snapshots, not fresh integrated photographic acceptance. Prior guide capture attempts were occluded and produced no captures; no visible-frame/occlusion gate was bypassed. Fresh composed-photo/guide appearance, physical-input feel, full-source statistics, real-photo export reopen, other devices/macOS 14 and physical monitors remain unclaimed.

## Exact next action

No further implementation is selected. When requested, agree an unoccluded real-photo scope acceptance exercise through [completion work](../completion.md#scopes). Full-source analysis requires separate approval; do not resume the historical geometry-rebase or deferred implementation steps.
