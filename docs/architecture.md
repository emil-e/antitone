# Architecture overview

This describes the implemented application, not the delivery plan. Current work/order lives in [PLAN.md](../PLAN.md); detailed invariants and rationale are indexed in [the documentation map](README.md).

## System boundaries

Antitone is a single Rust package with narrow native bridges. The numerical core can be tested without the desktop stack. Cargo features add `project`, `raw`, `gpu`, `render`, `export`, `ui` and `editor`; `editor` assembles the desktop application. Versions are pinned in `Cargo.toml`/`Cargo.lock`.

| Boundary | Implementation | Responsibility |
| --- | --- | --- |
| Scalar processing | `src/{processing,color,sampling}.rs` | f64 reference, response/domain validation, colour transforms, compensated means, actual-response neutral fitting |
| RAW development | `src/raw.rs`, `native/raw_bridge.{h,cpp}` | Owned LibRaw handle, controlled development, metadata/matrix and sensor diagnostics |
| Full-resolution source | `src/scan.rs` | Cached developed source, calibrated sampling, RGB16 preview/tile extraction |
| GPU processing | `src/gpu.rs`, `src/shaders/{camera_conversion,film_response,processing}.wgsl` | Exact integer source representation, accurate temporary conversion/response, AP1 results and floor flags |
| Output rendering | `src/rendering.rs`, `native/{ocio_bridge.cpp,rendering_transform.h}`, `assets/ocio/` | Shared OCIO ACES target definitions, CPU reference/export and pinned generated GPU algorithms/tables |
| Viewer/refinement | `src/{viewer,refinement,resample,viewport}.rs` | Provisional GPU images, tiled settled rendering, area averaging and source/view geometry |
| Desktop host | `src/platform/{host,macos}.rs` | winit events, egui integration, owned wgpu surface, native SDR tagging, repaint/surface lifecycle |
| Editor | `src/{editor,editor_controls,editor_navigation,editor_document,editor_files,editor_flat_field}.rs` | Stable-ID document state, live edits, picker/view workflow, file menus, staged worker transitions and GPU preflight |
| Output files | `src/{export,profiles}.rs` | Full-source TIFF/AP0 EXR, encoding/ICC/metadata, bounded buffers and safe destination replacement |
| Project state | `src/project.rs` | Schema-3 documents, stable source IDs/order, identity/compatibility and atomic save/load; relink primitives have no editor UI yet |

There is no eframe host, vendored renderer patch, custom demosaicer or CPU interactive-viewer fallback.

