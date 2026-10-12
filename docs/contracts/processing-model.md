# Processing model 4: native-camera inversion

Current numerical contract. Architecture: [overview](../architecture.md); rationale: [camera-inversion decision](../decisions/native-camera-inversion.md). Models 2/3 and their AP0 grades are incompatible; there is no migration or legacy inversion branch.

The numerical core is an f64 scalar reference using pinned `glam 0.34.1`. It applies neither ACES nor a display transfer. GPU/editor/export checks cover selected numerical paths; they do not establish calibrated scene colour, broader-device or physical-monitor acceptance.

## Color-space roles

Calibration, input-floor diagnostics, gamma balance and nonlinear inversion operate in **developed native-camera RGB**. This is demosaiced, fixed-WB LibRaw output, not a Bayer mosaic, narrowband dye measurements or a standardized colour space. `Inversion::apply` returns native-camera positive coordinates. Dmin→0 and Dmax→1 are per-channel response anchors, not a claim that camera `[1,1,1]` is AP1 neutral.

`Pipeline::new` takes the source's explicit camera→AP1 matrix. `Pipeline::apply` inverts first, then maps positives to linear AP1/ACEScg, applies common exposure and photographic Bradford WB, and returns **AP1**. Scalar composition is `WB_AP1 * camera_to_AP1 * 2^(roll_EV+photo_EV)` after inversion. Preserve signed/HDR values; no intermediate working-gamut clamp.

`Metadata::camera_to_ap1()` composes metadata camera→linear-sRGB, sRGB→XYZ/D65, Bradford D65→D60, then XYZ→AP1. There is no pre-inversion AP0/AP1 matrix. Using this positive camera matrix is the agreed pragmatic baseline, not proof of recovered scene radiometry through overlapping sensor/film spectra. No stock calibration, PCA-derived basis or invented dye unmixing is required/applied.

AP0 remains only at interchange/rendering boundaries: signed AP1→AP0 EXR, and unchanged ACES rendering's internal AP1→AP0 conversion. The rendering input/output limits do not alter working data or EXR. Source bypass maps developed camera RGB directly to AP1 for rendering, ignoring inversion/photographic edits; transient inspection exposure affects only that view.

## Camera source representation

One production GPU source: developed RGB16 in four u16 slots, **8 bytes/pixel**, fourth ignored. Sensor bit depth is separate from this interface's unsigned 16-bit development output. Upstream AHD uses a pinned checkout-local OpenMP runtime with bounded foreground/background CPU budgets; thread policy changes no development identity/settings or pixels. RAW decoding is independent of lazy whole-sensor statistics. Explicit `RawFile::unpack` still returns cached full diagnostics; development/regional picks decode without that global pass, and regional statistics retain original CFA/RAW/black/white/margins after processing. Diagnostic calls preserve cached camera pixels and invalid picks change no state. [Rationale/limits](../decisions/established-raw-development.md). No float-AP0 upload, retained wide converted source or CPU interactive fallback.

`Scan::pixel`, `patch` and `reference` return normalized camera RGB, optionally corrected in camera coordinates. Full-resolution CPU sampling/fitting/export use f64. Dmin/Dmax are compensated linear means before inversion; roll fitting processes every positive sample rather than inverting the mean scan.

`preview_source` uses integer nearest-grid centres; tiles copy integer full-resolution pixels and pad partial edges with zero. GPU normalization/field interpolation/gain multiplication retain scalar-f64 rounding in temporary f32 expansions. All response shapes use compensated log/exp and post-matrix pairs. Source bypass carries its complete expansion through the camera→AP1 matrix before narrowing. Camera matrix preflight rejects unrepresentable/subnormal constants and unsupported smallest-code products.

Working/linear-target buffers remain f32; presentation remains RGBA16Float. A 1024×256 source tile is 2 MiB with a 4 MiB working buffer. Settled images retain rendered/presentation resources only. Processing constants are 224 bytes, camera constants 144 bytes; optional field geometry is 64 bytes. Higher precision lives in temporaries/small constants, not a retained wide source cache.

## Optional roll-level flat-field correction

`FlatField` stores positive camera-RGB gains, maximum 256×256 nodes, bilinearly evaluated at original source pixel centres. Support is not a crop; outside it gains are exactly identity. Apply gains to normalized developed camera RGB **before inversion**; do not rewrite RGB16 source or apply the field in AP1/display coordinates. For the map's neutral/centre-luminance target, centre-patch Y from camera→AP1→XYZ determines the target; inverse camera→AP1 derives camera channels yielding equal AP1/D60 gray. Camera channels need not be equal.

