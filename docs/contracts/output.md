# Rendering, presentation and file output

## Shared rendering

`native/rendering_transform.h` defines both CPU reference/export and generated GPU rendering: AP1 input range `[0,1024]` → AP1-to-AP0 → unmodified OCIO 2.6.0 ACES Output Transform 2.0 → target range `[0,1]`. These are display-rendering limits, not working-data/EXR clamps.

Active targets are **100-nit sRGB/D65 and Display P3/D65**. Rendering/limiting primaries match the target file encoding and pinned ICC profile. Both transfers are the standard segmented sRGB curve; Display P3 is not cinema P3 gamma 2.6. Preview follows the chosen TIFF target, not an independent narrow-gamut DRT hidden inside a wide container.

`assets/ocio/` contains pinned generated AP1 algorithms plus two 363-entry 2D parameter tables per target, not a baked 3D LUT. `assets/profiles/` contains original pinned ICC matrix-shaper profiles with explicit D50 PCS adaptation and checksum provenance. Native definitions drive generation/validation; never hand-edit only a GPU ACES equation or embed a monitor profile.

[Target and presentation rationale](../decisions/sdr-output-and-presentation.md).

## Managed presentation

Convert linear target RGB into extended-sRGB, then encode once into `RGBA16Float`. Explicitly tag the macOS Metal layer extended-sRGB after each surface configuration and disable EDR. Ordinary egui UI is encoded sRGB; handle image/UI encoding separately. OS monitor conversion is the only monitor transform. Preserve out-of-sRGB coordinates from P3 before that conversion.

Layer tagging and offscreen reference/readback do not establish physical calibration, 100-nit monitor luminance, monitor-transition correctness or broader-device/macOS-14 acceptance. Zero submitted frames is a failed onscreen check, not success via an occlusion bypass.

## Responsiveness and optional caches

Use available resources to avoid repeated development/render setup, while shrinking optional storage/work under pressure; no minimum chip/RAM target is established. Immutable GPU pipelines/tables are device-shared. Proxies never supply measurements/exports, and cache pressure never changes development, inversion precision, ACES or settled output resolution.

Normal thumbnail budgets follow physical RAM: RGB16 proxy storage is RAM/1024 clamped to 16…64 MiB; GPU image pixel/storage plus small entry overhead is RAM/256 clamped to 32…256 MiB. Missing visible thumbnails may start after foreground source/document preparation, without waiting for settled refinement or grading idle. One request remains outstanding; cold work retains background native priority. Worker completion wakes the UI, with timed polling retained for lifecycle fallback. Picks, crop drafts, document/dialog transitions and critical pressure suspend admission. Optional normal-pressure offscreen prefetch remains idle-only, prioritizing nearby cells and admitting only spare-budget entries without eviction. Resumed foreground work cancels offscreen prefetch. Warning pressure quarters optional budgets and disables prefetch; critical pressure zeroes optional budgets and cancels/suspends new thumbnail work. Current visible GPU images and the current selected proxy are essential working sets and may exceed an optional budget. Source protection is released when the working view is gone. Inactive correction generations leave the GPU cache; the active shared CPU/GPU gain map remains document/image-owned storage outside pixel budgets.

A separate one-request, visible-only embedded-JPEG pass supplies labelled camera-preview placeholders independently of the RAW worker. It checks source/development compatibility and cache-only file revisions, rejects stale/cancelled results, and follows the same foreground/pick/crop/document/critical-pressure admission guards. LibRaw extracts the smallest eligible JPEG without sensor unpack/development: at most 1 MiB encoded and 2048×2048 declared dimensions; decoder checks actual dimensions and a 32 MiB allocation limit. Decoded images retain at most a 256-pixel edge, without upscaling; EXIF orientation takes precedence over native thumbnail orientation, padding is preserved, and embedded ICC is converted to sRGB (otherwise only declared sRGB is supported). Only currently missing visible cells retain textures; replacement, scrolling away or document/key changes release them. There is no persistent/recent JPEG cache or additional RAW development concurrency. Visible camera textures are an essential transient set of approximately 8 bytes/pixel (CPU/GPU RGBA8), plus egui/driver overhead; critical pressure cancels admission/installation while existing visible textures remain available.