**Roll-level flat-field correction:** `src/flat_field.rs` estimates/evaluates a bounded three-channel camera-RGB gain field with robust local log-light regression and centre luminance normalization. `Scan::estimate_flat_field` collects weighted full-pixel reference statistics within accepted support; optional `Scan::set_flat_field` applies gains before the AP0 matrix for CPU pixels, measurements and fitting. `processing_calibration` propagates the existing one-code bound without modifying stored Dmin/Dmax. `gpu_flat_field.rs` owns immutable shared gain storage; `SourceGeometry` maps sparse previews/full tiles to exact source pixel centres. `GpuImage::set_flat_field` binds it and invalidates cached parameters; refinement accepts the same resource and rejects CPU/GPU map mismatches. Source storage remains RGB16. `editor_flat_field` owns transactional reference preview/rectangle acceptance and one shared roll CPU/GPU map; accepted correction reaches provisional/settled views, full-resolution picks/fitting, thumbnails and exports. `project_flat_field` persists the map, algorithm contract, enabled state, reference identity and source/roll development snapshots including fixed WB. Reference-only records do not enter the filmstrip; existing photos stay visible. [Plan/checkpoints](plans/completed/flat-field-correction.md#verified-editor-checkpoint).

## Data and colour flow

```text
RAW Bayer samples
  → LibRaw black subtraction / fixed roll WB / AHD development
  → developed camera RGB16 (four u16 slots; fourth ignored)
  → normalize / optional source-coordinate camera-RGB flat-field gains
  → camera-to-AP0/D60 matrix
  → AP0 calibration / versioned inversion / roll gamma balance
  → exposure and AP0-to-AP1/D60 / photographic Bradford WB
  → signed/HDR AP1 working RGB
       ├→ AP1-to-AP0 → scene-linear EXR (no DRT/monitor conversion)
       └→ pinned OCIO ACES 2, 100-nit sRGB or P3/D65
            ├→ target transfer / quantization / ICC → RGB16 TIFF
            └→ target-to-extended-sRGB / encoding → tagged SDR layer
                 → OS monitor conversion
```

Calibration and fitting operate on full-resolution source pixels, never display pixels. Dmin/Dmax are each one AP0 RGB value; they carry no source/rectangle relationship. The development matrix is derived from the camera-to-sRGB convention with explicit XYZ/Bradford D65→D60 conversion. Source bypass enters the rendering path as developed AP1 while ignoring inversion/photographic edits; its transient inspection EV scales only the source-preview post matrix before unchanged ACES. It does not change development, measurement/fitting, saved grades or exported processing.

`Response::Original` is model 2; model 3 adds shared toe/optional shoulder and precise temporary arithmetic. `RollSettings::default()` remains Original for compatibility; the explicit new-grade factory selects model 3, toe 2.5/shoulder off. Model-2 shader execution is retained, not rewritten under a new storage API. See [processing contract](contracts/processing-model.md).

ACES's specified rendering limits are downstream of signed working data. Neither display limiting nor the TIFF transfer affects AP0 EXR. Generated shader assets are algorithms plus small parameter tables, not a baked 3D LUT.

## Ownership and execution

- `RawFile` uniquely owns the native LibRaw handle. `Developed` borrows its camera image; a mutable development operation invalidates the previous view. `Scan` retains the selected developed source and reads it without redecode on adjustment or picking.
- The winit/egui UI owns live edit/view state. A single backend thread owns the selected `Scan` and serially handles load, document work, full-resolution pick, export and refinement commands over channels. Reference editing temporarily owns an additional uncorrected `Scan`, retaining the working photo until accept/cancel; acceptance prepares the corrected candidate before replacement. Imports register headers/fingerprints without developing the whole roll. The first successful development fixes a new document's WB; every later load uses explicit document WB, including reopened projects. Candidate documents/initial GPU previews are prepared before replacing the active scan/state.
- UI and backend share handles to the same wgpu device/queue. Interactive compute/render preparation runs in the viewer; backend refinement submits tiled work. Results return as `Arc<GpuImage>` or small measurements/outcomes.
- Request IDs and generation counters reject stale source/refinement results. Refinement checks cancellation between tiles; submitted GPU dispatches themselves are not cancellable. Picks/exports run against the requested loaded source and a settings snapshot; app-level export cancellation is not implemented.
- Proposed edits/picks are validated before state mutation, including calibrated inversion and GPU precision/range. Worker failure or invalid input is reported; no silent numerical/rendering fallback is selected.
- `CpuRenderer` RAII-owns an OCIO processor behind an exception-translating C ABI. CPU rendering is for export/reference, not interactive presentation.

## Preview, resources and caches

The responsive provisional preview uses nearest-centre camera RGB16 samples on a bounded grid. Processing/rendering after sparse sampling is approximate; measurements and exports never consume this proxy.

After 250 ms idle, refinement visits every full-resolution pixel in the **visible source region**, processes/renders it in `1024×256` tiles, then area-averages **linear rendered target RGB** to the physical display grid. Nonlinear inversion does not commute with averaging negative inputs. Regional refinement is not a whole-image scope; scopes need their own explicit coverage policy.

Each source pixel occupies 8 bytes. A tile source buffer is 2 MiB and the existing working buffer 4 MiB. Camera constants occupy 144 bytes; processing parameters/signatures 224 bytes. Higher precision is temporary expansion/pair arithmetic, not a retained wide source image. `GpuImage::new_rendered` creates a settled target/presentation image without source, working or unused processing resources.

The host requests the adapter's reported storage-binding and buffer-size limits; other device limits remain at wgpu defaults. These are API capacities, not reserved memory or an aggregate application budget. Averaging retains 16 scratch bytes per output pixel alongside the 16-byte linear output texture. Strip reconstruction is deferred unless actual memory/performance problems justify it.

Preparation is keyed by actual parameters; unchanged repaints do not rerun compute/render work. Source replacement invalidates that key. The editor also keys settled work by target/view and rejects old generations. Only one settled job is outstanding, and tile completion is drained to bound staging accumulation. An aggregate application memory budget and whole-roll RAW cache are not implemented.

`editor_thumbnails` uses a separate one-request worker, leaving the selected `Scan` untouched. It verifies full-byte source identity, develops one temporary RAW with recorded WB, extracts a maximum-dimension-256 nearest-centre RGB16 proxy, then releases the RAW. Worker RGB16 and UI `GpuImage` LRU caches each hold at most 16 entries (~48 MiB maximum retained pixel storage together, excluding driver overhead, callbacks/in-flight work and one temporary RAW). Source ID/fingerprint/source and roll development key uncorrected proxies; target and exact active correction map additionally key GPU images. Images share immutable gain resources; retained map storage is additional to the pixel-storage figure and bounded by the same 16-entry image cache. Actual roll/photo parameter changes reuse these images and rerun existing GPU processing/rendering; uncalibrated thumbnails show zero-gain developed source. Visible/nearby requests wait for foreground/refinement idle. Generation cancellation is checked between phases; stale results never install, and cancellation does not enqueue another request before the old one completes. The strip virtualizes horizontal cells and resizes transiently; no project schema/export change.

## Presentation and exports

The host owns the surface and reapplies explicit extended-sRGB tagging after configuration. It uses `RGBA16Float`, encoded extended-sRGB SDR content and EDR disabled. The image callback converts from the selected linear target and encodes once; ordinary egui UI is already encoded sRGB. Core Animation/ColorSync performs monitor conversion—there is no second DRT or application monitor transform. Software tagging/readback is not physical-monitor validation.

TIFF/EXR are recomputed from full-resolution source through the f64 pipeline. TIFF then uses OCIO CPU rendering, matching transfer/ICC and RGB16 LZW/prediction. EXR converts AP1 to AP0 and writes ZIP16 half/full with overflow rejection. Writers use same-directory temporary files and safe no-overwrite/confirmed replacement; buffers currently have a 1 GiB cap each. These guarantees are not a full power-loss or cross-platform exclusive-rename claim. See [output contract](contracts/output.md).

## Persistence boundary and known gaps

`Document` owns authoritative `Project` state, its path and saved baseline. Accepted controls flush into source-ID-owned photo settings and shared roll/calibration/output state; no competing path-keyed photo store remains. Selection, ordinary picker rectangles/return context, viewport, source bypass, GPU resources and worker IDs stay transient. Accepted flat-field support, gains/reference identity and Enabled are persistent; cancelled rectangle edits are not. Save validates a snapshot and updates the baseline/path only after atomic write success; Save As rebases relative locators. Open verifies runtime/GPU domains and ordered sources, prepares the first photo with saved WB, then commits. New/Open/Close share a Save/Discard/Cancel guard; document work suppresses stale mutations while retaining the old display. See [project-state contract](contracts/project-state.md).

Recovery/autosave, missing-source/relink/changed-source UI and undo/redo remain future work. Unavailable ordered RAWs currently reject Open; nonordered unavailable records and stored reference RGB remain preserved.

Source-inspection brightness and Mac scroll/pinch/click/drag/cursor/Fit behavior are implemented; physical trackpad feel remains unverified. Bounded thumbnails are implemented. Other scheduled gaps include whole-frame scopes, remaining layout/colour investigations, geometry, assisted roll analysis, batch/export cancellation/metadata and distribution. See [roadmap](../PLAN.md), not this overview, for priorities. Broader device/resource tests, macOS 14 and physical-monitor checks remain separate acceptance work; no minimum chip/RAM target has been established.
