# From a negative to a positive

A camera scan records light passing through film. Dense parts of the negative transmit less light, but should become brighter parts of the positive. Antitone's processing model describes that transformation **given a calibration and grade**. [Roll analysis](roll-analysis.md) is a separate problem: choosing a useful starting calibration and grade from a set of scans.

The processing pipeline is:

```text
developed camera RGB
  → optional scan-illumination correction
  → per-channel negative inversion
  → camera-to-AP1 colour conversion
  → ungraded working positive (optional source-owned dust repair here)
  → exposure and photographic white balance
  → Photo contrast and saturation
  → ordered source-anchored local grades, when present
  → working positive, ready for rendering or EXR
```

[Colour and output](colour-and-output.md) explains what happens after the working positive. Display rendering is not part of negative inversion.

## Start with the camera's measurements

LibRaw develops the RAW into linear camera RGB using a fixed roll white balance. These are the camera's three channels, not sRGB and not three isolated film dyes. Keeping development fixed means a roll's calibration refers to the same kind of measurement in every photo.

LibRaw recognizes more RAW formats than Antitone's developer accepts. This path currently needs three-colour integer Bayer data without repeated black-level patterns; X-Trans, Foveon and linear DNG do not fit it. Both photo and reference pickers offer common camera RAW extensions, but decoded contents, the camera matrix and WB—not the filename—determine whether a source can be used. [Broader LibRaw compatibility](plans/raw-compatibility.md) is future work.

The first scan selects its as-shot WB for the roll; later loads keep that choice. LibRaw can leave the second green site's gain unspecified on a three-colour Bayer source. Automatic selection shares the green gain in that case without changing raw metadata or repairing other invalid gains. Explicit WB, including a saved roll's WB, is used as supplied and must pass validation.

Inversion must happen before the camera-to-working-space matrix. A matrix mixes channels; inversion applies a different nonlinear response to each channel. Mixing first and inverting afterward would produce a different model. This is the reason for the [native-camera boundary](decisions/native-camera-inversion.md), not simply an implementation preference.

An optional [flat-field map](flat-field.md) corrects scan illumination in these same camera coordinates before inversion.

## Two anchors define the response

The names Dmin and Dmax come from film density, but the stored values are **linear camera RGB measurements**, not optical densities:

- **Dmin** represents low-density film, usually film base. It transmits more light and anchors the positive at zero.
- **Dmax** represents a dense part of the negative. It transmits less light and anchors the positive at one.

Consequently, Dmin's measured value must be greater than Dmax's in each channel. The RGB anchors need not come from one jointly neutral pixel. Their role is to establish each channel's response, not to declare a sampled object white.

Between the anchors, the curve works with relative log attenuation: how much less light a sample transmits than the Dmin reference. Gamma controls the response through that interval; toe and shoulder change its shape. New grades use the adjustable response. The classic response is another shape within the same native-camera model, not a compatibility path for old AP0 grades. Displaying or resetting controls does not convert a classic grade; **Use adjustable response** is the explicit opt-in.

New grades and gamma resets start at **0.60**, a shared practical response prior rather than a measured population-mean film gamma. Using one common value leaves stock/process contrast differences available instead of equalizing each stock. Lower gamma generally increases tonal separation. Analyze fits relative channel balance while preserving the selected global gamma; saved projects retain their explicit value, so a default change does not regrade them.

The anchors are not clipping limits. Samples can lie beyond them. Above-base noise has a defined signed continuation, and a dense sample can produce a value above one. The input precision floor prevents interpreting values below the developed scan's precision as reliable density; it is not a hidden brightness pedestal or a measured noise floor.

## Curve balance is not white balance

Gamma balance changes the **relative shapes** of the three native-channel curves while preserving their geometric-mean gamma. Its colour effect depends on negative density. It can therefore correct a colour trend that differs between shadows and highlights.

Photographic WB acts on the inverted positive. Antitone uses Bradford adaptation: it converts AP1 to a cone-response basis, adjusts relative gains there, and converts back. Warmth/tint are coordinates for that operation, not Kelvin measurements or direct AP1 RGB multipliers.

