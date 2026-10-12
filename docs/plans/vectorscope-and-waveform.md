# Whole-frame vectorscope and waveform

Status: responsive proxy slice implemented and verified in `.worktrees/scopes` (`plan/vectorscope-waveform`, base `c0b664a`). User visual/use feedback remains; full-source analysis is deferred and not authorized. [Roadmap](../../PLAN.md) owns priority/status.

## Goal and boundaries

Provide useful vectorscope and RGB/luma waveform feedback for the rendered positive photograph without delaying grading, misrepresenting a zoomed region as the whole frame, or changing processing/export behavior.

Include GPU reductions, explicit colour/coverage conventions, bounded resources, transient controls, stale-result rejection and focused numerical/UI/performance verification. Exclude histogram, automatic correction, scope-driven picks, HDR display, broadcast compliance, new rendering transforms, project-schema changes, thumbnail fixes and unrelated layout redesign. Preserve pinned LibRaw/ACES, signed/HDR AP1, AP0 EXR, model-2 compatibility and the 8-byte/pixel RGB16 upload representation. No commits/pushes/global installs or geometry merge are authorized.

## Agreed design

The planning discussion established:

- **Purpose/basis:** judge the rendered photograph using display-referred selected-output-native encoded RGB before monitor conversion. The initial common extended-sRGB proposal was replaced; cross-target coordinate comparability is not the priority.
- **Availability:** positive-grade only; suspend outside that view and during temporary reference/busy operations. Inspection EV never affects scopes or makes an old positive trace current.
- **Placement:** one switchable slot above Roll/Photo controls, where Lightroom puts its histogram. Keep viewer dimensions and pinned Output; accept reduced grading scroll space rather than a floating window, enlarged filmstrip or extra viewer row.
- **Responsiveness:** reuse the existing whole-frame GPU proxy during edits. Full-source work is not part of the interactive critical path.
- **Amplification:** magnify small vectorscope traces around neutral, independently of image processing, colour basis or trace brightness. Show scale/reset, scale graticule/markers consistently, retain detail and account for offscreen samples. Resolve's reported issue was trace occupancy, not widget size.
- **Sharing:** optional. Share sample preparation/traversal/lifecycle only if simpler; waveform/vector projections, graticules and per-column/whole-image normalization remain distinct. No generic coordinate-system framework is required.

The authorized initial defaults are implemented: collapsible same-height slot, RGB/luma waveform, 1×/2×/4×/8× vector scale/reset, clearly labelled whole-frame approximation, no automatic full-source analysis. Detailed current formulas, reference amplitudes, density mapping, spatial/lifecycle/resource guarantees are canonical in the [scope contract](../contracts/scopes.md); rationale is in the [accepted decision](../decisions/output-native-scopes.md).

## Implemented boundaries

| Boundary | Implementation / constraint |
| --- | --- |
| Existing source/proxy | `editor::prepare_source` already constructs both target `Ready.images` from a maximum-dimension-1024 nearest-centre whole-active-area RGB16 proxy. Scopes reuse the selected one; no new RAW development/source cache. |
| Processing/rendering | `GpuImage::prepare` in `src/viewer.rs` shares parameter-keyed processing/ACES with `ImagePaint` and `ScopePaint`; either callback can prepare first. Source/correction writes increment image revision. |
| Scope specification | `src/scopes.rs` defines target-derived basis, projections and independent scalar bin oracle. No change to production processing or generated ACES artifacts. |
| Reduction/display | `src/gpu_scopes.rs`, `src/shaders/{scopes,scope_display}.wgsl` accumulate/draw GPU bins from a whole-frame linear-target raster, after target transfer. Waveform modes reuse the same four planes; vector scale re-bins cached rendering. |
| Scheduling/diagnostics | One reduction/16-byte async readback outstanding. Edits/hide/source transitions wait for retirement and reevaluate the latest UI state; stale diagnostics don't reinstall. Grading edits retain coherent same-source/basis bins labelled Updating; incompatible source/target/scale changes still clear the trace. Unchanged repaints do no compute or perpetual repaint. |
| Editor/layout | `src/editor_scopes.rs` owns transient controls, positive-only gating and the centralized `scope_input` boundary. Slot height follows sidebar width with a draggable divider/reset and a short-window cap; vector trace is square and waveform uses width. Both share panel footprint. Magnified vector graticule is clipped to its trace raster. |
| Viewer/refinement | Regional settled `ViewRequest` images are never scope inputs. Scope keys exclude viewer pan/zoom/resize/DPI. Existing refinement/backend/thumbnail workers are unchanged. |
| Persistence/export | No scope state in project schema, no pick/export use of proxies and no CPU viewer fallback. Save shortcut tests now scroll to the numeric edit because scopes consume control space. |

