# From camera RGB to a graded photograph

An input stage interprets developed camera RGB before shared photographic grading. **Negative** inverts light transmitted through film using measured calibration: denser film becomes a brighter positive. **Digital** treats the RAW as a scene-referred positive, without density inversion or anchors. [Roll analysis](roll-analysis.md) chooses starting calibration and grading for Negative.

The processing pipeline is:

```text
developed camera RGB
  → Negative: optional scan-illumination correction and per-channel inversion
    Digital: scene-referred positive
  → camera-to-AP1 conversion, including fixed Digital brightness normalization
  → normalized, ungraded working positive (optional source-owned dust repair here)
  → Roll exposure and photographic white balance
  → ordered adjustment layers: exposure, white balance, contrast and saturation
  → optional Film Emulation: image MTF → grain
  → working positive, ready for rendering or EXR
```

[Colour and output](colour-and-output.md) explains what happens after the working positive. Display rendering is not part of negative inversion.

## Start with the camera's measurements

LibRaw develops non-DNG RAWs; the editor routes supported DNGs through Adobe's SDK. Both produce linear camera RGB, not sRGB or three isolated film dyes. Each saved source retains its own development identity. Negative always develops with unity WB, so the scanner's as-shot setting cannot change the inversion basis; Digital develops each source with its as-shot WB. The native developer still enforces its supported sensor domain.

