# From camera RGB to a graded photograph

An input stage interprets developed camera RGB before shared photographic grading. **Negative** inverts light transmitted through film using measured calibration: denser film becomes a brighter positive. **Digital** treats the RAW as a scene-referred positive, without density inversion or anchors. [Roll analysis](roll-analysis.md) chooses starting calibration and grading for Negative.

The processing pipeline is:

```text
developed camera RGB
  → Negative: optional scan-illumination correction and per-channel inversion
    Digital: scene-referred positive
  → camera-to-AP1 colour conversion
  → ungraded working positive (optional source-owned dust repair here)
  → exposure and photographic white balance
  → Photo contrast and saturation
  → ordered source-anchored local grades, when present
  → working positive, ready for rendering or EXR
```

[Colour and output](colour-and-output.md) explains what happens after the working positive. Display rendering is not part of negative inversion.

## Start with the camera's measurements

LibRaw develops non-DNG RAWs; the editor routes supported DNGs through Adobe's SDK. Both produce linear camera RGB, not sRGB or three isolated film dyes. Each saved source retains its own development identity. Negative always develops with unity WB, so the scanner's as-shot setting cannot change the inversion basis; Digital develops each source with its as-shot WB. The native developer still enforces its supported sensor domain.

Negative uses one shared scan basis: the camera interpretation and developer contract must match, while WB is determined by the stage rather than stored in that basis. Digital admits mixed cameras and per-source as-shot WB. Crossing input stages redevelops active sources for the target stage and updates their recorded identities. [Project compatibility](projects.md#compatibility-is-about-the-processing-basis) owns admission and switching.

The accepted domain is three-colour integer Bayer without repeated black-level patterns: Sony ARW, Canon CR2/CR3 (including CRAW), Nikon NEF, Olympus ORF and Panasonic RW2 develop through it. LibRaw recognizes further formats (X-Trans, decimated sRAW/mRAW, others); broader compatibility is future work.

Negative inverts before the camera-to-working-space matrix: a matrix mixes channels, while inversion applies a different nonlinear response to each channel. An optional [flat-field map](flat-field.md) corrects scan illumination in these camera coordinates before inversion.

### Direct DNG development

The SDK backend defines the current bounded integer-Bayer domain: header and opcode inspection establish geometry, basis and precision without decoding every offscreen sensor, and required unsupported instructions reject — there is no uncorrected LibRaw fallback. One immutable source supplies all consumers: the original pre-opcode CFA remains capture authority, with its white/black/repeated-border flags propagated through repairs and warps. [Saved SDK compatibility](projects.md#compatibility-is-about-the-processing-basis) is separate from source-file freshness.

### Embedded Sony lens correction

Sony RAW development automatically applies available embedded vignetting, distortion and lateral chromatic aberration before inversion or camera-to-AP1 conversion — no measured flat field required. Distortion/CA Off is a camera-rendering preference, not an unavailable profile; unavailable components are skipped, while malformed available data rejects development. Converted DNGs do not inherit Sony profiles.

The current binding reconstructs one conservative valid rectangle at unchanged pixel pitch from the active image geometry; distortion and red/blue CA scales combine at the radial knots before interpolation. Shading multiplies each original developed sample **before** bilinear reconstruction (`2^(0.5 - 2^(v/8192 - 1))` at the knots, interpolated then reciprocated) — this is the [darktable 5.6.2 v2](https://github.com/darktable-org/darktable/blob/release-5.6.2/src/iop/lens.cc) interpretation; [RawTherapee 5.12](https://github.com/RawTherapee/RawTherapee/blob/5.12/rtengine/lensmetadata.cc) instead interpolates reciprocal gains, agreeing at knots but not between them. Neither decoder inspection nor these public interpretations proves exact vendor output.

Corrected originals retain continuous f32 camera samples (fractions and values above native white included) and release LibRaw's redundant RGB16 development; absent or all-zero profiles keep the native integer source. Corrected clipping flags let picks, Analyze and flat-field estimation check eligibility without tracing geometry. [The source-storage study](diagnostics.md#sony-source-storage-precision) distinguishes rounding differences from headroom loss. All consumers use the corrected developed raster rather than applying their own lens warp: [flat references](flat-field.md) require matching lens development, and [projects](projects.md#compatibility-is-about-the-processing-basis) retain its identity.

## Two anchors define the response

The names Dmin and Dmax come from film density, but the stored values are **linear camera RGB measurements**, not optical densities:

- **Dmin** represents low-density film, usually film base. It transmits more light and anchors the positive at zero.
- **Dmax** represents a dense part of the negative. It transmits less light and anchors the positive at one.

Consequently, Dmin's measured value must be greater than Dmax's in each channel. The RGB anchors need not come from one jointly neutral pixel; their role is to establish each channel's response, not to declare a sampled object white.

Between the anchors, the curve works with relative log attenuation: how much less light a sample transmits than the Dmin reference. Gamma controls the response through that interval; toe and shoulder change its shape. New grades use the adjustable response — the classic response is another shape within the same native-camera model, not a compatibility path for old AP0 grades, and **Use adjustable response** is its explicit opt-in.

New grades and gamma resets start at **0.60**, a shared practical response prior rather than a measured film gamma: using one common value leaves stock/process contrast differences available. Analyze fits relative channel balance while preserving the selected global gamma. The anchors are not clipping limits — samples can lie beyond them, and the input precision floor prevents interpreting values below the developed scan's precision as reliable density.

## Curve balance is not white balance

Gamma balance changes the **relative shapes** of the three native-channel curves while preserving their geometric-mean gamma. Its colour effect depends on negative density, so it can correct a colour trend that differs between shadows and highlights.

Photographic WB instead acts on the interpreted positive. Antitone uses Bradford adaptation: AP1 is converted to a cone-response basis, adjusted, and converted back; warmth/tint are coordinates for that operation, not Kelvin measurements.

| Adjustment | Role |
| --- | --- |
| Development WB | Unity for Negative; each source's as-shot for Digital |
| Gamma balance | Changes the native-channel inversion curves |
| Roll WB | Shared photographic colour balance after inversion |
| Photo WB | Individual photographic colour balance after Roll WB |

Manual gamma-neutral fitting uses individually inverted, full-resolution samples; neutral means AP1 white, not equal camera channels. WB picks estimate a replacement at their own scope — the Photo pick measures after Roll WB, the Roll pick bypasses both photographic WB settings.

## Grade in a common working space

After input interpretation, each source's metadata-derived camera matrix maps the positive to AP1/ACEScg with a D60 white. This gives grading and rendering a common basis — a pragmatic colour conversion, not proof that the original scene's spectra or film dyes have been recovered. Roll and Photo exposure add in stops; the working result retains signed and above-one values, so a negative AP1 component is not by itself a failed scan.

### Ungraded positive and repair placement

The ungraded positive is full-resolution signed/HDR AP1 after input interpretation and camera conversion, but before photographic Roll or Photo controls. `Pipeline::ungraded` keeps that boundary explicit: Negative includes correction and calibrated inversion; Digital preserves scene-referred light.

The provisional Dust tool heals here. RAW-development WB and gamma balance are upstream; exposure, photographic WB, contrast, saturation and locals are downstream. Calibration, response, correction or development changes legitimately require asynchronous reevaluation; downstream grading does not. Every independent repair reads the same unrepaired input and preserves signed/HDR values.

Healing and nonlinear grading do not commute, so pre-grade donor matching can differ from matching graded input. Full-resolution repair composition precedes view filtering; saved repairs identify this stage through an explicit [project repair contract](projects.md#source-owned-dust-repairs). Viewer and TIFF/AP0 EXR consume those repairs; [navigation thumbnails](editor.md#positive-thumbnail-reuse) and measurement inputs omit them.

### One accepted image recipe

[Full image recipes](../src/image_recipe.rs) capture accepted source/development, input interpretation and active correction, committed enabled repairs, Global/ordered-local grades, composition and the immutable creative-look asset. Navigation recipes retain grade/look/composition but omit repair payload. Numeric look identity affects rendering, not neutral healing or graded-AP1 reuse. Output target and format are consumer choices, including EXR's explicit look bypass; metadata-only repair IDs, look labels and disabled instructions do not change photographic identity. [Source-aware resolution](../src/repair_source.rs) shares the neutral sampling boundary across interactive and offscreen work.

### Photo contrast and saturation

Contrast scales a log-like ACEScct signal around a pivot; its Colour preservation blend reduces some contrast-driven colour changes without promising constant perceived saturation. Saturation blends between preserving AP1 luminance and anchoring the largest RGB channel, which gives the Density control its darkening character. Neither endpoint is a physical dye or print simulation.

### Local grades change controls, not rendered pixels

A linear or radial gradient determines how much of a local photographic adjustment reaches each original source pixel. Global grading runs first; ordered locals then apply exposure, Bradford WB, contrast and saturation in AP1 before rendering or geometric averaging, and their order matters across nonlinear stages.

Coverage, inversion and strength produce a weight between zero and one: exposure and WB scale by it, while contrast and saturation interpolate toward identity. Thus half coverage of +1 EV means +0.5 EV — not a halfway blend of two output pixels. Masks are analytic gradients, not retained rasters; [geometry](geometry.md#source-anchored-local-masks) explains why composition and navigation must not move them. The scalar pipeline and GPU grades consume immutable snapshots; full-stack preflight rejects unsupported attenuation/re-amplification rather than clamping.

### Preserve information, reduce mathematical work

GPU source processing retains compensated constants and temporaries rather than blanket ordinary-f32 arithmetic. Near film base, losing a small sample/reference difference can become visible after exposure; camera-matrix cancellation and later grades can likewise expose errors. Ordinary-output processing instead reduces transcendental **arguments** before using shorter compensated polynomials, then recombines them with stable identities — reducing mathematical work, not precision where it matters.

TIFF and EXR use the same guarded export response: compensated sample/reference subtraction near film base, one-word f32 where downstream sensitivity and log anchors allow. Both formats use FP32 linear working pixels and stable area accumulation. This is a current accuracy/cost choice, not a constraint imposed by TIFF or EXR storage. Sensitive values keep compensated inversion irrespective of format; one cached export kernel uses the existing renderer. An exact-code response cache is not retained — lens and flat-field correction make continuous processing the representative workflow, and a separate integer-only optimization would add machinery without serving that path. [Diagnostics](diagnostics.md#local-processing-and-reuse) distinguish this from original-sample processing.

### Physical-view processing is an approximation

The positive editor prototype factors exposure and photographic WB downstream of neutral inversion/camera AP1: original camera samples still receive continuous correction and calibrated inversion before exact affine-footprint filtering into signed/HDR f32 AP1, and fixed-view grading reuses that filtered input rather than processing the whole original again.

Filtering before nonlinear grading does **not** commute with original-sample grading before reconstruction, and footprint-centre local masks are not the original-sample mask integral. These are preview approximations, not changes to the photographic model or export authority: the appearance requirement is the same photograph in the file, potentially with better-resolved detail. Source inspection similarly acquires filtered linear AP1 from corrected camera light without inversion, applying only its display convention and transient inspection EV before pinned ACES. [Editor ownership](editor.md#a-stable-working-view-while-grading) and [diagnostics](diagnostics.md#stable-float32-view-grading) keep those distinctions explicit.

## Important boundaries

The current model is **4**. Negative calibration must be finite and satisfy `Dmin > Dmax >= input_floor > 0` per camera channel. Its scan sources share one compatible acquisition basis; Digital has no calibration and uses each source's camera matrix.

## Working on this area

- [Processing](../src/processing.rs): calibration, response, defaults, domains and pipeline order.
- [Colour](../src/color.rs): working-space matrices, balance coordinates and Bradford WB.
- [Photographic grading](../src/photographic.rs): contrast/saturation and stable signed evaluation.
- [Sampling](../src/sampling.rs): manual reference and neutral fitting.
- [GPU processing](../src/gpu.rs): the production implementation and preflight.
- [Local grades](../src/local_adjustments.rs) and [GPU grades](../src/gpu_local_adjustments.rs): ordered masks, weighted controls and snapshots.

The scalar implementation is the f64 reference. Exact equations, supported ranges and numerical gates live with the implementation and its tests: [camera inversion](../tests/camera_inversion.rs), [film response](../tests/film_response.rs), [photographic grading](../tests/photographic.rs) and [GPU processing](../tests/gpu_processing.rs). Start with the relevant [development checks](development.md#feature-specific-checks).