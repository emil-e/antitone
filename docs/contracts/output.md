# Rendering, presentation and file output

## Shared rendering

`native/rendering_transform.h` defines both CPU reference/export and generated GPU rendering: AP1 input range `[0,1024]` → AP1-to-AP0 → unmodified OCIO 2.6.0 ACES Output Transform 2.0 → target range `[0,1]`. These are display-rendering limits, not working-data/EXR clamps.

Active targets are **100-nit sRGB/D65 and Display P3/D65**. Rendering/limiting primaries match the target file encoding and pinned ICC profile. Both transfers are the standard segmented sRGB curve; Display P3 is not cinema P3 gamma 2.6. Preview follows the chosen TIFF target, not an independent narrow-gamut DRT hidden inside a wide container.

`assets/ocio/` contains pinned generated AP1 algorithms plus two 363-entry 2D parameter tables per target, not a baked 3D LUT. `assets/profiles/` contains original pinned ICC matrix-shaper profiles with explicit D50 PCS adaptation and checksum provenance. Native definitions drive generation/validation; never hand-edit only a GPU ACES equation or embed a monitor profile.

ProPhoto and 100-nit SDR Rec.2020 were rejected after colour-collapse gates. [Decision and rationale](../decisions/sdr-output-and-presentation.md); [research](../research/aces-yellow-collapse.md).

## Managed presentation

Convert linear target RGB into extended-sRGB, then encode once into `RGBA16Float`. Explicitly tag the macOS Metal layer extended-sRGB after each surface configuration and disable EDR. Ordinary egui UI is encoded sRGB; handle image/UI encoding separately. OS monitor conversion is the only monitor transform. Preserve out-of-sRGB coordinates from P3 before that conversion.

Layer tagging and offscreen reference/readback do not establish physical calibration, 100-nit monitor luminance, monitor-transition correctness or M1/macOS-14 acceptance. Zero submitted frames is a failed onscreen check, not success via an occlusion bypass.

## TIFF

Full-resolution pipeline → OCIO CPU rendering → matching transfer → round-to-nearest RGB16. Write lossless LZW with horizontal prediction and the exact matching ICC payload as an UNDEFINED tag. Reject non-finite/out-of-range encoded values rather than repairing them at quantization. TIFF compression uses the tested whole-image `write_data` API, retaining a bounded RGB16 frame.

## EXR

Full-resolution signed/HDR AP1 → AP0/D60 → ZIP16 half/full RGB EXR. Exclude ACES, transfer encoding and monitor conversion. Record AP0 chromaticities, processing model and exposure convention. Reject half overflow and offer full float; never silently clip/change precision. AP0 pixels alone are not a claim of complete ACES-container compliance.

Export APIs receive the actual `Response`. Original model-2 metadata remains unchanged. Film output records exact toe/shoulder and signed-root policy, with the actual model number; do not label original probes/exports using the latest global version.

## Destination safety and budgets

Recompute from full-resolution source, never preview buffers. Write same-directory temporaries; flush/sync before atomic installation. Refuse overwrite unless replacement is explicitly confirmed. macOS exclusive rename protects no-overwrite races; callback errors preserve old files and leave no final partial output. This is not full power-loss durability or cross-platform exclusive-rename support.

TIFF/EXR sample buffers each have a current 1 GiB cap (~306 MiB RGB16/half or ~612 MiB full for the supplied full developed frame, excluding RAW/runtime). Batch snapshots, progress/cancellation, metadata propagation and aggregate memory validation remain scheduled work. Editor export uses the active metadata image area; the diagnostic CLI intentionally uses full developed dimensions.

Software checks include ICC/transfer exhaustion, ColorSync/Core Image, compressed numerical reopen and full TIFF/ImageIO decode. Lightroom/Resolve interoperability remains unverified/deferred. Use [the runbook](../build-and-probes.md) for reproduction and [recorded evidence](../research/evidence/README.md) for past outcomes.