There are four distinct ideas here:

| Adjustment | Role |
| --- | --- |
| Fixed scan WB | Establishes the RAW development basis for the roll |
| Gamma balance | Changes the native-channel inversion curves |
| Roll WB | Shared photographic colour balance after inversion |
| Photo WB | Individual photographic colour balance after Roll WB |

Manual gamma-neutral fitting uses individually inverted, full-resolution samples; it does not invert an average negative. Neutral means AP1 white, not equal camera channels: the fitter works back through the camera matrix to find its native-channel target. WB picks estimate a replacement at their own scope: the Photo pick measures after Roll WB, while the Roll pick bypasses both photographic WB settings. Neither pick resets unrelated grading.

## Grade in a common working space

After inversion, the metadata-derived camera matrix maps the positive to AP1/ACEScg with a D60 white. This gives grading and rendering a common basis. It is a pragmatic positive colour conversion, not proof that the original scene's spectra or film dyes have been recovered.

Roll and Photo exposure add in stops. Roll WB is followed by Photo WB, then Photo contrast and saturation. The working result retains signed and above-one values. A negative AP1 component can result from colour conversion or grading; it is not, by itself, a failed scan.

### Ungraded positive and repair placement

The ungraded positive is full-resolution signed/HDR AP1 after scan correction, calibrated inversion and camera conversion, but before photographic Roll or Photo controls. `RollSettings::inversion_only` explicitly separates response/gamma/balance from exposure/WB; adding a future Roll field requires a stage-placement decision rather than silently changing repair input. `Pipeline::ungraded` is the scalar boundary, shared semantically with neutral GPU view acquisition.

The provisional Dust tool heals here. Fixed RAW-development WB and gamma balance are upstream, unlike photographic Roll WB. Calibration, response, correction or development changes legitimately require asynchronous reevaluation; exposure, photographic WB, contrast, saturation and downstream locals do not. This is an interpretation-stable working positive, not proof of recovered scene-linear truth. Every independent repair reads the same unrepaired input, preserves known f32 values and never clamps signed/HDR data.

GPU donor screening reduces matching work without replacing the healer's f64 score, competitive margin or tie breaks. Integer lower bounds may exclude only candidates proved outside that margin; every possible winner retains exact CPU scoring and reconstruction. Quantized screening coordinates and floored weights are evidence for rejection, not quantization or clipping of AP1 samples, repaired pixels or final scores. [The research guide](research/dust-removal.md#large-mask-acceleration) explains the conservative construction and its measured limits.

