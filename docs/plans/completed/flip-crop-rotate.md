# Non-destructive flip, crop and rotation — completed

Status: implemented in `f8969c1`; geometry and crop UX are user-accepted for now. Photographic/native-diagnostic acceptance and dense alignment guides remain separate [follow-up](../completion.md#geometry).

## Scope and result

Shared roll turns/flips, independent photo turns/flips and source-anchored crop/straightening agree across the viewer, full-resolution picks, thumbnails and TIFF/AP0 EXR exports without modifying RAWs or numerical grades.

- A shared coordinate model owns source/oriented/frame/raster mapping, actual coverage and affine presentation. Exact turns/flips are pixel permutations.
- Composition order is active source → roll turn → roll flips → photo turn → photo flips. Turn/flip actions transform the finished composition; straightening turns the image beneath a constrained frame.
- Crop drafts support aspect lock, edge/corner resize, image-under-frame dragging, outside-frame straightening, Apply/Cancel and optional thirds. Full-source editing retains scale/placement across resize; Apply Fits the accepted composition and Cancel restores the starting view.
- Canvas-aligned picks use actual inverse-mapped full-resolution coverage. Dmin/Dmax temporarily reveal uncropped source while preserving physical magnification/source anchors; Gamma/WB retain composition and deliberate picker navigation.
- Settled preview/TIFF process and render each contributing pixel before filtering linear target RGB. AP0 EXR filters signed/HDR scene-linear data without ACES or a clamp. Preview/thumbnail proxies remain bounded approximations, not measurement/export sources.
- Geometry is stable-source-ID-owned and independent of grade/view state. Export snapshots hold geometry and grade together; stale settled work cannot install after geometry changes.

The [geometry contract](../../contracts/geometry.md) defines the coordinate, interaction, filtering and compatibility requirements.

## Baseline and integration boundary

Integration preserves camera model/schema 4, camera-basis identity and accepted flat-field state. Gains and sensor diagnostics stay in **original source coordinates**; crop/frame bounds never redefine field support or calibration RGB. AP0 models/schemas remain incompatible without migration. Metadata orientation is not silently applied.

## Integrated native-camera / flat-field checkpoint

Recorded combined checks cover weighted native-camera fitting, corrected/uncorrected rotated tiles, source-raster/field agreement, nonzero origins, fractional pivots, reflections, tile seams/padded tails, both output targets and composed TIFF/AP0 EXR reopen. Source precision and colour-processing gates are unchanged. Source proxies, tiles, fields, picks, presentation, refinement and exports use the shared geometry rather than competing spatial definitions.

The [archived implementation record](../../research/history/implementation/flip-crop-rotate.txt) retains original designs, intermediate checkpoints, integration results and their environments. Later shared-renderer/composition checks are recorded in [performance evidence](../../research/evidence/rendering-performance.txt).

## Limits and follow-up

Synthetic/Metal/file checks do not establish photographic straightening agreement, real rotated native-diagnostic picks, physical interaction/monitor matching or broad device/resource acceptance. Real-RAW/train/editor/full-size-export acceptance and a release build were not rerun at the geometry integration checkpoint. Dense guides, recovery/undo and broader interoperability remain separate work; no perspective/lens correction, automatic crop, padding, dust removal or CPU interactive fallback was added.

[Completion work](../completion.md#geometry) owns remaining photographic acceptance and guide design; [release work](../completion.md#export-and-release-work) owns device/monitor/interoperability checks.
