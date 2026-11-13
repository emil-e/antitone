# Frontier density-response treatments

Frontier A and B are reproduction alternatives in the **Paper** selector, with **Negative: None**. They consume graded AP1 positives after interpretation and film texture. Their preparation model applies a common basis matrix and analytic forward toe before a smoothed density-response table, then bakes the complete function into the shared creative LUT format. A supplied the calibration controls against Antitone's Fuji Crystal Archive TypeII treatment; B shares the relative formation with its source-defined code origin. Fuji is a photographic-print prior, not a pointwise imitation target or factory calibration.

## Formation and response

The [formation model](../../src/looks/frontier/formation.rs) defines the neutral-preserving matrix `M` and density-code parameters:

```text
c = max(0, gamut(M × AP1))
d_j = b_variant + delta_j + (k / t) × log2(1 + (c_j / h)^t)
b_B = b_A + (512 − 450)
t = 2.5
```

Channel offsets are applied in GRB order, then exchanged into RGB-ordered table coordinates. A matching-slope rational continuation above 1015 approaches 1023. The source negative setup centers A's modes 0/2 at 450 and B's modes 1/3 at 512; its unit-slope target-minus-measured-density offset motivates B's 62-code translation before continuation. Matrix, scale, toe location and channel-relative offsets remain shared. The nonnegative clamp is adapter-local; upstream signed/HDR AP1 is retained.

Three aggregate A-neutral features constrain density formation: mean OKLab lightness and lightness/EV slope over −2…+2 EV, and lower slope over −5…−2 EV. Six independent matrix coefficients fit relative-neutral OKLab colour on 120 moderate controls. B and photographs supply no fitted coefficients. The generic toe shape uses Antitone's C-41-supported reference, not a recovered Frontier film curve.

`density-a.f32.z` and `density-b.f32.z` contain 65³ RGB float32 AP1/D60 samples, little-endian with the first input axis fastest, compressed with zlib. Input is uniformly allocated formed native code coordinates in `[0,1023]³`. These are not ACEScct creative LUTs: use them after formation, not directly on scene-linear AP1. This interface separates the fixed response from formation; negative-stock formation is not implemented by this adapter.

The [authoring tool](../../src/looks/frontier/authoring.rs) evaluates the supplied native shapers and 32³ response, applies an encoded-sRGB continuation and the pinned inverse sRGB-reference ACES transform into AP1, then smooths the AP1 response volume on a 129³ code grid. A separable Gaussian uses σ0.5 grid steps, radius 2 and constant-edge extension; even nodes form the final 65³ table. Preparation samples this source volume trilinearly after formation. Smoothing is across colour coordinates, not neighbouring image pixels.

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

[Controls](../../tests/fixtures/frontier/README.md) distinguish source-model correspondence, shared LUT execution and saved numeric authority. Production does not link OCIO; the inverse reference is used only by offline source-volume generation.
