# Frontier density-response treatments

Frontier A and B are reproduction alternatives in the **Paper** selector, supporting **Negative: None** and the colour-negative catalogue. They consume graded AP1 positives after interpretation and film texture, then bake formation and a smoothed density response into the shared creative LUT format. A is calibrated against Antitone's Fuji Crystal Archive TypeII; B shares the entry with its source-defined code origin. Fuji supplies a photographic-print prior, not pointwise imitation or factory calibration.

## Formation and response

### Negative None

The [formation model](../../src/looks/frontier/formation.rs) defines the neutral-preserving matrix `M` and code scale/gains. Its forward toe uses the [shared reference-negative curve](../../src/looks/characteristic.rs), with toe and knee from [one definition](../looks/reference-negative.json):

```text
c = gains_RGB × max(0, gamut(M × AP1))
d_j = origin_variant + (k / t) × log2(1 + (c_j / h)^t)
origin_A = 0; origin_B = 512 − 450
t = 2.5
```

Black enters A at code zero, without a fitted pedestal or additive channel offsets. A matching-slope rational continuation above 1015 approaches 1023. The source negative setup centers A's modes 0/2 at 450 and B's modes 1/3 at 512; its unit-slope target-minus-measured-density offset motivates B's 62-code translation before continuation. Other formation parameters are shared. The nonnegative clamp is adapter-local; upstream signed/HDR AP1 is retained.

Four parameters—code scale, exposure knee and R/G and B/G gains—fit 33 A-neutral controls over −5…+3 EV pointwise in OKLab. The reference is Fuji TypeII/None with its black aligned to A's zero in linear sRGB, preserving display white one: `(Fuji − Fuji_black) / (1 − Fuji_black)` per channel. This changes calibration targets, not output pixels. Those parameters are frozen before six row-sum-one matrix coefficients fit relative-neutral colour on 120 moderate controls. The matrix precedes gains and preserves the fitted neutral trajectory. B and photographs supply no fitted coefficients; shadow response need not equal Fuji's. The generic toe shape uses Antitone's C-41-supported reference, not a recovered Frontier film curve.

### Selected colour negative

The [stock formation](../../src/looks/spektrafilm/negative.rs) is shared with colour film/paper processing. Fixed-exposure development produces dye coefficients; the [scanner entry](../../src/looks/frontier/negative.rs) integrates transmission through the dye/base spectra against RGB bands, normalized at film base. Its output is optical density, `−log10(relative transmission)`.

A balance `b_j = mean(Dgrey) / Dgrey_j`, measured at scene grey 0.18, removes grey chroma while retaining each stock's mean optical density. The shared mapping is `q = origin_variant + 515 × gains_RGB × M × (OD × b)`, with A's origin zero and B's source-derived +62 codes before continuation. Negative codes clamp to zero; above 1015 the same upper continuation approaches 1023. The row-sum-one matrix fits 120 Portra 400/A relative-own-neutral OKLab colour controls against Fuji TypeII with the same stock, biased toward identity. Stock exposure stays fixed, without additional H-D/toe/shoulder compensation or per-stock brightness equalization.

A shared rendering gamma `sign(x) × |x|^0.9514395916432657` operates componentwise in linear sRGB-reference RGB after the native response. Zero and normalized rendering white one are fixed points; that white is not scene AP1 one or native code 1023. The exponent matches Portra 400/A middle-grey lightness to None/A with density calibration frozen. None supplies the tone reference, not the colour target. B shares the exponent, so its nonzero source floor can shift. Contrast and coloured lightness remain free to differ from the references.

[`sensor.json`](sensor.json) contains approximate digitized SP3000-inspired bands, with source attribution and uncertainty. They represent relative RGB illumination on a monochrome detector, without an additional D65 illuminant or CFA. Detector/optics response is assumed constant within each band. The calibration and source identities are recorded in [provenance](provenance.json).

### Shared response

`density-a.f32.z` and `density-b.f32.z` hold the native responses used by None. `density-negative-a.f32.z` and `density-negative-b.f32.z` hold the gamma-authored responses shared by all colour stocks. Each contains 65³ RGB float32 AP1/D60 samples, little-endian with the first input axis fastest, compressed with zlib. Input is uniformly allocated formed native code in `[0,1023]³`. These are preparation volumes, not ACEScct creative LUTs: sample them after formation, not directly on scene-linear AP1.

The [authoring tool](../../src/looks/frontier/authoring.rs) retains each source shaper's first constant interval and grey ordinate, joins the interior with monotone Hermite curves, and evaluates the 32³ response. It applies an upper encoded-sRGB continuation and the pinned inverse sRGB-reference ACES transform into AP1; exact black needs no positive landing guard. A separable Gaussian smooths a 129³ code grid with σ0.5 grid steps and radius 2. Difference-form odd reflection at lower boundaries preserves the native endpoint; upper boundaries use constant extension. Even nodes form the final 65³ table. Native A's endpoint is zero; B's source endpoint is genuinely nonzero. Smoothing is across colour coordinates, not neighbouring image pixels.

For selected-stock responses, authoring takes the native 65³ AP1 nodes through the forward sRGB reference, shared gamma and inverse reference, then stores float32 AP1 at those same code nodes. There is no additional smoothing. Precomposing this adjustment keeps normal preparation independent of OCIO and avoids maintaining a separate full creative cube per stock. Preparation samples the selected response trilinearly; this introduces a small approximation compared with applying gamma after native-response interpolation.

AP1 output carries the authored rendering, not recovered scene light. Ordinary selected-output ACES follows, including the actual Display P3 transform. Do not apply another inverse lift to the AP1 table. [Colour/output](../../docs/colour-and-output.md#colour-treatments-precede-output-rendering) owns composition and bypasses; [provenance](provenance.json) records source identities, derivation and calibration.

## Regenerate from supplied software

Normal application builds and tests use the derived response assets; original Fujifilm software is not included or needed. After [native setup](../../docs/build-and-run.md#build-and-run), supply the original installation's `PARAM/imgp/ColorData.dat` and `ez171autoc-nega.dat`:

```sh
cargo run --locked --release --no-default-features \
  --features project,ocio-reference --bin antitone-frontier-bake -- \
  /path/to/ColorData.dat /path/to/ez171autoc-nega.dat \
  .cache/frontier-response-candidate
```

The command requires a fresh destination, checks the supported source revision, reads parameter files without executing vendor software and writes derived response candidates plus generation metadata. Inspect candidate numbers and run correspondence checks before replacing maintained assets. The interpretation uses authored monotone run-centre Hermite shapers and bounded tails, not recovered ASIC interpolation. Source availability and transformation do not establish redistribution permission; public asset clearance remains separate from local verification.

## Preparation and execution

The [provider model](../../src/looks/frontier.rs) feeds [shared preparation](../../src/looks/authoring.rs), producing an ordinary 65³ ACEScct/AP1 creative LUT. Formation and the source density table are preparation inputs, not captured runtime stages. The cube approximates the full model: its analytic toe is not an exact asymptotic guarantee of piecewise interpolation. The default input range covers approximately −0.054 to +356 linear AP1 per component; values outside it clip at lookup, without modifying upstream data.

Projects embed the resolved LUT, numeric identity and provider provenance. Reopening executes those samples without loading Frontier source assets or consulting the catalogue. All providers use the same GPU tetrahedral operation and weakly shared LUT buffers; active consumers retain their own leases. Explicit catalogue selection prepares the current model.

[Controls](../../tests/fixtures/frontier/README.md) distinguish source-model correspondence, shared LUT execution and saved numeric authority. Production does not link OCIO; reference transforms are used only by offline source-volume generation.