Negative uses one shared scan basis: the camera interpretation and developer contract must match, while WB is determined by the stage rather than stored in that basis. Digital admits mixed cameras and per-source as-shot WB. Crossing input stages redevelops active sources for the target stage and updates their recorded identities. [Project compatibility](projects.md#compatibility-is-about-the-processing-basis) owns admission and switching.

The accepted domain is three-colour integer Bayer without repeated black-level patterns: Sony ARW, Canon CR2/CR3 (including CRAW), Nikon NEF, Olympus ORF and Panasonic RW2 develop through it. LibRaw recognizes further formats (X-Trans, decimated sRAW/mRAW, others); broader compatibility is future work.

LibRaw's highlight-safe development divides WB gains by their maximum. [Immutable source reads](../src/scan.rs) restore `max(WB) / green(WB)` in floating point before camera conversion, retaining above-one headroom and scaling the precision floor with the signal. CPU sampling, proxies, GPU views, repairs and exports share these samples. Unity-WB Negative and direct SDK DNG keep their existing scale. This decoder correction leaves photographic exposure controls and deliberate capture exposure differences intact; camera/ISO exposure-reference calibration is a separate concern.

Negative inverts before the camera-to-working-space matrix: a matrix mixes channels, while inversion applies a different nonlinear response to each channel. An optional [flat-field map](flat-field.md) corrects scan illumination in these camera coordinates before inversion.

### Direct DNG development

The SDK backend defines the current bounded integer-Bayer domain: header and opcode inspection establish geometry, basis and precision without decoding every offscreen sensor, and required unsupported instructions reject — there is no uncorrected LibRaw fallback. One immutable source supplies all consumers: the original pre-opcode CFA remains capture authority, with its white/black/repeated-border flags propagated through repairs and warps. [Saved SDK compatibility](projects.md#compatibility-is-about-the-processing-basis) is separate from source-file freshness.

### Embedded Sony lens correction

Sony RAW development automatically applies available embedded vignetting, distortion and lateral chromatic aberration before inversion or camera-to-AP1 conversion — no measured flat field required. Distortion/CA Off is a camera-rendering preference, not an unavailable profile; unavailable components are skipped, while malformed available data rejects development. Converted DNGs do not inherit Sony profiles.

The current binding reconstructs one conservative valid rectangle at unchanged pixel pitch from the active image geometry; distortion and red/blue CA scales combine at the radial knots before interpolation. Shading multiplies each original developed sample **before** bilinear reconstruction (`2^(0.5 - 2^(v/8192 - 1))` at the knots, interpolated then reciprocated) — this is the [darktable 5.6.2 v2](https://github.com/darktable-org/darktable/blob/release-5.6.2/src/iop/lens.cc) interpretation; [RawTherapee 5.12](https://github.com/RawTherapee/RawTherapee/blob/5.12/rtengine/lensmetadata.cc) instead interpolates reciprocal gains, agreeing at knots but not between them. Neither decoder inspection nor these public interpretations proves exact vendor output.

Corrected originals retain continuous f32 camera samples (fractions and values above native white included) and release LibRaw's redundant RGB16 development; absent or all-zero profiles keep the native integer source. Corrected clipping flags let picks, Analyze and flat-field estimation check eligibility without tracing geometry. [The source-storage study](diagnostics.md#sony-source-storage-precision) distinguishes rounding differences from headroom loss. All consumers use the corrected developed raster rather than applying their own lens warp: [flat references](flat-field.md) require matching lens development, and [projects](projects.md#compatibility-is-about-the-processing-basis) retain its identity.

## Digital starting brightness

Digital uses the embedded camera preview as a per-capture brightness reference, leaving Roll and layer exposure at zero. Source preparation folds the fixed gain into the camera-to-AP1 transform, so tools, repairs, viewer, thumbnails and exports receive the same normalized baseline pixels. The native samples and recorded colour matrix remain unchanged; no additional full-image pass is needed. Roll and adjustment-layer grading follow that baseline.

The estimator compares interior luminance quantiles from an as-shot AP1 proxy before brightness normalization with a colour-managed preview. A small [generated inverse neutral-ACES lookup](../assets/ocio/README.md#neutral-exposure-reference) translates display luminance into the current 100-nit rendering convention. This chooses roughly camera-like brightness without copying the camera's contrast, saturation or monochrome style. Photographer-set exposure compensation is not subtracted.

[Lazy source preparation](architecture.md#services-and-workers) resolves the value before publishing a processed positive, including cache hits and cold exports. Edits to crop, WB, exposure or look do not re-meter the capture. Missing usable colour/framing or insufficient nonclipped luminance produces an explicit unavailable result with zero starting EV; manual exposure remains available. [Projects](projects.md#save-accepted-choices-not-working-machinery) retain completed values separately from accepted controls.

## Two anchors define the response

The names Dmin and Dmax come from film density, but the stored values are **linear camera RGB measurements**, not optical densities:

- **Dmin** represents low-density film, usually film base. It transmits more light and anchors the positive at zero.
- **Dmax** represents a dense part of the negative. It transmits less light and anchors the positive at one.

Consequently, Dmin's measured value must be greater than Dmax's in each channel. The RGB anchors need not come from one jointly neutral pixel; their role is to establish each channel's response, not to declare a sampled object white.

Between the anchors, the curve works with relative log attenuation: how much less light a sample transmits than the Dmin reference. Gamma controls the response through that interval; toe and shoulder change its shape. Toe 1 with no shoulder removes that shaping; gamma still sets the curve's contrast between the anchors.

New grades and gamma resets start at **0.60**, a shared practical response prior rather than a measured film gamma: using one common value leaves stock/process contrast differences available. Analyze fits relative channel balance while preserving the selected global gamma. The anchors are not clipping limits — samples can lie beyond them, and the input precision floor prevents interpreting values below the developed scan's precision as reliable density.

## Curve balance is not white balance

Gamma balance changes the **relative shapes** of the three native-channel curves while preserving their geometric-mean gamma. Its colour effect depends on negative density, so it can correct a colour trend that differs between shadows and highlights.

Photographic WB instead acts on the interpreted positive. Antitone uses Bradford adaptation: AP1 is converted to a cone-response basis, adjusted, and converted back; warmth/tint are coordinates for that operation, not Kelvin measurements.

| Adjustment | Role |
| --- | --- |
| Development WB | Unity for Negative; each source's as-shot for Digital |
| Gamma balance | Changes the native-channel inversion curves |
| Roll WB | Shared photographic colour balance after inversion |
| Layer WB | Photographic colour balance at that layer's position after Roll WB |

Manual gamma-neutral fitting uses individually inverted, full-resolution samples; neutral means AP1 white, not equal camera channels. WB picks estimate a replacement at their own scope. The Photo pick measures after preceding layers and fits the selected layer's WB before its contrast/saturation, accounting for coverage, inversion, strength and exposure. An uncovered patch or correction outside the control range rejects. The Roll pick bypasses layer grading and Roll WB.

## Grade in a common working space

After input interpretation, each source's metadata-derived camera matrix maps the positive to AP1/ACEScg with a D60 white. This gives grading and rendering a common basis — a pragmatic colour conversion, not proof that the original scene's spectra or film dyes have been recovered. Roll and layer exposure use stops; the working result retains signed and above-one values, so a negative AP1 component is not by itself a failed scan.

### Ungraded positive and repair placement

The ungraded positive is full-resolution signed/HDR AP1 after input interpretation and camera conversion, including fixed Digital brightness normalization, but before Roll or layer controls. Production repair inputs evaluate this composition on the original grid, with correction and interpretation resolved for the captured source.

The provisional Dust tool heals here. RAW-development WB, gamma balance and fixed Digital normalization are upstream; Roll and adjustment-layer controls are downstream. Calibration, response, correction or development changes legitimately require asynchronous reevaluation; downstream grading does not. Every independent repair reads the same unrepaired input and preserves signed/HDR values.

[Donor matching](../src/dust_repair/matching.rs) uses unmarked input context. Consistent structure can guide translations and gradient weighting, while full-detail colour matching remains active. The selected translation is stored separately from deterministic reconstruction.

Healing and nonlinear grading do not commute, so pre-grade donor matching can differ from matching graded input. Full-resolution repair composition precedes view filtering; saved repairs identify this stage through an explicit [project repair contract](projects.md#source-owned-dust-repairs). Viewer and TIFF/AP0 EXR consume those repairs; [navigation thumbnails](editor.md#positive-thumbnail-reuse) and measurement inputs omit them.

### Input and grading have separate contracts

[`GpuInputProgram`](../src/gpu/input_program.rs) prepares source interpretation and camera conversion; [`Grading`](../src/gpu/grading.rs) owns captured Roll/layer controls, prepared resources and grading operations on working positives. [`ProcessingInput`](../src/gpu/input.rs) captures these independently, including the original extent for spatial admission.

The [ordered photographic definition](../src/gpu/photograph.rs) drives both component admission and [image assembly](../src/gpu/evaluation/photograph/assembly.rs). Full photographs and requested prefixes follow that definition; source adapters supply captured settings and coordinate mappings rather than separate upstream operation lists. Recognized folded camera implementations preserve their arithmetic and representable range without requiring each logical boundary to become a buffer.

### One accepted image recipe

[Full image recipes](../src/image_recipe.rs) capture accepted source/development, input interpretation and active correction, committed enabled repairs, ordered adjustment layers, Film Emulation, composition and the immutable creative-look asset. Navigation recipes retain grade/emulation/look/composition but omit repair payload. Numeric look identity affects rendering, not neutral healing or graded-AP1 reuse. Output target and format are consumer choices, including EXR's explicit look bypass; metadata-only repair IDs, look labels and disabled instructions do not change photographic identity. Interactive and offscreen repairs use a [shared GPU source](../src/gpu/repair_source.rs) to supply bounded crops to the CPU matching and healing algorithms.

### Photo contrast and saturation

Contrast scales a log-like ACEScct signal around a pivot; its Colour preservation blend reduces some contrast-driven colour changes without promising constant perceived saturation. Saturation blends between preserving AP1 luminance and anchoring the largest RGB channel, which gives the Density control its darkening character. Neither endpoint is a physical dye or print simulation.

### Adjustment layers change controls, not rendered pixels

Each layer has the same photographic controls and global, linear-gradient or radial-gradient coverage. Layers run bottom to top in the editor's list, applying exposure, Bradford WB, contrast and saturation in AP1 before rendering or geometric averaging. Global coverage is simply weight one; a whole-image finishing grade can follow a gradient, and nonlinear stages make order significant.

Coverage, inversion and strength produce a weight between zero and one: exposure and WB scale by it, while contrast and saturation interpolate toward identity. Thus half coverage of +1 EV means +0.5 EV — not a halfway blend of two output pixels. Masks are analytic gradients, not retained rasters; [geometry](geometry.md#source-anchored-local-masks) explains why composition and navigation must not move them. Grading consumes immutable snapshots; full-stack preflight rejects nonfinite constants and overflowing intermediates rather than clamping.

### Film Emulation

Rendering owns project-wide Film Emulation after the complete ordered adjustment-layer stack, before the creative look and ACES. Recipe assembly supplies independent source-scale Gaussian image-MTF and multiplicative RGB log-light grain effects in that order. Each declares its support and GPU implementation; [common execution](architecture.md#pixel-stages-and-spatial-support) manages the supplied work and resources.

Size is a percentage of the accepted cropped frame's shorter side, measured in original-source pixels. A tighter crop therefore reduces the source-unit MTF blur and grain scale instead of magnifying an uncropped-frame blur. Exact turns/flips, viewport zoom, pan and output resolution do not redefine that reference. The persistent source ID seeds the continuous grain field in original source coordinates. Amount is green log-light RMS at a native source-pixel footprint; reduction loses variance, while magnification keeps that native footprint. Colour runs from shared monochrome through the fitted RGB covariance at its midpoint to stronger chroma. Mean compensation preserves expected AP1 light without tone-dependent strength compensation.

Disabling emulation produces the clean plate; Amount zero retains image MTF and omits the grain effect. A previously generated field may remain cached for reuse without executing grain. Controls are visible for Digital, while stored enabled settings can process other positive interpretations. Source inspection bypasses emulation. Immutable grain fields survive Amount, Colour and photographic edits; geometry, Size or seed changes require reevaluation. Cold generation grows with footprint relative to grain Size, so very fine grain in reduced views remains expensive; the current view-sized cache does not reuse overlapping fields across pan or zoom. [Saved contracts](projects.md#save-accepted-choices-not-working-machinery) preserve the chosen interpretation.

### One f32 photographic implementation

GPU source conversion, illumination correction, inversion, camera matrices, repairs and photographic grades use ordinary f32 arithmetic. Preview, export, photographic measurements, candidate responses and repair inputs share those operators. Signed and above-one working values remain available to downstream stages; area reconstruction separately accumulates original-cell contributions.

Additional precision carries compilation and execution costs on every photograph. It needs evidence of a meaningful difference in real photographic output, rather than disagreement with f64 in a constructed cancellation case. CPU statistics, fitting and repair reconstruction retain f64 arithmetic, consuming shared f32 forward pixels. GPU fitters account for their forward model's residual resolution while retaining conditioning checks. [Photographic diagnostics](diagnostics.md#photographic-arithmetic) produce paired rendered files for that assessment.

Continuous signed contrast and the film-base continuation retain explicit zero-input limits. The shaders otherwise use built-in f32 exponentials and logarithms. Preflight checks representable constants and finite intermediate ranges, including response evaluation after f32 packing.

### Physical-view processing is an approximation

The positive editor prototype factors Roll exposure/WB and adjustment layers downstream of baseline AP1 production: original camera samples still receive continuous correction and calibrated inversion before exact affine-footprint filtering into signed/HDR f32 AP1, and fixed-view grading reuses that filtered input rather than processing the whole original again.

View-space film filtering also approximates original-sample processing: grain uses a Gaussian footprint approximation, while authoritative export and original references apply emulation before source-cell reconstruction. Physical views acquire the stages' declared neighbourhood beyond the viewport and extract the requested raster after processing; viewport edges are not source edges.

Filtering before nonlinear grading does **not** commute with original-sample grading before reconstruction, and footprint-centre local masks are not the original-sample mask integral. These are preview approximations, not changes to the photographic model or export authority: the appearance requirement is the same photograph in the file, potentially with better-resolved detail. Source inspection similarly acquires filtered linear AP1 from corrected camera light without inversion, applying its [viewing WB and exposure](colour-and-output.md#source-inspection-has-its-own-exposure-convention) before pinned ACES. [Editor ownership](editor.md#a-stable-working-view-while-grading) and [diagnostics](diagnostics.md#stable-float32-view-grading) keep those distinctions explicit.

## Important boundaries

The current model is **5**. Negative calibration must be finite and satisfy `Dmin > Dmax >= input_floor > 0` per camera channel. Its scan sources share one compatible acquisition basis; Digital has no calibration and uses each source's camera matrix.

## Working on this area

- [Processing](../src/processing.rs): calibration, input interpretation and Roll adjustments.
- [Colour](../src/color.rs): working-space matrices, balance coordinates and Bradford WB.
- [Photographic grading](../src/photographic.rs): contrast/saturation and stable signed evaluation.
- [Sampling](../src/sampling.rs): manual reference and neutral fitting.
- [GPU processing](../src/gpu.rs): the production implementation and preflight.
- [Adjustment layers](../src/adjustment_layers.rs) and [GPU grading](../src/gpu/grading.rs): coverage, shared control settings and complete ordered snapshots.

GPU photographic components define the rendered pixels; CPU calculations prepare parameters and bounds and perform statistics, fitting and healing. Test expectations need their own justification: a second renderer establishes agreement, not photographic correctness. Start with the [test guidance](development.md#meaningful-tests) and relevant [development checks](development.md#feature-specific-checks).