These limits exclude the selected Scan, one temporary development, short-lived JPEG/header/decoder temporaries, active/refining outputs/scratch, [scope bins/totals/readback](scopes.md#gpu-lifecycle-and-resources), driver/runtime overhead and in-flight callbacks; they are not an aggregate application or universal OOM guarantee. A normal ≤36-image roll is the retention/use case, not a maximum supported source count.

Disk RGB16 proxies are disposable, versioned/checksummed and atomically installed under `~/.cache/antitone/thumbnails-v2`, with a 128 MiB pruning budget. Cache keys include resolved/canonical source path, length/mtime (also device/inode/ctime on Unix), source development and fixed roll development. Document IDs, grades, targets and correction maps do not key disk proxies. Unavailable source stats disable disk reuse; changed stats quietly miss/reject stale results, never declare the grade incompatible. Stats are cache freshness, not photograph identity or immutable-byte proof. No RAW contents are hashed; corruption checksums cover generated RGB16 pixels only. Corrupt/unavailable cache data is a reported miss; development still reports missing/unloadable/incompatible RAWs. Selected proxies retain their pre-open/development file key rather than acquire later stats. Uncorrected RGB16 proxies are transformed by the current grade/correction, never reinterpreted as baked display photographs. [Decision](../decisions/no-raw-content-hashing.md).

A registered-photo selection may display a labelled **cached pending preview** from an already retained compatible 256 RGB16 memory proxy while normal foreground RAW preparation continues. It has independent image-owned GPU processing/storage for both output targets and applies the current grade/source-inspection gain, correction and accepted composition. It never supplies a `Scan` or authoritative `Ready`: full-resolution picks, crop editing, refinement, scopes and exports remain unavailable until full preparation completes. Missing/unavailable/stale keys keep the loading placeholder; completion, failure, replacement or document work clears pending display. Selection adds no disk read, cache tier or full-roll developed-RAW retention. Pending images are a temporary selected working set outside optional thumbnail budgets; both targets require 80 pixel bytes per proxy pixel plus bindings/driver overhead.

## TIFF

Full-resolution pipeline → OCIO CPU rendering → matching transfer → round-to-nearest RGB16. Write lossless LZW with horizontal prediction and the exact matching ICC payload as an UNDEFINED tag. Reject non-finite/out-of-range encoded values rather than repairing them at quantization. TIFF compression uses the tested whole-image `write_data` API, retaining a bounded RGB16 frame.

## EXR

Full-resolution signed/HDR AP1 → AP0/D60 → ZIP16 half/full RGB EXR. Exclude ACES, transfer encoding and monitor conversion. Record AP0 chromaticities, processing model and exposure convention. Reject half overflow and offer full float; never silently clip/change precision. AP0 pixels alone are not a claim of complete ACES-container compliance.

Full-resolution exports include the accepted Photo contrast/saturation grade before their existing output boundary. TIFF descriptions and EXR `antitonePhotographicProcessing` identify `ap1-acescct-rgb-black-relative-contrast-y-max-saturation-blends-v1`. Metadata identifies the operator family, not a complete reconstructible grade; projects retain exact Photo settings.

Export APIs receive the actual `Response`. Both classic Original and Film use native-camera model 4; Film additionally records exact toe/shoulder and signed-root policy. AP0/model-2/3 project processing is not a supported export path.

## Spatial composition

Editor export snapshots accepted framing (centre, extents and source-plane angle) and exact roll/photo orientation together with calibration/grade. Exact integer cropping/flips/quarter-turns retain pixel permutations with no new interpolation. Rotated output pixels inverse-map to real source coverage and use exact pixel-cell intersection weights. TIFF renders each original processed pixel through unchanged ACES before averaging linear target RGB and encoding; EXR averages signed/HDR scene-linear data and converts AP1→AP0 without ACES or clamping. Bounded TIFF strips retain contributing/rendered samples, not a wide whole-source cache. Output dimensions follow the composed frame and swap for discrete 90°/270° actions. Diagnostic raster-writer APIs remain available, while composed export adapters own shared footprint reconstruction. Every output corner is source-covered; no draft or bounding-box-only pixels enter the export. [Geometry and reconstruction contract](geometry.md).

## Destination safety and budgets

Recompute from full-resolution source, never preview buffers. Write same-directory temporaries; flush/sync before atomic installation. Refuse overwrite unless replacement is explicitly confirmed. macOS exclusive rename protects no-overwrite races; callback errors preserve old files and leave no final partial output. This is not full power-loss durability or cross-platform exclusive-rename support.

TIFF/EXR sample buffers each have a current 1 GiB cap (~306 MiB RGB16/half or ~612 MiB full for the supplied full developed frame, excluding RAW/runtime). Batch snapshots, progress/cancellation, metadata propagation and aggregate memory validation remain scheduled work. Editor export uses the accepted composition within the active metadata image area; the diagnostic CLI intentionally uses full developed dimensions.

Software checks include ICC/transfer exhaustion, ColorSync/Core Image, compressed numerical reopen and full TIFF/ImageIO decode. Lightroom/Resolve interoperability remains unverified/deferred. Use [the runbook](../build-and-probes.md) for checks.
