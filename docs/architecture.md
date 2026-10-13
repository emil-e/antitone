# Architecture

The implemented application is one Rust package with narrow native bridges. The numerical core works without the desktop stack; features add `project`, `raw`, `gpu`, `render`, `export`, `ui` and `editor`. [README](../README.md) defines the shared Cargo/native build path; [contracts](README.md#contracts) define detailed invariants.

## Modules

| Boundary | Implementation | Responsibility |
| --- | --- | --- |
| Scalar processing | `src/{processing,photographic,color,sampling}.rs` | f64 response/domain reference, transforms, compensated sampling and actual-response fitting |
| RAW/source | `src/{raw,scan}.rs`, `native/raw_bridge.{h,cpp}` | Owned LibRaw development, metadata, immutable sensor diagnostics and camera RGB16 source/proxies/tiles |
| Flat-field | `src/{flat_field,gpu_flat_field,editor_flat_field,project_flat_field}.rs` | Native-camera gain estimation/evaluation, transactional reference workflow and durable map identity |
| GPU processing | `src/gpu.rs`, owned shaders | Exact integer source, precise temporary arithmetic, AP1 results and floor flags |
| Rendering | `src/rendering.rs`, `native/{ocio_bridge.cpp,rendering_transform.h}`, `assets/ocio/` | OCIO CPU reference/export and pinned generated GPU algorithms/tables |
| Geometry | `src/{geometry,crop,coverage,frame_raster}.rs` | Source/frame/raster types, composition, exact pixel-cell coverage and affine adapters |
| Viewer/refinement | `src/{renderer,viewer,refinement,resample,viewport}.rs` | Shared GPU resources, approximate preview, tiled settled reconstruction and transient canvas layout |
| Scopes | `src/{scopes,gpu_scopes,editor_scopes}.rs` | Output-native bins, whole-composition proxy sampling and bounded scheduling |
| Desktop host | `src/platform/{host,macos}.rs` | winit/egui, owned wgpu surface, SDR layer tagging and repaint lifecycle |
| Editor | `src/editor*.rs` | Stable-ID document state, controls/pickers/crop, staged worker transitions, thumbnails and GPU preflight |
| Files | `src/{export,profiles,project}.rs` | Full-source TIFF/AP0 EXR, ICC/metadata, project compatibility and safe writes |

## Editor workspace

`editor_controls`/`editor_icons` share semantic actions, numerical rows and collapsible sections. Roll → Photo → Rendering tabs select one inspector scroll area; scopes have separate chrome above it. Rendering owns the shared preview/TIFF target. `editor_viewer` owns a fixed-height bottom bar and the common tinted icon/name/Cancel/Apply group. Canvas dimensions stay stable; only selection/frame artwork covers the image. Crop settings remain in Photo Composition and flat-field support in its existing Roll section. Tabs may hide settings without accepting/cancelling drafts. Numeric/popover and bar/inspector input ownership precede canvas gestures.

`editor_preferences` loads symbolic section expansion, scope visibility/height ratio and preferred filmstrip height independently of projects. The normal app loads once and the host's post-event-loop exit callback atomically saves changed preferences. Invalid/unreadable files remain unchanged; errors are nonfatal. Successful accepted New selects Roll/opens missing Calibration; cancelled/failed/stale preparation leaves UI state intact. [Control contract](contracts/editor-controls.md).

## Data and colour flow

```text
RAW Bayer samples
  → LibRaw black subtraction / fixed roll WB / AHD
  → developed camera RGB16 (four u16 slots; fourth ignored)
  → normalize / optional source-coordinate camera-RGB flat-field gains
  → native-camera calibration / model-4 inversion / roll gamma balance
  → camera-to-AP1/D60 / exposure / photographic Bradford WB
  → Photo ACEScct contrast + colour preservation / AP1 saturation + density mix
  → signed/HDR AP1
       ├→ AP1-to-AP0 → scene-linear EXR
       └→ pinned ACES 2, 100-nit sRGB or P3/D65
            ├→ target transfer / RGB16 quantization / ICC → TIFF
            ├→ native-target transfer → approximate whole-frame scopes
            └→ target-to-extended-sRGB / encoding → tagged SDR layer
                 → OS monitor conversion
```

Calibration/fitting/export use full-resolution source, never proxy/display pixels. The metadata camera→AP1 matrix includes explicit XYZ/Bradford D65→D60 conversion. Roll camera-model/matrix identity rejects mismatched source/reference bases. Dmin/Dmax persist RGB only; one-code floors and corrected bounds are separate from measurement context. Source bypass enters rendering as developed AP1 and ignores inversion/photographic edits; transient inspection EV affects only that view. [Processing contract](contracts/processing-model.md).

## Ownership and execution

- `RawFile` uniquely owns LibRaw; `Developed` borrows its camera image. `Scan` retains selected development, so edits/picks do not redecode. Decoding and optional whole-sensor statistics are separate; regional diagnostics read immutable original sensor data. Scoped OpenMP budgets follow foreground/background priority and pressure. [Native rationale](decisions/established-raw-development.md).
- The UI owns live document/view state. One backend thread serializes source loading, document work, full-resolution picks, export and refinement. Candidate documents/previews are prepared before replacement. Imports register headers without hashing RAWs or developing the whole roll. First development fixes document WB; later loads and Open use that explicit WB.
- Flat-field reference editing temporarily owns an uncorrected Scan while retaining the working photo. Acceptance prepares the corrected candidate before replacement. One shared roll CPU/GPU map reaches measurements, preview, refinement, thumbnails and export; reference-only records do not enter the filmstrip. Gains always use original source coordinates.
- UI, backend and thumbnails share one wgpu device/queue and lazily initialized, prewarmed `GpuRenderer`. Immutable processing/ACES/presentation/reconstruction resources are shared; source/working/output storage, revision and signatures stay image/job-owned. Viewer/scopes share whole-proxy preparation; refinement has independent tile/output images. Scope bins/readback identity remain editor-owned.
- Request IDs/generations reject stale results. Edits/picks validate before mutation. Failure is reported without numerical/rendering fallback. Refinement and thumbnail cancellation occurs between phases; LibRaw calls and submitted GPU work cannot be interrupted. Exports use loaded-source/settings snapshots; application export cancellation is not implemented.
- `CpuRenderer` owns an OCIO processor behind an exception-translating C ABI. CPU work supplies development, f64 measurement/validation, export and reference checks—not interactive image processing/presentation.

## Preview, resources and caches

A bounded nearest-centre camera RGB16 proxy provides approximate live preview. After 250 ms idle, settled refinement processes the visible composed region in `1024×256` tiles and averages linear rendered target RGB onto the physical display grid. Exact geometry uses restricted rectangular averaging; rotated framing uses exact convex source-cell weights. Bounding boxes limit iteration, never coverage. Nonlinear processing precedes averaging. [Geometry contract](contracts/geometry.md).

`editor_selected_preview` keeps switching display separate from authoritative `Ready`: a compatible retained 256 RGB16 memory proxy supplies independent selected GPU images after dispatching the normal RAW load. Mutable processing never aliases a filmstrip image. Current grade/source inspection, correction and accepted framing apply; request/source/development/file-revision/correction changes invalidate pending state. Memory misses keep the loading placeholder. Full-resolution operations wait for the normal Scan, and completion/failure/document work discards pending display. This temporary selected working set introduces no new cache tier or full-roll Scan retention.

Only one settled job/tile is outstanding; waits target that tile's submission rather than later UI work. `GpuImage::prepare` reuses exact parameter signatures; source/correction writes increment revision and invalidate preparation, while rejected writes do neither. Rendered-only images have no source/working storage. Adapter-reported storage/buffer limits are capacities, not reserved memory or an aggregate budget.

A separate one-request thumbnail worker leaves the selected Scan untouched. Visible missing cells are admitted after foreground preparation without waiting for settled refinement or grading idle; optional offscreen prefetch remains idle-only. Worker outcomes wake the UI. Native work keeps background thread priority, and foreground/reference/crop/document/critical-pressure guards remain in place. Compatible selected development seeds its uncorrected 256-pixel proxy with the file key captured before opening/development. Cold work checks camera/development compatibility and uses resolved/canonical paths plus cheap file stats for optional cache freshness. It reuses a checksummed disk proxy or develops one temporary source with document WB; changed stats quietly miss/reject stale results. No RAW contents are hashed, and cache revisions are not capture identity. Source/development errors differ from disposable-cache misses. CPU/GPU LRU retention follows byte budgets and OS pressure, protecting the selected proxy/visible images; idle prefetch uses spare budget and yields to foreground work. [Cache contract](contracts/output.md#responsiveness-and-optional-caches).

Scopes independently sample the whole accepted composition through canonical affine mapping, not regional refinement or viewport navigation. Their nearest-proxy data is approximate. Image/revision/parameters/composition/target/scale identity governs one outstanding reduction and asynchronous diagnostic readback. Source/reference/crop-draft views suspend analysis. [Scope contract](contracts/scopes.md).

## Spatial geometry

Shared mappings compose active source → roll turn/flips → photo turn/flips → source-anchored framing. Source development/calibration/correction coordinates are never reoriented. `viewport::CanvasLayout` maps the viewed domain to logical UI points; pointer selections inverse-map actual coverage to full-resolution source cells. Rotated measurements use compensated area-weighted means/fitting and matching sensor-site spans.

Crop editing is a cancellable draft, independent of accepted processing/output. Exact provisional/settled textures remain source-oriented with affine presentation; rotated settled textures are destination-frame ordered. Composition joins request signatures, so stale results cannot install after geometry edits. TIFF averages individually rendered pixels in linear target RGB; EXR averages signed/HDR scene-linear pixels before AP0 serialization. [Geometry](contracts/geometry.md), [controls](contracts/editor-controls.md).

## Presentation and exports

The host reapplies explicit extended-sRGB tagging after surface configuration, using `RGBA16Float` SDR content with EDR disabled. Image conversion/encoding occurs once; ordinary egui UI is encoded sRGB. Core Animation/ColorSync supplies the sole monitor conversion.

TIFF/EXR recompute from full-resolution f64 processing. TIFF uses OCIO, matching transfer/ICC and RGB16 LZW/prediction; EXR writes ZIP16 half/full AP0 with overflow rejection. Same-directory temporary files protect no-overwrite/confirmed replacement. Buffer caps and atomic installation are not an aggregate OOM or full power-loss guarantee. [Output contract](contracts/output.md).

## Persistence boundary and known gaps

`Document` owns authoritative `Project`, path and saved baseline. Accepted roll/photo/output/composition/flat-field state is durable and source-ID-owned. Picker/view/reference/crop drafts, GPU resources and worker IDs are transient; durable UI sizing/disclosure preferences live in their independent application file. Save updates the baseline only after write success; Save As rebases relative locators. Open validates runtime/GPU/source identity, prepares the first photo with saved WB, then commits. New/Open/Close share Save/Discard/Cancel protection. [Project-state contract](contracts/project-state.md).

Recovery/autosave, relink/changed-source UI and undo/redo are unwired. Missing/unloadable/development-incompatible ordered sources reject Open; nonordered unavailable records and calibration RGB remain preserved. Photographic, physical-input, broader-device/macOS-14 and monitor acceptance remain open. [Roadmap](../PLAN.md) owns priorities and unfinished feature scope.