Scope buffers retain 1,049,616 bytes (shared bins/totals), plus one 16-byte staging buffer, small uniforms/bindings and unmeasured driver overhead. No extra RAW or wide retained source/working image. Fixed resources are not a whole-process budget claim. Simple atomics are sufficient on the measured machine; no speculative sharding/atlas/renderer redesign was added.

Relevant unchanged constraints: [output](../contracts/output.md), [editor controls](../contracts/editor-controls.md), [project state](../contracts/project-state.md), and accepted [output/presentation](../decisions/sdr-output-and-presentation.md), [colour-space boundaries](../decisions/colour-space-boundaries.md) and [RGB16 source](../decisions/rgb16-gpu-source.md) decisions. [Visual feedback](visual-feedback.md) remains the thumbnail/follow-up record.

## Concurrent rotate/crop integration

The user explicitly identified `.worktrees/flip-crop-rotate` (`plan/flip-crop-rotate`, inspected at `ace5628` with uncommitted geometry implementation). Merge order is unknown; one branch must rebase on the other before combined acceptance. It was inspected read-only; no copying/merge or edits there.

Its `docs/contracts/geometry.md` distinguishes original **source**, **oriented-source**, final composed **frame**, per-image **raster** and UI **canvas** coordinates. Pixel centres are `(i+0.5,j+0.5)`, active origins can be nonzero, and integer regions are half-open. Accepted order is active source → roll quarter-turn/flips → photo quarter-turn/flips → crop/straightening. Scopes must cover the **exported composition**, with waveform x in final frame coordinates, never canvas/viewport x or a crop's source bounding box. Flat-field evaluation remains at original source centres before colour processing.

This branch has identity geometry: the whole-active-area rendered proxy is also the whole-frame raster. `Editor::scope_input` is the integration boundary. At rebase, adapt it using canonical `Composition`/raster/coverage mappings; include accepted composition in identity, exclude crop drafts or explicitly suspend during editing, and compare oriented/cropped scope coverage with exports. Arbitrary-angle coverage must not become its source bounding rectangle. No second geometry system or independent crop/orientation shader algebra was introduced. Expected conflict points: `editor.rs`, `viewer.rs`, `lib.rs`, smoke/tests and architecture/control docs.

## Initial verification checkpoint

