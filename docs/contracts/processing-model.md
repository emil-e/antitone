# Processing models 2 and 3

Current numerical contract. Architecture: [overview](../architecture.md); rationale: [decisions](../decisions/README.md).

The numerical core is a double-precision scalar reference using pinned `glam 0.34.1` vectors/matrices. Owned GPU kernels, the editor/export implementation and OCIO/presentation probes validate selected processing/rendering paths; basic saved-editor state/output reproduction is verified; broader device/monitor acceptance remains incomplete. The scalar core itself does not apply an ACES transform or a display transfer function. Matrix constants use explicit column constructors; matrix-vector products and inverses are supplied by `glam`.

## Color-space roles

Inversion, photographic working space and file encoding are independent roles. The initial inversion basis remains AP0/D60. `Inversion::apply` returns values in that basis; Dmin/Dmax measurements, input-floor diagnostics and roll-gamma fitting stay there.

`Pipeline::apply` converts the inversion result to linear AP1/ACEScg, then applies photographic exposure and Bradford WB, returning **AP1** working RGB. AP0/AP1 conversion uses their XYZ definitions and `glam` matrix inversion; both share D60, so no chromatic adaptation is required at this boundary. Preserve signed and above-one values: there is no working-gamut clamp.

Scene-linear EXR converts processed AP1 to AP0/D60 at export via `ap1_to_ap0()`, without output rendering. The OCIO rendering probe accepts AP1, applies the upstream ACES rendering input limit in AP1, converts to AP0 for the fixed-function ACES transform, and renders to the selected TIFF target. Those display-rendering limits never affect working data or EXR. The developed-source view likewise converts to AP1 for rendering while bypassing inversion/photographic edits.

A selectable inversion basis remains a separate deferred feature. Processing version 2 records the working-space change; it does not change the inversion formula, gamma mapping, calibration units, or EXR encoding. Tests verify that the Bradford operation remains equivalent in XYZ/AP0 interchange coordinates.

## Camera source representation

The sole production GPU input is LibRaw-developed camera RGB16, retained in its native four-u16-slot pixel layout (8 bytes/pixel; the fourth slot is ignored). This is developed/demosaiced camera RGB, not the original Bayer mosaic. Sensor bit depth is separate: the supplied Sony RAWs contain 14-bit samples, while this LibRaw development interface returns unsigned 16-bit channels. There is no float-AP0 upload API or wider converted-source cache.

`Scan::preview_source` samples integer pixels at nearest display-grid centres; `tile_region` copies full-resolution integer source pixels, padding partial edge tiles with zero. The GPU normalizes RGB and applies the immutable per-source camera→AP0 matrix using f32 expansions/FMA in shader temporaries. Small matrix constants carry the required precision; retained source storage remains RGB16. Matrix preflight rejects unrepresentable/subnormal constants and coefficients whose smallest nonzero camera-code product would underflow.

For model 2, conversion produces the same f32 AP0 values previously obtained by casting the scalar f64 camera conversion, then runs the unchanged inversion/photographic shader mathematics. Tests check exact f64 conversion and original f32 input bits across 393,216 RGB16 samples over three matrices. These finite checks do not prove all possible metadata/matrices or the expanded extreme-exposure domain; see [precision gate](../research/film-response-precision-gate.md).

Working AP1 and linear target outputs remain f32; presentation remains RGBA16Float. A 1024×256 source tile is 2 MiB (previously 4 MiB), with its existing 4-MiB working buffer. Settled output images retain only the rendered target/presentation resources, with no source, working buffer or unused processing/ACES pipeline. CPU calibration/fitting/export still read full-resolution normalized RGB16 and use the unchanged f64 camera conversion. This upload change does not alter the processing model, project schema, development/output contracts or export metadata.

## Coordinates and ranges

For both two-coordinate controls, the orthonormal opponent mapping is:

```
w = warmth / sqrt(2)
t = tint / sqrt(6)
log_offsets = [w + t, -2*t, -w + t]
```

Both coordinates range from -1 to 1. Positive warmth raises the red/L response relative to blue/S; positive tint raises red/L and blue/S relative to green/M. The wheel, sliders and numeric entry will edit these same coordinates. No physical Kelvin claim is made.

