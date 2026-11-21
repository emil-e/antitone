# Frontier density-response treatments

Frontier A and B are reproduction alternatives in the **Paper** selector, supporting **Negative: None** and the colour-negative catalogue. They consume graded AP1 after interpretation and film texture. None uses a fixed positive-input convention; a selected stock adds nominal film formation and spectral scanner readout. Both end at the same native A/B response and prepare an ordinary creative LUT.

## Input adapters

### Negative None

The [positive adapter](../../src/looks/frontier/positive.rs) converts AP1/D60 to floating-point linear Rec.709/sRGB primaries at D65 using a fixed Bradford-adapted matrix. This is a change of coordinates, without the sRGB transfer curve or display-range clipping. It is an artistic input convention, not recovered scanner colourimetry.

Radial compression handles negative and near-boundary converted drives before the [shared characteristic](../../src/looks/characteristic.rs). Some ordinary photographic/chart colours enter its shoulder. A single scalar pulls the vector toward its maximum-channel neutral anchor; it preserves RGB direction and ordering before the nonlinear readout. All-nonpositive drives retain the characteristic's black endpoint.

```text
v = compress(AP1_to_linear_Rec709_D65 × AP1)
delta = neutral_None_bank × (phi(v) − phi(scene_grey))
q = grey_codes_bank + codes_per_log × gains_RGB × delta
```

[Positive parameters](positive.json) contain the code scale, per-bank diagonal neutral slopes and compression knee/limit. The basis is a named fixed transform in code, not a fitted matrix; there is no colour post-matrix. Working AP1 remains signed/HDR upstream of this adapter.

### Selected colour stock

The [stock adapter](../../src/looks/frontier/negative.rs) uses the same [nominal exposure and development](../../src/looks/spektrafilm/negative.rs) as colour film/paper processing. Its scanner integrates dye/base transmission against RGB bands, normalizes flux at film base and computes optical density `OD = −log10(relative transmission)`.

At scene grey, channel balance is `b = mean(Dgrey) / Dgrey`. The [stock parameters](negative.json) supply one density scale and per-bank diagonal neutral slopes:

```text
delta = neutral_stock_bank × (b × (OD − Dgrey))
q = grey_codes_bank + codes_per_OD × delta × gains_RGB
```

There is no cross-channel density-colour matrix. The scanner's spectral channels already define this input basis; applying the positive adapter's AP1-to-Rec.709 matrix to OD would mix domains. Retaining `mean(Dgrey)` preserves stock density strength: uniformly scaling OD scales centred code spans, whereas `OD/Dgrey` would cancel that strength. Nominal film exposure is fixed; individual stocks retain their own characteristic response.

[`sensor.json`](sensor.json) contains approximate digitized SP3000-inspired illumination bands. Detector/optics response is assumed constant within each band, without another D65 illuminant or CFA. [Provenance](provenance.json) records attribution and uncertainty.

## Neutral calibration and native readout

[Shared readout data](readout.json) own the scene-grey reference, native grey coordinates and relative channel gains. The [readout owner](../../src/looks/frontier/readout.rs) validates them and supplies both adapters' code-range continuation: negative codes clamp to zero; above 1015, `1023 − 64/(q − 1007)` approaches 1023. The A/B grey coordinates return AP1 0.18 directly from the native tables; do not append a separate source-mode +62 translation.

The density scale, about 408 code/OD, aligns the generic reference's A neutral exposure curve with Fuji Crystal Archive Type II/None. The positive adapter's separate scale, about 101 code/characteristic unit, follows that tonal reference. Bank-specific neutral slopes reduce tint while softly retaining the calibrated brightness curves. Stock slopes are common across all 16 treatments, not per-stock corrections. Actual stock neutral OD curves are off-axis, so diagonal calibration does not make every exposure perfectly neutral.

A can retain small warm stock floors; B retains its native nonzero floor and softer response. Native colour–brightness coupling remains part of the rendering, without compensating output gamma or pedestal subtraction. [Provenance](provenance.json) gives calibration context separately from executable parameters.

## Native response assets

`density-a.f32.z` and `density-b.f32.z` contain 65³ RGB float32 AP1/D60 samples, little-endian with the first input axis fastest, compressed with zlib. Input is uniformly allocated formed native code in `[0,1023]³`. These are preparation volumes: sample them after the input adapter, not directly on scene-linear AP1.

The [authoring tool](../../src/looks/frontier/authoring.rs) retains each source shaper's first constant interval and grey ordinate, joins the interior with monotone Hermite curves, and evaluates the 32³ response. An upper encoded-sRGB continuation and the pinned inverse sRGB-reference ACES transform produce the AP1 carrier. A separable Gaussian smooths a 129³ code grid with σ0.5 and radius 2; difference-form odd reflection preserves lower endpoints, while upper boundaries extend constantly. Even nodes form the 65³ table. This smooths colour coordinates, not neighbouring image pixels.

Preparation samples native volumes trilinearly. Their AP1 output carries authored rendering, not recovered scene light. Ordinary selected-output ACES follows, including the actual Display P3 transform. [Colour/output](../../docs/colour-and-output.md#colour-treatments-precede-output-rendering) owns composition and bypasses; do not add another inverse transform after native readout.

## Regenerate from supplied software

Normal builds and tests use the derived response assets; original Fujifilm software is not included or needed. After [native setup](../../docs/build-and-run.md#build-and-run), supply the installation's `PARAM/imgp/ColorData.dat` and `ez171autoc-nega.dat`:

```sh
cargo run --locked --release --no-default-features \
  --features project,ocio-reference --bin antitone-frontier-bake -- \
  /path/to/ColorData.dat /path/to/ez171autoc-nega.dat \
  .cache/frontier-response-candidate
```

The command requires a fresh destination, checks the supported source revision and reads parameter files without executing vendor software. It writes native response candidates and generation metadata. Inspect candidate numbers and run correspondence checks before replacing assets. The interpretation uses authored run-centre Hermite shapers and bounded tails, not recovered ASIC interpolation. Source availability does not establish redistribution permission; public asset clearance remains separate from local verification.

## Preparation and saved projects

The [provider](../../src/looks/frontier.rs) feeds [shared preparation](../../src/looks/authoring.rs), producing a 65³ ACEScct/AP1 creative LUT. Formation and native tables are preparation inputs, not captured runtime stages. The creative cube approximates the complete model, especially near colour boundaries. Its default input range covers approximately −0.054 to +356 linear AP1 per component; lookup clips outside that range without altering upstream data.

Projects embed the resolved samples and provenance. Reopening executes those numbers without consulting the catalogue; explicitly selecting a treatment prepares the current model. All providers use the same GPU tetrahedral operation and resource ownership. [Controls](../../tests/fixtures/frontier/README.md) distinguish source-model correspondence, creative-cube execution and saved numeric authority. Production does not link OCIO; offline authoring and reference checks do.
