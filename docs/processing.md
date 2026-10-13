# From a negative to a positive

A camera scan records light passing through film. Dense parts of the negative transmit less light, but should become brighter parts of the positive. Antitone's processing model describes that transformation **given a calibration and grade**. [Roll analysis](roll-analysis.md) is a separate problem: choosing a useful starting calibration and grade from a set of scans.

The processing pipeline is:

```text
developed camera RGB
  → optional scan-illumination correction
  → per-channel negative inversion
  → camera-to-AP1 colour conversion
  → exposure and photographic white balance
  → Photo contrast and saturation
  → working positive, ready for rendering or EXR
```

[Colour and output](colour-and-output.md) explains what happens after the working positive. Display rendering is not part of negative inversion.

## Start with the camera's measurements

LibRaw develops the RAW into linear camera RGB using a fixed roll white balance. These are the camera's three channels, not sRGB and not three isolated film dyes. Keeping development fixed means a roll's calibration refers to the same kind of measurement in every photo.

LibRaw recognizes more RAW formats than Antitone's developer accepts. This path currently needs three-colour integer Bayer data without repeated black-level patterns; X-Trans, Foveon and linear DNG do not fit it. Both photo and reference pickers offer common camera RAW extensions, but decoded contents, the camera matrix and WB—not the filename—determine whether a source can be used. [Broader LibRaw compatibility](plans/raw-compatibility.md) is future work.

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

### Photo contrast and saturation

Contrast scales a log-like ACEScct signal around a pivot. Its Colour preservation blend moves from independent RGB contrast toward a black-relative colour-direction proxy with the same neutral tone response. That reduces some contrast-driven colour changes; it does not promise constant perceived saturation.

Saturation blends between preserving AP1 luminance and anchoring the largest RGB channel. The latter gives the Density control its darkening character on positive colours. Neither endpoint is a physical dye or print simulation. Contrast and saturation amounts of one are exact identities, including when their secondary controls are non-default.

See the [grading rationale](decisions/photographic-grading.md) for why these operators support signed working data.

## Important boundaries

The current model is **4**. Calibration must be finite and satisfy `Dmin > Dmax >= input_floor > 0` per camera channel. A roll retains its camera model and matrix; incompatible native-camera bases cannot share its calibration.

Unsupported domains fail validation rather than being repaired with an extra clamp, pedestal or CPU interactive fallback. Display-rendering limits belong at the [output boundary](colour-and-output.md), not in the working positive. Scalar/GPU agreement checks numerical implementation, not physical colour accuracy.

## Working on this area

- [Processing](../src/processing.rs): calibration, response, defaults, domains and pipeline order.
- [Colour](../src/color.rs): working-space matrices, balance coordinates and Bradford WB.
- [Photographic grading](../src/photographic.rs): contrast/saturation and stable signed evaluation.
- [Sampling](../src/sampling.rs): manual reference and neutral fitting.
- [GPU processing](../src/gpu.rs): the production implementation and preflight.

The scalar implementation is the f64 reference. Exact equations, supported ranges and numerical gates live with the implementation and its tests: [camera inversion](../tests/camera_inversion.rs), [film response](../tests/film_response.rs), [photographic grading](../tests/photographic.rs) and [GPU processing](../tests/gpu_processing.rs). Start with the relevant [development checks](development.md#feature-specific-checks).