Gamma uses `global_gamma * exp(log_offsets)` in AP0 channels. Global gamma defaults to 0.65 (the mockup's starting value) and ranges from 0.1 to 3; the channel gammas remain positive and their geometric mean stays equal to global gamma. These are initial control ranges, not universal film-response limits.

Photo WB uses Bradford cone coordinates: `C = Bradford * AP1_to_XYZ`, then `WB = inverse(C) * diag(exp(log_offsets)) * C`. AP1 white is D60. Zero coordinates are an exact identity. The gains' geometric mean is one, separating the explicit exposure control from an arbitrary common cone gain. This is determinant normalization, not a guarantee that every pixel's luminance stays constant under WB.

Exposure defaults to zero, ranges from -16 to 16 stops per scope, and multiplies by `2^EV`. Roll and photo exposure remain separate state even when their multiplication combines numerically. Processing retains negative and above-one results.

## Versioned film response

`Response::Original` retains model-2 scalar equations and the existing f32 GPU execution. `RollSettings::default()` retains that original numerical/probe state; **`RollSettings::new_grade()`**, used by new projects and editor sessions, selects model 3 with `Film { toe: 2.5, shoulder: 0 }`. Existing schema-3 documents without a response deserialize as Original/model 2. Original serialization omits the new field; it does not silently rewrite an existing grade. Film response has a tagged `kind: "film"` object with f64 toe/shoulder and processing version 3. Model/response mismatches are rejected. Switching an existing grade is explicit; calibration and unrelated edits are retained.

For each channel, with effective gamma `g`, shared toe `beta`, shoulder `k` and `l = ln(dmin/max(scan,floor))`:

```text
a(l) = beta*l / (g - k*l/ln(10))
A = a(ln(dmin/dmax))
E = sign(a) * exp((ln(abs(expm1(a))) - ln(expm1(A))) / beta)
```

Dmin maps exactly to 0 and Dmax to 1. Toe is 1…4, default 2.5; shoulder is 0…0.02, default/off 0. At beta 1 / shoulder 0 the scalar dispatch calls the unchanged original stable `expm1` ratio, bit-for-bit. Above-base negative values use the approved signed-root continuation, explicitly a numerical/noise policy rather than physical below-base exposure. There is no pedestal, scan clamp, flat-field correction or ACES-derived shaping. The generic default is not a stock calibration or population median; see [manufacturer study](../research/film-response-model-study.md) and [image validation](../research/film-response-image-validation.md).

Scalar construction rejects invalid shapes and any `g - k*log10(dmin/floor) <= 0` shoulder pole over the complete floor domain. Full-resolution roll fitting evaluates this actual response on every sample/trial; photo fitting uses actual-response AP1 before the existing photo WB. Gamma/balance/exposure coordinates are unchanged.

Model 3 retains source conversion/calibration differences in temporary expansions, and evaluates response log/exp and AP1/WB sums with compensated f32 pairs. Only small constants/uniforms grow (processing block **224 bytes**, camera matrix 144 bytes); retained source/working/output formats remain unchanged. GPU preparation rejects unsupported range, unrepresentable precision constants and floor-response exponents above **4096**. The latter bounds the near-pole arithmetic domain even if Dmax equals the floor and the usual gain-bound test would see only E=1. Tested floor-anchored exponents 80/400/1000/4000 pass; a nearly singular floor-anchored inverse is rejected. These finite gates are not an all-metadata/domain proof.

TIFF/EXR APIs receive the actual response: original model-2 metadata remains unchanged; model-3 TIFF description and EXR `antitoneFilmResponse` include exact toe/shoulder strings and signed-continuation policy. `antitoneProcessingModel` identifies 2 or 3 accordingly. AP0 EXR pixels, ACES/target transforms, ICC and development/output contracts are unchanged. [Numerical/editor evidence and original-model limits](../research/film-response-precision-gate.md#production-film-response-checkpoint).

## Inversion and validation

`Calibration::{dmin,dmax}` contains linear RGB measurements, not optical-density numbers. Dmin is the minimum-density film base; Dmax is the maximum-density reference. Consequently Dmin transmits more light and has higher RGB values.

Original/model 2 uses the agreed `((dmin/scan)^(1/gamma)-1) / ((dmin/dmax)^(1/gamma)-1)` formula, evaluated as an `expm1` ratio of logarithmic density differences. Near equal scan/Dmin values use `log1p`; for large exponents evaluate the ratio without first overflowing its numerator and denominator.

Require finite `dmin > dmax >= input_floor > 0` in each channel. Require `ln(dmin/dmax) >= 1e-6` to reject indistinguishable references. The input floor is an explicit calibration/development-policy parameter, **not a hidden default**; the supplied-roll probe projects one developed camera code to `2.163093362227e-5` in AP0 units. This remains a precision-bound candidate, not a measured sensor-noise estimate or silently selected production calibration. Non-finite scan values are errors, not floored values. Floor flags identify affected channels. Valid scan values above Dmin produce negative positive-image RGB and are preserved.

Dmin/Dmax samples are compensated means of full-resolution linear scans before inversion. Each stored reference is only one RGB value; sampling source/rectangle and working-photo return context are transient. Calibration basis, development identity and precision floor belong to the roll, while source fingerprints belong to photos for relinking. Re-picking starts on the current image without reinstating a prior patch. Project schema 3 stores `dmin`/`dmax` as RGB arrays and rejects older provenance-bearing schemas. It now supports explicit response models 2/3 without changing calibration/development/output identity.

Neutral picks fit the mean of full-resolution positive pixels, not the inversion of the mean scan and not display pixels.

Gamma picking solves two log RGB-neutrality residuals using a bounded Newton method, finite-difference Jacobian and decreasing-residual line search. Global gamma/exposure and calibration remain immutable inputs. Reject near-anchor/ill-conditioned fits (`abs(det(J)) < 1e-9`), nonpositive mean positive RGB (`<= 1e-8`), floored input patches, out-of-range corrections, and non-convergence. Successful fitting requires residual norm `< 1e-9`. Input saturation rejection must also use the RAW diagnostic/mask when that information becomes available.

Photo picking computes the Bradford gain ratios from the pre-WB AP1 patch mean to the D60 white response, removes the common log gain, and projects to the same two control coordinates. Reject non-finite means, nonpositive/non-finite cone responses and out-of-control-range corrections. Negative AP1 components alone are not a rejection reason: a usable neutral cast can lie outside AP1 while its cone responses remain valid. Picks return replacement coordinates; state/undo application belongs to the UI/project layer.

## Verification tolerances

- Scalar algebra/formula tests: `abs(error) <= 2e-12 + 2e-11 * abs(reference)` in the tested domain.
- Gamma-fit coordinate recovery tests: absolute error `< 1e-8` on synthetic full-resolution patches.
- f32/GPU inversion tests: initial acceptance `abs(error) <= 2e-5 + 2e-4 * abs(reference)` over the declared operating test domain; measure behavior near floors/anchors separately.
- OCIO CPU/GPU rendering: establish target-specific tolerances before adopting the generated shader; scalar inversion tolerances are not automatically appropriate to ACES table interpolation.

GPU exposure is folded into the stable ratio before AP1/WB conversion to avoid overflowing a large intermediate before attenuation. Source bypass converts developed AP0 directly to AP1 and ignores photographic edits. GPU parameter preparation rejects unsupported f32 range/normal-floor requirements explicitly rather than selecting a CPU fallback. The working buffer's fourth component holds floor diagnostic bits, not display alpha.

Presentation uses encoded extended sRGB on an explicitly tagged RGBA16Float SDR layer; source/rendered RGB must first be converted to that basis. The signed transfer extension is checked against Core Image, including negative and above-one components. The custom image shader encodes once, while egui's ordinary UI is already encoded sRGB. Neither an application monitor transform nor an additional ACES rendering belongs at this boundary. Attachment precision has its own tolerance, separate from f32 processing/rendering.

The reusable OCIO CPU renderer consumes AP1/D60 and shares its target/group definitions with shader generation. It returns limited linear target RGB; target encoding is an explicit separate boundary. Current targets are 100-nit sRGB/D65 and P3/D65. Display P3 and sRGB both use the standard segmented sRGB transfer, with different primaries and matching ICC payloads. ProPhoto and the proposed SDR Rec.2020 target were retired after yellow-collapse gates; the user explicitly approved P3. ACES mathematics are unchanged; see [P3 output evidence](../research/p3-output-update.md) for regressions and output-contract versioning. U16 quantization is round-to-nearest after encoding; invalid or out-of-range output is rejected rather than repaired at that stage. The output transform itself still performs its specified limiting. EXR bypasses the renderer and these transfers.

The pinned original ICC artifacts in `../../assets/profiles/` are generated from the same target primaries/whites, with ICC D50 PCS adaptation and fixed-point precision checked using Little CMS and independently using ColorSync/Core Image. Their validation is distinct from TIFF reopening, Lightroom interoperability and physical-monitor correctness.

A passed scalar test does not validate the camera matrix, RAW headroom, output-target color behavior, display path, or actual device performance.