Snapshot environment: Apple M5 Pro, macOS 26.6.2, checkout-local Rust 1.99.0, pinned LibRaw 0.22.0 / OCIO 2.6.0, Metal/wgpu 30.0.1 and Apple Clang. Commands are in [the runbook](../build-and-probes.md#responsive-image-scopes); selected measurements/reproduction are retained in [checkpoint evidence](../research/evidence/scopes-checkpoint.txt).

- **99 focused tests pass:** 89 library, 5 GPU scope and 5 viewer tests. The unrequested local-RAW flat-field test remains ignored; the separate scope latency probe was run explicitly and passed. Formatting, editor-feature all-target strict Clippy and release editor build pass. `git diff --check` and 149 local Markdown links/anchors across changed/new docs pass.
- **Math/bins:** target-specific weights, neutral centre, primary/secondary directions, B/Y cardinal positions, encoded ramps/endpoints, invalid/out-of-range inputs, odd/narrow raster columns and exact count reconciliation. Numerically sensitive vector bin boundaries permit at most one adjacent bin per axis; production colour tolerances remain unchanged.
- **Amplification/cache:** detail-preserving re-binning, offscreen counts, unchanged image/processing on zoom, RGB/luma reuse, image/source-revision identity, stale diagnostics, bounded rapid requests/hide/show and no repeated failing reduction. Actual density paint is checked by a GPU output pixel readback.
- **Editor/processing regression:** whole proxy under viewer zoom, unchanged viewer canvas when scopes show/hide, high-DPI callback bounds, fixed switchable footprint and amplified graticule clipping. Source/busy/reference suspension, transient project state, numeric Save commit and grading-control reachability with pinned Output pass. Existing Original/Film, correction and both-target viewer gates pass.
- **Responsiveness probe:** 1024×1024 flat/varied proxies, six encode/submit/wait iterations per plot. Warmed reduction samples are approximately **0.57…1.36 ms**, with initial RGB samples **2.85…4.46 ms**. This excludes upload/ACES/diagnostic mapping and is not end-to-end control latency or other-device acceptance.
- **Onscreen integration:** final scopes+four-layout-hold smoke passes **1,496 submitted frames / 30.15 s**. Scope check records **699,392 samples / 8 reductions / 0 invalid**. Both targets, RGB/luma/vector scale, source bypass, Fit/native refinement and reference return are exercised without weakening the visible-frame gate. Four normal/narrow window-only captures pass and were inspected for placement, traces, magnified clipping/offscreen accounting and pinned Output. Diagnostic RAW/calibration/settings are not the user's saved grade; captures remain private/ignored under `.cache/probes/scopes-{roll,photo}-{normal,narrow}.png`.
- **Independent review:** performed once. Fixed outstanding-job coalescing across rapid edits/hide/source changes and avoided redundant RGB/luma reduction. A clipping defect noticed in window capture was fixed at the vector graticule boundary and regression-tested before the final capture/smoke.

No full-source scopes, full export rerun, physical-control latency measurement, geometry integration, other-device/macOS-14 or physical-monitor acceptance is claimed. Resource figures exclude driver overhead and aren't an aggregate application memory policy. User use/visual feedback remains open.

Local ignored setup reuses `.cache/{rustup,deps}` and `.cache/samples` read-only inputs, with worktree-local `CARGO_HOME="$PWD/.cache/cargo"` and separate `target` products. No downloads/bootstrap/global installs, RAW edits, commits or pushes.

## User-feedback follow-up

The user reported small, non-width-scaling scopes, control-adjustment flicker and poor plot/graticule colour separation, and approved the proposed divider. Implemented a larger width-based default with draggable height and double-click reset; the transient aspect preference continues to scale with width. Both plots retain the same footprint and the viewer/pinned Output remain unchanged. Short windows cap height; resizing/collapsing trades scope height for grading space.

The flicker was an explicit missing-group path while the previous reduction/readback was outstanding. Reuse only coherent bins for unchanged source/revision and target/projection/amplification during grading edits, labelled Updating. No second bin/image allocation, extra queued job or weaker stale-result guard. Bright vector/luma traces, blue-purple graticules and coloured RGB/CMY reference markers separate data from guides.

Verification: [follow-up evidence](../research/evidence/scopes-layout-and-flicker.txt). Divider drag/release/idle/reset and width scaling are tested at both DPIs, with physical-pixel rounding tolerance and settled egui DPI initialization. GPU output verifies retained grading traces, completion, the one-job bound and clearing for target/plot/scale/source/revision/suspension changes. Scope sizing remains transient; unchanged source/viewer tests pass. Normal/narrow captures check larger plots, guide contrast and pinned Output; physical-input feel remains user acceptance, not a headless-test claim.

A subsequent user request sets vectorscope as the initial plot and fixes dimness under zoom. Vector drawing now uses chroma-bin-area compensation rather than decreasing brightness as samples spread; the base intensity is also raised. This is presentation-only, with whole-valid-sample normalization (including offscreen samples), unchanged bins/colour math and no per-edit peak auto-scaling; [current formula](../contracts/scopes.md#colour-convention).

Latest snapshot on the same M5/Metal environment: **105 focused tests pass** (93 library, 7 GPU scopes, 5 viewer), with the two unrequested/manual tests still ignored; strict editor-feature all-target Clippy, formatting and release build pass. New default-mode test and GPU display-transfer fixtures cover both targets and all four gains, preserving intensity for equal physical density and ensuring offscreen counts don't renormalize brightness. The runbook's scopes/layout smoke passes **1,497 frames / 30.33 s**, **699,392 samples / 8 reductions / 0 invalid**; 2× normal and 4× narrow window-only captures show improved trace visibility. Logs/captures stay ignored under `.cache/probes/scopes-intensity-*`; no new physical-input/device acceptance or reduction-latency measurement is claimed.

The user subsequently approved always-visible colour directions and a conventional approximate skin-hue guide. RGB/CMY ticks/letters stay at target-derived rim angles at all gains; 100% code boxes still magnify and disappear when clipped or obscured by labels. Added an amber dashed Skin ray using the declared positive-I/zero-Q native-code heuristic; [definition, sources and limits](../contracts/scopes.md#skin-hue-reference). No measurement/basis, processing, histogram, scheduling or project change; waveform overlays are unchanged.

Latest focused snapshot: **108 tests pass** (96 library, 7 GPU scopes, 5 viewer), with manual/unrelated ignores unchanged; strict editor-feature all-target Clippy and release build pass. Independent Q=0 fixtures verify positive-I direction in both native bases, neutral-offset/magnitude invariance and red/yellow ordering. UI shape checks cover all four gains and minimum/normal trace sizes: all seven annotations remain visible/clipped, and skin dashes/label positions stay fixed. Waveform retains only its level labels. Two subsequent real-source onscreen attempts became occluded/focus-lost before the scope/layout checkpoint: the first hit the 60-second tool timeout; a normal retry was terminated after 55 seconds with no captures. No occlusion bypass or visible-frame-gate change; fresh onscreen appearance is unverified. Logs stay in `.cache/probes/scopes-directions-smoke{,-occluded}.txt`. Physical-input/device and combined geometry acceptance remain separate.

## Full-source follow-up (deferred, requires separate approval)

If selected later:

1. Agree automatic-versus-requested analysis from actual cost/use need, never making it an interactive prerequisite.
2. Process the accepted exported-frame coverage with bounded 1024×256 source tiles and canonical geometry mappings. Exclude padding, preserve original-source correction coordinates and reconcile valid pixel/coverage counts with checked arithmetic.
3. Accumulate after per-pixel processing/rendering, before viewer area averaging. Do not retain a full rendered/working source or re-develop another RAW; rendered code statistics must not be confused with EXR diagnostics.
4. Keep the selected `Scan` on the existing backend with narrow incremental scheduling between tiles: pause/yield to foreground/refinement, cancel changed content, reject old revisions and prevent backlog. Measure thumbnail fairness and failures, not only throughput.
5. Full-source bins cannot gain detail under amplification without reprocessing tiles. Maintain responsive proxy re-binning with an explicit Approximate label; never relabel coarse magnification or proxy zoom as fresh Full source.
6. Verify edge tiles/nonzero origins and one-shot/tiled bins, coverage matching exports and geometry, full/proxy labels, cancellation, latency, resource peak and diagnostic/reconstruction tolerances. No weakened numerical gates or minimum hardware assumption.

**Exact next action:** user evaluation of the built responsive slice. If geometry merges first, rebase this work onto it and adapt/test `scope_input` against the exported composition; if scopes merge first, geometry must integrate that boundary on its rebase. Full-source work remains unselected. Keep this plan active until user acceptance or a selected follow-up, rather than resuming deferred steps automatically.
