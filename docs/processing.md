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
  → optional emulation: image MTF → halation → grain
  → working positive, ready for rendering or EXR
```

[Colour and output](colour-and-output.md) explains what happens after the working positive. Display rendering is not part of negative inversion.

## Start with the camera's measurements

LibRaw develops non-DNG RAWs; the editor routes supported DNGs through Adobe's SDK. Both produce linear camera RGB, not sRGB or three isolated film dyes. Each saved source retains its own development identity and resolved WB baseline. Negative chooses unity WB, so the scanner's as-shot setting cannot change the inversion basis; Digital initially chooses each source's as-shot WB. Redevelopment reuses the accepted balance. The native developer still enforces its supported sensor domain.

Negative uses one shared camera basis for its RGB calibration. Digital admits mixed cameras and per-source WB. Input intent is accepted independently of source availability. [Captured acquisition](../src/source/input.rs) resolves the requested interpretation's development and supplies its physical pixels, camera conversion and starting exposure together. Recipes retain that captured development; consumers must use its resolved facts rather than the source's registered baseline. [Project compatibility](projects.md#compatibility-is-about-the-processing-basis) owns persistence and stage switching.

LibRaw supplies three-channel camera RGB from both CFA and non-CFA RAWs, including reduced-size Sony YCbCr. Standard Bayer sources use DHT's ratio-based reconstruction (quality 11), reducing green-texture transfer into weak channels in unbalanced scans. X-Trans retains three-pass interpolation (quality 3); other CFA layouts use LibRaw's fallback, and already-interpolated sources bypass demosaicing. Requested WB and the camera basis remain unchanged. Admission requires three camera colour channels and usable interpretation metadata, rather than a CFA pattern.

LibRaw undoes any declared capture-time WB before applying the requested balance. Its highlight-safe development divides the resulting effective gains by their maximum; for pre-balanced sources, the unused fourth RGB slot shares effective green. [Immutable source reads](../src/scan.rs) restore `max(effective WB) / green(effective WB)` in floating point before camera conversion, retaining above-one headroom and scaling the precision floor with the signal. CPU sampling, proxies, GPU views, repairs and exports share these samples. Direct SDK DNG keeps its own scale. This decoder correction leaves photographic exposure controls and deliberate capture exposure differences intact; camera/ISO exposure-reference calibration is a separate concern.

Negative inverts before the camera-to-working-space matrix: a matrix mixes channels, while inversion applies a different nonlinear response to each channel. An optional [flat-field map](flat-field.md) corrects scan illumination in these camera coordinates before inversion.

### Direct DNG development

The SDK backend defines the current bounded integer-Bayer domain: header and opcode inspection establish geometry, basis and precision without decoding every offscreen sensor, and required unsupported instructions reject — there is no uncorrected LibRaw fallback. One immutable developed camera-RGB source supplies all consumers. [Project reopening](projects.md#compatibility-is-about-the-processing-basis) keeps recorded development choices; acquisition checks current decoder observations when the source is needed.

### Early highlight handling

Both developers apply shared CPU camera-RGB Blend after LibRaw development or SDK demosaic, before geometric lens correction. Capture balance estimates colour independently of requested output WB and input mode. Missing capture balance, pre-balanced LibRaw clipping, or unsupported numerical headroom retain baseline development.

An original developed sample reaching decoded white admits the frame. In capture-balanced units, Blend compares chroma with a common-white-limited signal, mixes channels through quadratic shoulders, then progressively desaturates high lightness. A hue-direction bound keeps the result inside nonnegative camera light for negative inversion. This deliberately trades brightness and colour for smoother clipped highlights; it neither preserves the original RGB sum nor reconstructs missing detail. Below common balanced white the operation is identity, but genuine brighter colour can change even far from clipping. There is no neighbourhood map.

Blend can lift weaker channels above normalized sensor white. Before SDK warps, a temporary power-of-two scale protects that headroom and is restored at final output, raising the warps' effective clipping ceiling. Completed source pixels supply measurements, proxies and rendering alike. Decoder provenance and cache keys identify this pixel producer separately from older development.

### Embedded Sony lens correction

Sony RAW development automatically applies available embedded vignetting, distortion and lateral chromatic aberration before inversion or camera-to-AP1 conversion — no measured flat field required. Distortion/CA Off is a camera-rendering preference, not an unavailable profile; unavailable components are skipped, while malformed available data rejects development. Converted DNGs do not inherit Sony profiles.

The current binding reconstructs one conservative valid rectangle at unchanged pixel pitch from the active image geometry; distortion and red/blue CA scales combine at the radial knots before interpolation. Shading multiplies each original developed sample **before** bilinear reconstruction (`2^(0.5 - 2^(v/8192 - 1))` at the knots, interpolated then reciprocated) — this is the [darktable 5.6.2 v2](https://github.com/darktable-org/darktable/blob/release-5.6.2/src/iop/lens.cc) interpretation; [RawTherapee 5.12](https://github.com/RawTherapee/RawTherapee/blob/5.12/rtengine/lensmetadata.cc) instead interpolates reciprocal gains, agreeing at knots but not between them. Neither decoder inspection nor these public interpretations proves exact vendor output.

Corrected originals retain continuous f32 camera samples (fractions and values above native white included) and release LibRaw's redundant development buffer. Without optical correction, sources retain native integer storage unless early highlight handling produces changed float pixels. [The source-storage study](diagnostics.md#sony-source-storage-precision) distinguishes rounding differences from headroom loss. All consumers use the corrected developed raster rather than applying their own lens warp: [flat references](flat-field.md) estimate residual illumination in those coordinates, and [projects](projects.md#compatibility-is-about-the-processing-basis) reuse accepted WB with the current decoder.

## Digital starting brightness

Digital uses the embedded camera preview as a per-capture brightness reference, leaving Roll and layer exposure at zero. Source preparation resolves the fixed gain; captured interpretation includes it in the camera-to-AP1 transform, so tools, repairs, viewer, thumbnails and exports receive the same normalized baseline pixels. The native samples and recorded colour matrix remain unchanged; no additional full-image pass is needed. Roll and adjustment-layer grading follow that baseline.

The estimator compares interior luminance quantiles from an as-shot AP1 proxy before brightness normalization with the preview’s display-linear luminance. Declared sRGB and Adobe RGB previews use their own transfer curves and luminance coefficients, without an intermediate sRGB image; embedded ICC profiles retain colour-managed interpretation. Filmstrip display separately converts Adobe RGB to sRGB. A small [generated inverse neutral-ACES lookup](../assets/ocio/README.md#neutral-exposure-reference) translates display luminance into the current 100-nit rendering convention. This chooses roughly camera-like brightness without copying the camera's contrast, saturation or monochrome style. Photographer-set exposure compensation is not subtracted.

[Lazy source preparation](architecture.md#services-and-workers) resolves the value before publishing a processed positive, including cache hits and cold exports. Edits to crop, WB, exposure or look do not re-meter the capture. Missing usable colour/framing or insufficient nonclipped luminance produces an explicit unavailable result with zero starting EV; manual exposure remains available. [Projects](projects.md#save-accepted-choices-not-working-machinery) retain completed values separately from accepted controls.

## Two anchors define the response

The names Dmin and Dmax come from film density, but the stored values are **linear camera RGB measurements**, not optical densities:

- **Dmin** represents low-density film, usually film base. It transmits more light and anchors the positive at zero.
- **Dmax** represents a dense part of the negative. It transmits less light and anchors the positive at one.

Consequently, Dmin's measured value must be greater than Dmax's in each channel. The RGB anchors need not come from one jointly neutral pixel; their role is to establish each channel's response, not to declare a sampled object white.

Between the anchors, the curve works with relative log attenuation: how much less light a sample transmits than the Dmin reference. Gamma controls the response through that interval; toe and shoulder change its shape. Toe 1 with no shoulder removes that shaping; gamma still sets the curve's contrast between the anchors.

New grades and gamma resets start at **0.60**, a shared practical response prior rather than a measured film gamma: using one common value leaves stock/process contrast differences available. Analyze fits relative channel balance while preserving the selected global gamma. The anchors are not clipping limits: samples beyond film base retain signed outputs, and denser samples retain above-one highlights. Calibration and inversion use camera-light ratios independently of decoder quantization; sampling precision is a separate measurement estimate.

### Finite continuation beyond the calibrated range

For each channel, let `B` be Dmin, `W` be Dmax, `s` be transmitted camera light, `g` be gamma, `t` be Toe and `h = Shoulder / ln(10)`. The response uses density `l = ln(B/s)` and exponent `q = t*l/(g-h*l)`, normalized so `B` maps to zero and `W` to one. Signed `expm1(q)` with a `1/t` power gives the toe shape; with Toe 1 and Shoulder 0 this reduces to an anchored power ratio.

Two continuations make the response useful outside calibration. Before the shoulder denominator can vanish, at `l = g/(2*h)`, its exponent continues along the matching tangent `q = 4*t*l/g - t/h`. Value and slope agree at the join. At a positive response of `2^16`, the curve switches to its tangent in transmitted light, reaching a finite value at zero; nonpositive samples use that endpoint. Sixteen stops beyond the white anchor is a practical extrapolation choice, not a sensor-precision estimate or a clip at one. The joins and coefficients depend only on calibration and curve settings.

Coefficients are prepared once. Ordinary GPU pixels retain built-in f32 logs/exponentials; the continuations use branches and basic arithmetic, with no iterative solve or higher-precision GPU path. Scalar/GPU checks cover the anchors, matching slopes, zero light and camera-unit scaling.

## Curve balance is not white balance

Gamma balance changes the **relative shapes** of the three native-channel curves while preserving their geometric-mean gamma. Its colour effect depends on negative density, so it can correct a colour trend that differs between shadows and highlights.

Photographic WB instead acts on the interpreted positive. Antitone uses Bradford adaptation: AP1 is converted to a cone-response basis, adjusted, and converted back; warmth/tint are coordinates for that operation, not Kelvin measurements.

| Adjustment | Role |
| --- | --- |
| Development WB | Saved per source; initially unity for Negative or as-shot for Digital |
| Gamma balance | Changes the native-channel inversion curves |
| Roll WB | Shared photographic colour balance after inversion |
| Layer WB | Photographic colour balance at that layer's position after Roll WB |

Manual gamma-neutral fitting uses individually inverted, full-resolution samples; neutral means AP1 white, not equal camera channels. WB picks estimate a replacement at their own scope. The Photo pick measures after preceding layers and fits the selected layer's WB before its contrast/saturation, accounting for coverage, inversion, strength and exposure. An uncovered patch or correction outside the control range rejects. The Roll pick bypasses layer grading and Roll WB.

### Picked-colour gamma pivot

The **Gamma colour pivot** holds a picked area's chromaticity through manual gamma and gamma-balance edits by compensating Roll WB. Its measurement boundary is after Roll WB, before Photo layers, look and output; brightness remains free, and nonlinear downstream processing can still change rendered appearance. Deliberate Roll WB establishes the colour held by subsequent gamma gestures.

Capture freezes corrected original camera light, before inversion. Small selections retain exact source-cell coverage; large quadrilaterals use deterministic stratified evidence, bounded to 4096 weighted originals. Each proposed gamma individually inverts those frozen samples through the shared f32 GPU response before positive averaging. Camera-to-AP1 and Bradford conversion then give log-cone opponents; their difference is added to Roll WB. Undefined colour, source-floor evidence and out-of-range compensation reject the whole edit. A continuous gesture references its starting gamma/WB pair, allowing an exact return with no net edit.

## Grade in a common working space

After input interpretation, each source's metadata-derived camera matrix maps the positive to AP1/ACEScg with a D60 white. This gives grading and rendering a common basis — a pragmatic colour conversion, not proof that the original scene's spectra or film dyes have been recovered. Roll and layer exposure use stops; the working result retains signed and above-one values, so a negative AP1 component is not by itself a failed scan.

### Ungraded positive and repair placement

The ungraded positive is full-resolution signed/HDR AP1 after input interpretation and camera conversion, including fixed Digital brightness normalization, but before Roll or layer controls. Production repair inputs evaluate this composition on the original grid, with correction and interpretation resolved for the captured source.

The provisional Dust tool heals here. RAW-development WB, gamma balance and fixed Digital normalization are upstream; Roll and adjustment-layer controls are downstream. Calibration, response, correction or development changes legitimately require asynchronous reevaluation; downstream grading does not. Every independent repair reads the same unrepaired input and preserves signed/HDR values. [Repair nodes](../src/gpu/repairs/definition.rs) declare target and donor rectangles. Execution assembles CPU arrays independently of upstream tiling, masks unused target cells before validation, and runs the f64 solver in whole-repair units. Sparse GPU composition replaces only marked cells; completed units can be reused as ordinary images across downstream grades.

[Donor matching](../src/dust_repair/matching.rs) uses unmarked input context. Consistent structure can guide translations and gradient weighting, while full-detail colour matching remains active. The selected translation is stored separately from deterministic reconstruction.

Healing and nonlinear grading do not commute, so pre-grade donor matching can differ from matching graded input. Full-resolution repair composition precedes view filtering; saved repairs identify this stage through an explicit [project repair contract](projects.md#source-owned-dust-repairs). Viewer and TIFF/AP0 EXR consume those repairs; [navigation thumbnails](editor.md#positive-thumbnail-reuse) and measurement inputs omit them.

### One complete image recipe

[ImageRecipe](../src/image_recipe.rs) captures complete photographic intent: source/development, interpretation/correction, enabled repair instructions, ordered grading, MTF/grain, halation, composition and immutable LUT/Film & paper assets. Accepted-document and live-editor capture gather their intended values directly: accepted records, working edits, available exposure facts and unfinished repair instructions have distinct owners. Ordinary value fields become a shared snapshot through ownership, immutable borrows and `Arc`. Authoring another interpretation can clone and modify that value. Shared [source preparation](../src/photo_preparation.rs) resolves missing starting exposure and returns compatible physical inputs and a recipe in the same representation. A recorded reference takes precedence over transient preparation facts; execution consumers use the prepared capture. An uncalibrated negative can be captured; requesting a positive validates calibration. Navigation retains complete intent while graph quality chooses approximations. Output target and format remain consumer choices, including EXR's bypass of colour assets. Dust donor search reads the same unrepaired prefix through a [GPU source adapter](../src/gpu/repair_source.rs).

Request equality tracks complete captured intent, including looks; graph identities determine computational reuse. A changed look changes the photograph while preserving reusable upstream nodes. Explicit pre-look requests choose an endpoint without removing captured assets. Repair bookkeeping and disabled controls remain captured values rather than additional mathematical operation arguments.

### Input and grading have separate contracts

[Photographic graph assembly](../src/gpu/photograph/recipe.rs) translates complete intent and available camera resources into named `Value` endpoints. Graph construction captures repair instructions; [execution](../src/gpu/evaluation/context/unit.rs) owns readback and solving. Ordinary stage functions compose interpretation, repairs, grading, MTF, halation and grain; the graph owner places reconstruction for original samples, physical views or camera proxies. Proxy quality omits original-coordinate repairs. Recipe-backed consumers select endpoints and supply raster demand. The captured neutral recipe derives calibration and camera conversion for graph execution, CPU repair input and parameter validation. Measurements and repairs consume original-grid prefixes without downstream emulation; measurements bypass repairs. [`InputTransform`](../src/gpu/interpretation.rs) and [`Grading`](../src/gpu/grading.rs) also support directly authored calculations. Uploaded camera samples and mapping are immutable; separate correction choices can share them.

[Photographic calculations](../src/gpu/photograph.rs) expose corrected camera light independently of positive readiness, plus interpreted, neutral, Roll, graded and processed endpoints. Measurements and Analyze consume prefixes; inspection branches before inversion with its own viewing transform and distinct image meaning. [Stage functions](../src/gpu/photograph/assembly.rs) connect the [operations](../src/gpu/photograph/operations.rs) that own mathematics and upstream reads. [Acquisition](../src/gpu/view/acquisition.rs), [view processing](../src/gpu/view/processing.rs) and [original execution](../src/gpu/execution/originals.rs) consume Values and raster demand. Generic [evaluation](../src/gpu/evaluation.rs) plans dependencies, resources and production. Original, resident-camera and physical-view graphs retain their distinct sampling order [below](#physical-view-processing-is-an-approximation). WGSL bodies live beside their mathematical Rust owners. [Shared photographic functions](../src/gpu/photograph/shader.rs) assemble interpretation, correction and grading libraries from separate packed bindings.

[Values](../src/gpu/evaluation/value.rs) own immutable mathematical definitions, ordered operands and captured assets. Requests select coverage without replacing those definitions. An operation describes [unit-local Calculations](../src/gpu/evaluation/value/calculation.rs) with actual input windows; the compiler chooses their backend work units. Publication and reuse follow the Value's declared [Output](../src/gpu/evaluation/contract.rs), rather than the temporary instructions. Gaussian lowers to horizontal and vertical operations sharing a WGSL body; healing uses whole f64 CPU calculations followed by GPU replacement composition. Pointwise inputs arrive as pixels; neighborhood inputs have sampling functions. Tables carry masks and spatial indices; byte-copy concatenation joins replacement arguments. The compiler supplies bindings and output writes. Shared [Programs](../src/gpu/evaluation/execute.rs) retains parsed, validated and linked shader code alongside GPU pipelines; argument bindings remain per request. The compiler selects GPU functions or stored steps from these definitions, bounding expression expansion and declared per-pixel sampling work. The existing executor and materialization store consume both forms.

Stored arguments retain their complete upstream definitions and actual layouts. Image reuse follows captured content, upstream calculations, domain and sampling rather than shader bytes or output allocations; anonymous inputs require their actual shared definition. One [materialization store](../src/gpu/evaluation/materializations.rs) discovers host and GPU images independently of byte-bounded optional retention. Completed values share by calculation and layout; pending production additionally matches execution budgets so one caller's resource failure cannot reject a differently provisioned caller. Callable fusion hoists deferred calculations only when declared reads require their natural units; otherwise execution follows those reads. A worker claims unit calculations inline instead of queueing child work onto the blocking pool it occupies. Bounded [run-owned host values](../src/gpu/evaluation/working.rs) support reuse with optional retention disabled, without changing the solver's natural units. Eviction releases optional ownership without withdrawing another consumer's value. Recorded GPU results become discoverable after queue submission; an unsubmitted recording cannot make a worker wait for a paint.

[Tiled view reconstruction](../src/gpu/evaluation/reconstruction.rs) asks its input for content over the disjoint contributing source spans, using execution's conservative geometry. [Repair composition](../src/gpu/repairs/definition.rs) selects its own influencing masks through that ordinary description contract, keeping irrelevant donors unevaluated. Full captured intent still governs frame publication. Aligned units support unaffected-region reuse on compatible sampling grids; fresh viewport-relative grids can prevent overlap reuse. Sampling halos remain flexible demands. Complete frames and regional results share byte-bounded, recency-based retention; a small allowance can require recomputation of either. Views own live results independently of optional caching, while the common store can retain completed calculations across view retirement. Submission makes buffers discoverable without independently choosing optional ownership.

[Bounded area reconstruction](../src/gpu/evaluation/reconstruction/phases.rs) lowers to contribution, ordered weighted-combination and coverage-output expressions. Paired records preserve means, mass and flags without packing away numerical precision. The compiler assigns live records to reusable scratch slots and lowers operations to execution steps. Unit descriptions name their actual input Values; admission checks state, phase reads, compiler scratch and per-binding device limits before acquiring inputs. The same kernels record into caller-owned encoders or execute on workers, retaining physical owners through completion. Recorded steps can sample upstream functions instead of allocating an image for every thin read. Isolated step linkage keeps shader programs independent of preceding step count. Minimum working-state admission precedes graph expansion, and description storage is bounded. Oversized candidates split into smaller output units before acquisition or execution; semantic errors reject without subdivision. An irreducible aperture under a very small allowance can still reject. A larger allowance permits larger source parts without changing the requested output's identity.

Source reads follow contributing row spans with real-edge extension and packed-coordinate uncertainty, rather than acquiring entire rotated-footprint bounding rectangles. A fitting rectangular contribution can stay whole. Common execution assembles rectangular inputs, subdividing upstream transfers for resource admission. Related roots can interleave phases and share containing source images through ordinary materializations; round-owned handles retire after those submissions. GPU assembly batches ready copies, draining them at the working allowance and before calculating or waiting for another unit. Outputs, state and input assembly share the working allowance. Aggregate working-storage failures retry roots under their individual allowances.

### Photo contrast and saturation

Contrast scales a log-like ACEScct signal around a pivot; its Colour preservation blend reduces some contrast-driven colour changes without promising constant perceived saturation. Saturation blends between preserving AP1 luminance and anchoring the largest RGB channel, which gives the Density control its darkening character. Neither endpoint is a physical dye or print simulation.

### Adjustment layers change controls, not rendered pixels

Each layer has the same photographic controls and global, linear-gradient or radial-gradient coverage. Layers run bottom to top in the editor's list, applying exposure, Bradford WB, contrast and saturation in AP1 before rendering or geometric averaging. Global coverage is simply weight one; a whole-image finishing grade can follow a gradient, and nonlinear stages make order significant.

Coverage, inversion and strength produce a weight between zero and one: exposure and WB scale by it, while contrast and saturation interpolate toward identity. Thus half coverage of +1 EV means +0.5 EV — not a halfway blend of two output pixels. Masks are analytic gradients, not retained rasters; [geometry](geometry.md#source-anchored-local-masks) explains why composition and navigation must not move them. Grading consumes immutable snapshots. Control domains, packed constants and GPU resource sizes are validated before execution; extreme pixel arithmetic is bounded locally.

### Film texture

Rendering owns coupled image-MTF/grain and independently enabled halation after the complete adjustment-layer stack, before LUT, Film & paper and ACES. Recipe assembly orders source-scale Gaussian image MTF, independently enabled halation, then multiplicative RGB log-light grain. Each composes its mathematics and upstream reads; [common execution](architecture.md#pixel-stages-and-spatial-support) manages the supplied work and resources.

Size is a percentage of the accepted cropped frame's shorter side, measured in original-source pixels. A tighter crop therefore reduces the source-unit MTF blur and grain scale instead of magnifying an uncropped-frame blur. Exact turns/flips, viewport zoom, pan and output resolution do not redefine that reference. Size uses a fixed grain-basis unit; spectrum scale is 1.375 times that value. GrainSettings stores both lengths; its shared Size control scales them proportionally, preserving the authored image-response/grain relationship rather than adding independently scaled noise over the image.

The persistent source ID seeds the continuous grain field in original source coordinates. Amount is green log-light RMS of the continuous grain field before pixel integration. Averaging reduces its visible variance; magnification retains the original-pixel footprint. Neither the Amount reference nor the crop-relative spectrum depends on source resolution. Colour runs from shared monochrome through the fitted RGB covariance at its midpoint to stronger chroma. Mean compensation preserves expected AP1 light without tone-dependent strength compensation.

The Grain toggle controls MTF/grain, independently of Halation; both disabled produce the clean plate. Amount zero retains image MTF and omits the grain effect. A previously generated field may remain cached for reuse without executing grain. Controls are visible for Digital, while stored enabled settings can process other positive interpretations. Source inspection bypasses emulation. Immutable grain fields survive Amount, Colour and photographic edits; geometry, Size or seed changes require reevaluation. Cold generation grows with footprint relative to grain Size, so very fine grain in reduced views remains expensive; the current view-sized cache does not reuse overlapping fields across pan or zoom. [Saved contracts](projects.md#save-accepted-choices-not-working-machinery) preserve the chosen interpretation.

The **35mm** preset supplies the reference texture and halo values. **Medium format** uses a nominal 6×6 frame: its spatial lengths are multiplied by 24/56, while grain Amount/Colour and halo Strength stay unchanged. This is a geometric appearance starting point, not a stock profile. Finer relative grain naturally loses more visible contrast through pixel averaging. Preset selection replaces actual values while preserving each effect's enable state; subsequent controls keep the same units and ranges, rather than percentages of the chosen preset.

### Halation

Halation redistributes positive AP1 light through a unit-sum Gaussian after completed image MTF and before grain, LUT, Film & paper and ACES. It runs independently of MTF/grain enablement. Its own Core size is Gaussian sigma as a percentage of the accepted crop's shorter side; grain Size does not change it. Both settings groups use Digital-only control visibility while allowing stored enabled settings on other positive interpretations. Source inspection bypasses both.

For each channel, `P = max(E, 0)` generates the halo and `H = E + alpha * (blur(P) - P)`, with `alpha = k / (1 + k)`. Strength is the red coefficient `k`; green uses 0.56 times it and blue is retained without redistribution. The starting values, Core size 0.109% and Strength 0.54, approximate a warm fringe merging into the highlight when composed with the 35mm reference image MTF. They redistribute about 35% of red and 23% of green light, adding channel-dependent spatial attenuation to the image MTF. The complete response is not a single Gaussian blur. These AP1 coefficients describe an appearance choice, not recovered film-layer sensitivities. The signed original remains in the result; negative neighbours can receive positive light but cannot generate it. Constant fields stay unchanged, while structured neutral edges can gain red/orange surrounds and complementary core shifts. There is no highlight threshold or clipped-highlight reconstruction.

The kernel uses separable, four-sigma-truncated discrete taps with constant extension at actual source edges and real neighbours across crop, tile and viewport boundaries. Physical views use the same effect at their declared pitch, retaining the [view approximation](#physical-view-processing-is-an-approximation); original references and exports process before reconstruction.

### One f32 photographic implementation

GPU source conversion, illumination correction, inversion, camera matrices, repair composition and photographic grades use ordinary f32 arithmetic. Preview, export, photographic measurements, candidate responses and repair inputs share those operators. Signed and above-one working values remain available to downstream stages; area reconstruction separately accumulates original-cell contributions.

Additional precision carries compilation and execution costs on every photograph. It needs evidence of a meaningful difference in real photographic output, rather than disagreement with f64 in a constructed cancellation case. CPU statistics, fitting and repair reconstruction retain f64 arithmetic, consuming shared f32 forward pixels. GPU fitters account for their forward model's residual resolution while retaining conditioning checks. [Photographic diagnostics](diagnostics.md#photographic-arithmetic) produce paired rendered files for that assessment.

Continuous signed contrast and the film-base continuation retain explicit zero-input limits. Interpretation, grading and multiplicative grain bound extreme arithmetic locally, leaving factor-four f32 headroom for subsequent sums and interpolation. Exponentials saturate before overflow; overflowing matrix products retry on normalized operands. Ordinary signed/HDR values retain their range for grading and [output rendering](colour-and-output.md#render-for-a-declared-target), rather than being clipped to display white. Repair magnitude does not veto grading or omit a requested patch. Film anchors must remain distinct after f32 packing, and a representable Dmax sample retains its unit response even with large response constants.

### Physical-view processing is an approximation

The positive editor prototype factors Roll exposure/WB and adjustment layers downstream of baseline AP1 production: original camera samples still receive continuous correction and calibrated inversion before exact affine-footprint filtering into signed/HDR f32 AP1, and fixed-view grading reuses that filtered input rather than processing the whole original again.

Physical views separate quality selection from dependency geometry. Effects supply a preferred pitch; acquisition subdivides presentation cells integrally, stopping at source resolution and bounding refinement including neighbours to roughly five million cells (larger presentations retain their requested grid). Gaussian effects currently aim for sigma of at least 0.6 working cells. These are engineering quality/cost choices: a bounded view can remain under-resolved. [Acquisition](../src/acquisition/graph.rs) on original and frame grids composes the same ordered effect definitions used for rendering and queries their declared unit reads. Its rectangular input envelope includes natural-unit expansion and asymmetric neighbours; reuse checks every side of that window. These queries use the `gpu` feature without acquiring pixels or device resources.

Grading, completed signed MTF and positive halation run before area reduction; footprint-integrated grain follows on presentation cells, then LUT, Film & paper and ACES. Grain's Gaussian footprint approximation omits signal–grain correlation within a pixel. Original references and export apply the complete photograph, including rendering, before source-cell reconstruction. Real neighbours extend beyond the viewport at the acquired pitch; viewport edges are not source edges.

Filtering before nonlinear grading does **not** commute with original-sample grading before reconstruction, and footprint-centre local masks are not the original-sample mask integral. These are preview approximations, not changes to the photographic model or export authority: the appearance requirement is the same photograph in the file, potentially with better-resolved detail. Source inspection similarly acquires filtered linear AP1 from corrected camera light without inversion, applying its [viewing WB and exposure](colour-and-output.md#source-inspection-has-its-own-exposure-convention) before pinned ACES. [Editor ownership](editor.md#a-stable-working-view-while-grading) and [diagnostics](diagnostics.md#stable-float32-view-grading) keep those distinctions explicit.

## Important boundaries

The current model is **6**. Negative calibration must be finite and satisfy `Dmin > Dmax > 0` per camera channel. Its scan sources share one compatible acquisition basis; Digital has no calibration and uses each source's camera matrix.

## Working on this area

- [Processing](../src/processing.rs): calibration, input interpretation and Roll adjustments.
- [Colour](../src/color.rs): working-space matrices, balance coordinates and Bradford WB.
- [Photographic grading](../src/photographic.rs): contrast/saturation and stable signed evaluation.
- [Sampling](../src/sampling.rs): manual reference and neutral fitting.
- [GPU calculations](../src/gpu/photograph.rs): photographic graph construction and operations.
- [Adjustment layers](../src/adjustment_layers.rs) and [GPU grading](../src/gpu/grading.rs): coverage, shared control settings and complete ordered snapshots.

GPU photographic components define the rendered pixels; CPU calculations prepare parameters and perform statistics, fitting and healing. Test expectations need their own justification: a second renderer establishes agreement, not photographic correctness. Start with the [test guidance](development.md#meaningful-tests) and relevant [development checks](development.md#feature-specific-checks).