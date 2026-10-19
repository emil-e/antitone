# Embedded lens correction at RAW development

Status: automatic Sony shading and distortion/lateral-CA correction are implemented at the RAW boundary. [Processing](../processing.md#embedded-sony-lens-correction), [flat-field](../flat-field.md), [source storage rationale](../decisions/rgb16-gpu-source.md) and [projects](../projects.md#compatibility-is-about-the-processing-basis) own the current choices. The bounded [DNG SDK evaluation and opt-in native prototype](../research/dng-sdk.md) are complete; application DNG integration and broader photographic/resource acceptance remain unfinished. [The roadmap](../../PLAN.md) owns priority.

## Direction and support

RAW conversion supplies one immutable linear camera-RGB raster after supported embedded correction. Inversion remains before camera-to-AP1 conversion, and downstream composition, masks, measurements, repairs and outputs use that raster's coordinates. A corrected-source contract describes available-data handling; it does not imply an adapted lens was corrected.

| Input/instruction | Current behaviour |
| --- | --- |
| Genuine Sony RAW, valid embedded distortion/CA | Automatic combined camera-space reconstruction |
| Sony absent/unavailable/all-zero coefficients | Native integer development and coordinates retained |
| Malformed available Sony profile | Reject development; metadata diagnostics can still inspect it |
| Converted DNG carrying Sony coefficients | Ignore inherited Sony profile |
| DNG opcode correction | Direct SDK backend in shared editor sources; bounded gain/repair/warp admission and supplied M9/Q2/GR III/IIIx checks, separate photographic/resource/release acceptance |
| Sony embedded vignetting/shading | Source-site gain before geometric reconstruction; measured flat field corrects residual illumination |

The supplied profile-bearing set is one camera/lens combination: Sony ILCE-1M2 / FE 24–70mm F2.8 GM II. Metadata checks cover 112 captures spanning 24–70 mm and f/2.8–f/8; distortion is marked Off, not unavailable, and CA Auto. The inspected adapted-lens scans report unavailable distortion/CA and identity shading. Authored TIFFs exercise parser isolation, byte order, channels, geometry, reconstruction, capture clipping, precision, persistence and export without private-image dependencies. This is not universal Sony or photographic acceptance.

The current Sony baseline uses active-image geometry, positive bilinear reconstruction and conservative rectangular coverage without replicated borders or additional zoom-to-fill. It retains fractional originals as f32 while keeping a compact native no-op path. These are revisable technical choices, not mandated representations or a claim of exact vendor output.

## Remaining Sony acceptance

- Compare actual corrected photographs at wide/tele zoom and differing apertures: line straightness, edge registration, fine texture/aliasing, dense-negative detail, field of view and illumination. Inspect orientations and cold/reopened shared consumers, not just matching intermediate buffers.
- Compare shading against a suitable optical/vendor reference, including whether camera firmware has already baked some shading into RAW. Public implementations differ in interpolation convention; the current attenuation-interpolation interpretation is a baseline, not vendor proof.
- Validate the active-frame/optical-centre convention against a suitable independent reference. Sony's RAW IFD, active crop and LibRaw's padded raster are distinct; a darktable pixelpipe buffer is not evidence that it uses LibRaw padding. Current source binding is a deliberate baseline, not vendor-calibration proof.
- Test measured flat references on profile-bearing scans. Developing reference and scans into the same corrected coordinates is not identical to multiplying gains before reconstruction. Current exact lens-development matching is intentionally stricter than camera-roll compatibility; dimensions alone cannot establish reference suitability. Revisit a prewarp gain hook only with useful evidence.
- Measure decode-to-ready latency, aggregate/in-flight memory, source switching and foreground/background work. A 50-MP three-channel f32 raster is about 570 MiB, plus original CFA; native RGB is released after reconstruction but overlaps the output during construction. Corrected capture eligibility retains two bits per output pixel, with row padding. Its one-time native support dilation uses temporary packed masks; queries need no source-geometry reconstruction. Bounded coordinates/tiles and optional-cache accounting are not aggregate residency guarantees.
- Exercise supported thread/pressure policies and complete-source determinism. Broader hardware, physical interaction and photographic approval remain distinct from numerical checks.

Saved calibration, crops, local masks, donors and sampling evidence must not silently move onto another development. The implemented contract explicitly rejects older developments; it is not a migration or a retained legacy path. Any recovery/migration workflow needs a separate decision.

## DNG backend follow-up

Sony ARW discovery retains the bounded LibRaw callback; Adobe's SDK is a separate DNG backend. The [current support/basis and supplied-camera evidence](../research/dng-sdk.md#maintained-direct-sdk-adapter) describe shared immutable ownership, conservative original capture flags, source precision and explicit saved compatibility. M9/Q2 and Ricoh GR III/IIIx decode and cold-roundtrip through shared projects; authored editor/export checks are separate from photographic acceptance.

Remaining acceptance:

1. Assess actual negative scans and vendor/reference correspondence for the chosen post-opcode WB, bilinear demosaicing, bad-pixel repair and cubic warp. Successful camera-photo decode does not establish negative-inversion or lens-edge quality.
2. Exercise real default crops/orientations, physical frame validity, measured residual references and complete interactive pick/Analyze/repair/export workflows. Maintain original capture authority; neither cropped RGB thresholds nor numerical agreement alone establish photographic eligibility.
3. Extend initial full-resolution timings/peak-RSS observations to real-camera interruption latency, in-flight overlap, mixed-roll aggregate memory pressure and broader hardware/firmware. Current cache charges do not bound SDK scratch or a running native call.
4. Resolve production dependency selection, reproducible clean-machine builds and SDK/bundled-library release terms/notices. Local validator-derived builds are not a reviewed distribution.
5. Expand sensor/opcode support only for concrete unsupported inputs, with matching authored range/capture coverage and explicit saved interpretation. Signed/linear/non-Bayer sources, nonuniform black/linearization and broader instructions remain separate work rather than hidden adaptation or fallback.

Use the normal [native setup/build workflow](../../README.md#build-and-run), including [existing dependency reuse](../native-development.md#reusing-native-builds). Disposable evaluation builds belong under `.cache/`; do not acquire or install a new SDK implicitly. A complete custom instruction engine is separate scope, not the automatic fallback. Lensfun/LCP management, manual calibration, other vendor parsers, a custom demosaicer, general positive-image import and universal DNG support are not part of this slice.

## Evidence and completion

Use [development checks](../development.md), authored RAWs and proportionate production builds. Geometry expectations should come from analytic cases or independent references, not only agreement between consumers. Keep existing photographic numerical gates; test reopened TIFF/AP0 EXR as well as arithmetic. Preserve no-profile identity, native-base versus amplified-source floor handling, original-site clipping, per-source lens identity and failed/stale publication safety. Real DNGs and broader cameras are needed before claiming that coverage.

A finished DNG slice needs a documented backend/support table, single execution of supported instructions, coherent camera range and capture diagnostics, shared source coordinates, compatible saved interpretation and measured resource behaviour. Photographic and hardware acceptance remain separately stated, not inferred from green tests.

## Next action

Continue the remaining photographic/reference and resource acceptance against the Sony baseline. For DNG, assess actual negative scans in the shared editor workflow and compare photographic/edge behaviour against independent references before broadening the [bounded SDK backend](../research/dng-sdk.md#maintained-direct-sdk-adapter).

## Sources

- [LibRaw 0.22 DNG integration notes](https://github.com/LibRaw/LibRaw/blob/0.22.0/README.DNGSDK.txt) and [implementation](https://github.com/LibRaw/LibRaw/blob/0.22.0/src/integration/dngsdk_glue.cpp).
- [Adobe DNG specification and SDK](https://helpx.adobe.com/photoshop/digital-negative.html); confirm the selected version against its SDK/specification during evaluation.
- NegPy revision `0dc536a8dbf37ab103ad1a90e9cc200de1cd176b`: [metadata](https://github.com/marcinz606/NegPy/blob/0dc536a8dbf37ab103ad1a90e9cc200de1cd176b/negpy/infrastructure/loaders/lens_metadata.py), [radial model](https://github.com/marcinz606/NegPy/blob/0dc536a8dbf37ab103ad1a90e9cc200de1cd176b/negpy/features/lens/warps.py) and [execution](https://github.com/marcinz606/NegPy/blob/0dc536a8dbf37ab103ad1a90e9cc200de1cd176b/negpy/features/lens/logic.py). The model is useful evidence for knot locations and coefficient units; its input-image shape alone does not establish Sony's physical calibration frame.
- [Exiv2 0.28.5 tag definitions](https://github.com/Exiv2/exiv2/blob/v0.28.5/src/tags_int.cpp) identify Sony shading tag `0x7032` as 17 signed shorts; [RawTherapee 5.12](https://github.com/RawTherapee/RawTherapee/blob/5.12/rtengine/lensmetadata.cc) confirms count-prefixed samples and the reciprocal knot gains.
- [darktable 5.6.2 embedded lens implementation](https://github.com/darktable-org/darktable/blob/release-5.6.2/src/iop/lens.cc) and [DNG opcode parser](https://github.com/darktable-org/darktable/blob/master/src/common/dng_opcode.c). GPL code reuse needs licensing review; these are research references, not imported implementation.
- [Apple CIRAWFilter](https://developer.apple.com/documentation/coreimage/cirawfilter) and [Lensfun](https://lensfun.github.io/manual/latest/) are alternative boundaries if concrete SDK limitations warrant them; neither is established here to provide the whole embedded camera-native development contract.