Healing and nonlinear grading do not commute, and pre-grade donor matching can differ from matching graded input. The solver remains frozen; [research evidence](research/dust-removal.md) is not all-grade photographic acceptance. Full-resolution repair composition precedes view filtering or original-sample output reconstruction; it is not painting into an already downsampled proxy. Saved repairs identify this stage through an explicit [project repair contract](projects.md#source-owned-dust-repairs). Viewer/reference/scopes, TIFF/AP0 EXR and proper positive thumbnails consume these repairs. Camera navigation placeholders and measurement inputs have separate purposes; they are not completed repaired positives.

### One accepted image recipe

[Image recipes](../src/image_recipe.rs) capture accepted source/development, correction, calibration/inversion, committed enabled repairs, Global/ordered-local grades and composition. Output target and format are consumer choices, not alternate grades. Explicit viewer drafts reuse the same construction without changing accepted records; export and proper cards never capture a gesture draft. Metadata-only repair IDs/provenance and disabled instructions do not change photographic identity, while source freshness remains the acquisition owner's responsibility.

[Source-aware resolution](../src/repair_source.rs) shares the frozen scalar neutral sampling boundary across interactive and offscreen work. A cold consumer recomputes without requiring the active viewer cache; cancellation is not a failed heal. Export checks exact composition coverage before excluding an irrelevant support. Evaluated sparse content can remain CPU-owned while each bounded original unit uploads only its contributing pixels. Reconstruction never substitutes an unrepaired success because a GPU patch is absent.

Dmin/Dmax, gamma/WB fitting, Analyze statistics and source inspection retain their existing unrepaired numerical contracts. Positive picker/reference presentation includes repairs; its measurement input is deliberately separate. Thumbnail and physical-view filtering before nonlinear grading remains the explicit approximation below, not a second photographic definition.

### Photo contrast and saturation

Contrast scales a log-like ACEScct signal around a pivot. Its Colour preservation blend moves from independent RGB contrast toward a black-relative colour-direction proxy with the same neutral tone response. That reduces some contrast-driven colour changes; it does not promise constant perceived saturation.

Saturation blends between preserving AP1 luminance and anchoring the largest RGB channel. The latter gives the Density control its darkening character on positive colours. Neither endpoint is a physical dye or print simulation. Contrast and saturation amounts of one are exact identities, including when their secondary controls are non-default.

See the [grading rationale](decisions/photographic-grading.md) for why these operators support signed working data.

### Local grades change controls, not rendered pixels

A linear or radial gradient determines how much of a local photographic adjustment reaches each original source pixel. Global Roll/Photo grading runs first; ordered locals then apply exposure, Bradford WB, contrast and saturation in AP1 before rendering or geometric averaging. Their order matters across nonlinear stages. Locals do not change inversion, calibration, gamma balance, correction or the output target.

Coverage, inversion and strength produce a weight between zero and one. Exposure and WB amounts scale by that weight; contrast and saturation interpolate their amounts toward identity. Thus half coverage of +1 EV means +0.5 EV—not a halfway blend of two output pixels. Pivot and the secondary colour/density controls remain stage settings. Empty, disabled, identity and zero-coverage stages bypass exactly, preserving signed/HDR values and camera-floor flags.

Masks are analytic cubic-smoothstep gradients, not retained mask rasters. Linear endpoints define zero/full boundaries; radial centre, radii and angle define an ellipse with a feathered boundary. [Geometry](geometry.md#source-anchored-local-masks) explains why composition and navigation must not move them.

Original GPU sources may concatenate two validated full-resolution rectangles for bounded export work. Both members retain their own integer source coordinates before correction and masks; this is not nearest-centre sampling of a fictitious combined rectangle. Grade admission checks both members. Ordinary single-raster nearest-centre sampling remains unchanged. The [output guide](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns batching, lifetime and reconstruction guarantees.

The scalar spatial pipeline and GPU grades consume immutable snapshots. Full-stack preflight propagates continuous-weight intermediate bounds forward and observable-signal sensitivity backward, rejecting both overflow and unsupported attenuation/re-amplification. The next stage receives the proven output envelope, not a scratch peak. Compensated constants/temporaries preserve inversion and photographic precision; ordinary f32 AP1 is not an exact reusable prefix. These conservative checks can reject modest stacks and passing fixtures are not universal supported-domain proof.

Normal editor controls, source-anchored tools, persistence and GPU export share these snapshots. Admission validates complete candidates against the calibrated GPU domain, including inactive/offscreen records, without publishing replacement resources. Support depends on the full calibration/response/Global/local stack, not a fixed adjustment count. [Photographic, interaction and broader-device acceptance](plans/local-adjustments.md) remains open; the editor's soft radial creation default does not change persisted falloff semantics.

### Preserve information, reduce mathematical work

GPU source processing retains compensated constants and temporaries, not blanket ordinary-f32 arithmetic. Near film base, losing a small sample/reference difference can become visible after exposure; camera-matrix cancellation and later grades can likewise expose errors. Ordinary-output processing instead reduces transcendental arguments before using shorter compensated polynomials, then recombines them with stable identities. This preserves the existing numerical gates rather than selecting a looser precision contract. The primitive checks cover the reduction joins and dense arguments; truncation bounds alone do not establish whole-pipeline or photographic accuracy.

This shared math benefits continuous corrected-original processing and some precise grading, including source/view acquisition. Lookup-backed uncorrected inversion avoids much of that work, and cached physical-view slider grading already has its separate stable-f32 implementation. Analyze's statistical estimator is CPU-owned. Compensated EXR uses full-series functions and preserves precision through original-sample reconstruction and output quantization.

### Compact storage can reuse expensive computation

The retained source preserves the developed RGB16 codes. For uncorrected positive processing, the GPU evaluates all integer codes exactly into a bounded response table, then looks up channel values before unchanged matrix/photographic processing. There is no interpolation or code quantization. Its key covers native response constants and combined Roll/Photo exposure; WB, camera matrix, local stages, source geometry and output target remain downstream. A continuous flat-field-corrected sample still uses precise direct inversion, and inspection bypasses the table and photographic grades.

This is one kernel-owned 2.5-MiB table, not an image-sized source cache. Queue-ordered initialization and immutable publication keep abandoned/reordered encoders and concurrent workers safe; in-flight commands can retain evicted versions. Physical-view AP1 reuse has separate view-scoped ownership and keys. [Diagnostics](diagnostics.md#local-processing-and-reuse) distinguish exact original processing from interactive view processing.

### Physical-view processing is an approximation

Ordinary source inspection also retains filtered linear AP1, but acquires corrected camera light without inversion. Only its display convention and transient inspection EV apply downstream, before pinned ACES; no calibration or photographic-grade object is required. Source/positive mode and correction are acquisition identity, whereas inspection EV and target reuse the source. Precision tools and original-sample processing remain separate.

The positive editor prototype factors exposure and photographic WB downstream of neutral inversion/camera AP1. Original camera samples still receive continuous correction and calibrated inversion before exact affine-footprint filtering into signed/HDR f32 AP1. Fixed-view grading then reuses that filtered input rather than processing the whole original again. Target-only changes can reuse graded AP1. Coarse whole-composition navigation support is independently filtered from the same originals into neutral AP1. Its accepted grade/target must match regional detail before their target-linear rasters are composed; composition adds neither another photographic grade nor another ACES transform. Available detail is not replaced by the coarse raster.

Filtering before nonlinear grading does **not** commute with original-sample grading before reconstruction. Nor do footprint-centre local masks equal the original-sample mask integral. These are preview approximations, not changes to the photographic model or original-resolution reference/export authority. The appearance requirement is the same photograph in the file, potentially with better-resolved detail—not changed exposure, colour, tone or local effects.

Stable f32 colour uses black-relative/linear-toe formulas, analytic divided differences and targeted compensated luminance accumulation. It does not replace precise source geometry/acquisition, add a working clamp, or remove whole-stack preflight. Identical-filtered-input arithmetic tests retain the existing gates; they do not establish preview/export photographic agreement or all-device precision. [Editor ownership](editor.md#a-stable-working-view-while-grading), [diagnostics](diagnostics.md#stable-float32-view-grading) and [unfinished acceptance](plans/view-working-set.md) keep those distinctions explicit.

## Important boundaries

The current model is **4**. Calibration must be finite and satisfy `Dmin > Dmax >= input_floor > 0` per camera channel. A roll retains its camera model and matrix; incompatible native-camera bases cannot share its calibration.

Unsupported domains fail validation rather than being repaired with an extra clamp, pedestal or CPU interactive fallback. Display-rendering limits belong at the [output boundary](colour-and-output.md), not in the working positive. Scalar/GPU agreement checks numerical implementation, not physical colour accuracy.

## Working on this area

- [Processing](../src/processing.rs): calibration, response, defaults, domains and pipeline order.
- [Colour](../src/color.rs): working-space matrices, balance coordinates and Bradford WB.
- [Photographic grading](../src/photographic.rs): contrast/saturation and stable signed evaluation.
- [Sampling](../src/sampling.rs): manual reference and neutral fitting.
- [GPU processing](../src/gpu.rs): the production implementation and preflight.
- [Local grades](../src/local_adjustments.rs) and [GPU grades](../src/gpu_local_adjustments.rs): ordered masks, weighted controls, snapshots and cumulative supported-domain checks.

The scalar implementation is the f64 reference. Exact equations, supported ranges and numerical gates live with the implementation and its tests: [camera inversion](../tests/camera_inversion.rs), [film response](../tests/film_response.rs), [photographic grading](../tests/photographic.rs) and [GPU processing](../tests/gpu_processing.rs). Start with the relevant [development checks](development.md#feature-specific-checks).