Estimation bypasses any accepted field, rejects sensor black/white contamination and collects full-pixel physical-cell statistics strictly inside support. Field acceptance validates support before mutation. Corrected CPU pixels, reference means, fitting, GPU viewer/refinement/thumbnails and export share the same map. Stored Dmin/Dmax are never automatically resampled. [Persistence](project-state.md#flat-field-state), [controls](editor-controls.md#flat-field-workflow).

The uncorrected input floor is **one normalized camera code, `1/65535`**, independent of the colour matrix. Corrected floor is the maximum channel gain over nodes and identity, divided by 65535. Bilinear extrema occur at nodes. `processing_calibration` requires the native-code base floor and replaces a runtime copy with this exact corrected floor; it rejects already-amplified/non-base floors rather than applying them twice. It may reject formerly usable Dmax or response settings; removal restores the base floor. This is a quantization bound, not a measured noise floor, pedestal or strength/clamp control.

`GpuFlatField` shares nine-float nodes, 36 bytes/node (maximum 2.25 MiB). CPU/GPU source geometry uses the same nearest-centre mapping. Preflight rejects corrected-code/product underflow, unsupported constants/ranges and invalid geometry. Corrected and uncorrected source views use the same precise source-inspection path.

## Coordinates and ranges

Both two-coordinate controls use the orthonormal mapping:

```text
w = warmth / sqrt(2)
t = tint / sqrt(6)
log_offsets = [w+t, -2*t, -w+t]
```

Warmth/tint range −1…1. Gamma is `global_gamma * exp(log_offsets)` in **native-camera channels**, with geometric mean equal to global gamma. Global gamma defaults to 0.65 and ranges 0.1…3. These are initial control ranges, not universal film-response limits or physical Kelvin/saturation claims.

Photo WB uses Bradford in AP1: `C = Bradford * AP1_to_XYZ`, `WB = inverse(C) * diag(exp(log_offsets)) * C`. The reference white is D60; zero coordinates give exact identity and gains have unit product/determinant. This does not imply pixel luminance preservation.

Roll/photo exposure remain separate persistent state, each −16…16 stops, default zero, multiplier `2^EV`. GPU exposure is folded into response evaluation before camera→AP1/WB to avoid overflow before attenuation.

## Versioned film response

All response shapes identify **processing model 4**. New grades use toe **2.5**, shoulder **0/off**. `Response::Original` / numerical `RollSettings::default()` denote the classic toe-1/shoulder-0 curve in the same camera model, not an AP0-grade compatibility path. Projects serialize response explicitly, including Original; missing response rejects.

For effective channel gamma `g`, shared toe `beta`, shoulder `k`, and `l = ln(dmin/max(scan,floor))`:

```text
a(l) = beta*l / (g-k*l/ln(10))
A = a(ln(dmin/dmax))
E = sign(a) * exp((ln(abs(expm1(a)))-ln(expm1(A)))/beta)
```

Dmin maps to 0; Dmax maps to 1. Toe 1…4; shoulder 0…0.02. At beta 1 / shoulder 0 the scalar uses the same stable `expm1` ratio. Above-base negatives use signed-root continuation as a numerical/noise policy, not physical film exposure. No pedestal, approximate ACES, scan clamp or response-derived source repair. The default is generic, not stock calibration or a population median.

Require finite `dmin > dmax >= floor > 0` per camera channel, `ln(dmin/dmax) >= 1e-6`, valid shapes and `g-k*log10(dmin/floor) > 0` across the floor domain. Nonfinite pixels error; finite below-floor values set camera-channel flags then floor. AP1 negatives introduced by the matrix are not input-floor events.

GPU preparation rejects unsupported constants, normal-floor/range failures and floor-response exponents above **4096**, including near-poles hidden by floor-anchored Dmax. Range checks cover both positive floor response and the above-base negative-continuation bound, through each actual camera→AP1/WB intermediate. No CPU fallback or weakened numerical gate.

TIFF descriptions and EXR processing metadata identify model 4/native-camera inversion. Film shapes retain exact toe/shoulder strings and continuation policy. [Output](output.md) defines AP0 EXR pixels/chromaticities, ACES and matching target encoding/ICC.

## Inversion and validation

Dmin/Dmax are RGB-only camera measurements; source/rectangle/working-photo return context stays transient. Development compatibility and precision floor are independent of RAW byte changes; source contents are not hashed. Only project schema 5 with compatible camera development is supported; incompatible documents reject rather than reinterpret their numbers. A retained roll `CameraBasis` (camera model plus matrix) must match all sources, including transient calibration/flat-field references; incompatible cameras require a separate roll/calibration. Basis identity survives removal of source records. [Project contract](project-state.md).

Roll fitting uses actual-response full-resolution native-camera positive means. The neutral direction is `target = inverse(camera_to_AP1) * [1,1,1]`, required finite and positive. Solve log ratios of `mean_positive/target` using the existing bounded Newton/finite-difference/line-search method. It does **not** force equal camera channels or mutate calibration/global gamma/exposure. Reject near-anchor/ill-conditioned fits (`abs(det(J)) < 1e-9`), nonpositive means (`<= 1e-8`), floored patches, out-of-range corrections and non-convergence. Successful residual norm is `< 1e-9`; synthetic coordinate recovery error `< 1e-8`. Sensor clipping is also checked from RAW diagnostics.

Photo fitting uses actual-response AP1 before existing photo WB, computes Bradford ratios to D60 white, removes common log gain and returns replacement controls. Reject nonfinite/nonpositive cone responses and out-of-range controls. Negative AP1 components alone are not grounds for rejection. Invalid/cancelled picks and rejected edits change no processing state.

## Verification tolerances

- Scalar algebra: `abs(error) <= 2e-12 + 2e-11*abs(reference)` in tested domains.
- f32/GPU inversion: `abs(error) <= 2e-5 + 2e-4*abs(reference)`; floors/anchors have separate checks.
- OCIO CPU/GPU/rendering, target encoding/ICC and presentation retain their separate gates; scalar agreement does not establish colour accuracy or hardware acceptance.

The working buffer's fourth component holds floor flags, not alpha. Output/presentation boundaries are defined in [the output contract](output.md). [Development checks](../development.md) and [specialist probes](../build-and-probes.md) exercise scalar/GPU/source, composition and export paths.
