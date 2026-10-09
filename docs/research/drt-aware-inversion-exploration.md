# DRT-aware inversion exploration

Historical experiment; no DRT lift or pedestal was deployed. Later film-response integration is recorded in [the precision gate](film-response-precision-gate.md). The statements below describe this experiment's checkpoint, not today's response defaults.

## Status and question

This is an isolated, offline experiment, **not an implemented processing change**. The editor, processing model 2, project schema 3, GPU assets, ACES mathematics, TIFF targets and AP0 EXR contract remain unchanged. Samples were read only.

The subsequent agreed direction is to fit a generic film-response family to published photographic-film curves rather than derive it from ACES. See [film-response-model-study.md](film-response-model-study.md). The black-placement counterfactual here is not evidence that physical film can be less dense than its base, nor a justification for changing the base anchor; illumination/flat-field work is deferred.

The practical question is whether an inversion can produce a grading-friendly working representation informed by the DRT, without requiring a measured film-stock characteristic curve. The analogy is lifting a positive, display-referred image through an inverse rendering response: zero exposure restores a chosen positive appearance; exposure operates in the lifted space before rendering again.

Unlike display-referred input, a negative does not provide that positive appearance. Choosing it remains a modelling/look decision. Numerical neutral identity does not establish recovery of scene radiometry, the stock's original toe/shoulder, or full RGB identity.

See [processing-model.md](../contracts/processing-model.md) for the unchanged deployed processing contract.

## Actual forward/inverse reference

The experiment uses OCIO 2.6.0's public ACES Output Transform 2.0 forward/inverse, with Antitone's current 100-nit sRGB/D65 parameters. The forward is the actual processor from `native/rendering_transform.h`, including the normal input/output ranges. The inverse fixed function returns AP0, subsequently converted to AP1 through the public OCIO built-in. Only neutral input is used to derive the scalar lift; no dependency equations were copied or patched.

Let `R(x)` be the mean linear target RGB produced by a neutral AP1 input `[x,x,x]`. Its inverse is evaluated with the public inverse processor and mean reconstructed AP1 RGB. This is a neutral-response study, not an inverse of the full colour/gamut rendering graph.

A 9,729-node logarithmic forward sweep, from `2^-32` to `64`, was strictly monotonic. The public inverse's maximum forward round-trip difference was `1.43e-6` in mean linear display RGB. The largest relative reconstructed scene-value difference was `7.01e-5`, near the flatter high end. Selected values:

| Neutral AP1 | Linear display mean |
| --- | --- |
| 0.00390625 | 0.000090678 |
| 0.03125 | 0.007008952 |
| 0.125 | 0.063304760 |
| 1 | 0.457565576 |
| 8 | 0.885574162 |
| 64 | 0.991555452 |

sRGB/P3 neutral responses differed by at most `9.94e-8` in the separate reference sweep. A fixed reference can therefore be investigated without making the working reconstruction depend on the selected output target. This finite neutral agreement is not an equivalence claim for colour rendering.

## Candidate baselines and lifts

All candidates start with the existing inversion's signed AP1 output `q`, **before photographic exposure**, unless explicitly stated otherwise. Original gamma and gamma balance are retained. This isolates conditioning from replacing the whole film model.

The initial neutral reference treats a normalized positive level as linear display brightness, with white matched to current neutral AP1 1:

```
c = R(1) = 0.457565575838
F(t) = c * t
C(t) = inverse_R(F(t))
```

For neutral levels `0..1`, `R(C(t))` restores that chosen baseline within `4.23e-7` in linear display mean. Exposure is subsequently an ordinary `2^EV` multiplication followed by unchanged ACES. The white matching is an explicit convention, not a fitted film property.

The following candidates were compared:

- **Positive-luminance shared gain:** use AP1 luminance of positive components as `t`, then multiply the original signed RGB by `C(t)/t`.
- **Per-channel inverse:** apply the neutral inverse independently to each RGB component, using a signed extension. This changes working RGB direction and visibly changes colour; it is not a suitable default.
- **Naive density reference:** average channel-normalized AP0 log ratios `ln(dmin/scan)/ln(dmin/dmax)`, map the mean to an encoded positive tone with white `0.706683743`, decode the sRGB transfer, and lift its luminance while retaining the original RGB direction. This creates additional black pixels on these images. Averaging effective RGB densities is not a validated colour/density model, and this particular baseline was rejected. It does not rule out better density-based positive renditions.
- **Signed-safe radial shared gain:** use a magnitude `t = sqrt(dot(AP1_Y_weights, q*q))`, then multiply all RGB by `C(t)/t`. The value at RGB zero is zero. Unlike positive-luminance division, the radial magnitude is nonzero for every nonzero signed RGB vector.
- **Partial radial toe lift:** use the radial gain fully below magnitude `0.1`, fade its logarithm smoothly to zero between `0.1` and `0.18`, and leave RGB unchanged above `0.18`. These thresholds are illustrative design choices. This is a partial DRT-aware conditioner, **not full neutral identity over the entire tonal range**.

The actual ACES forward renderer remains in every output. No curve approximation or baked LUT replaces it.

### Signed-domain finding

The first positive-luminance formulation is unsuitable for general signed working data. For `q=(-0.04,1e-12,0)`, it produces approximately `(-47032,1.18e-6,0)`. As the small positive channel tends to zero, its gain becomes arbitrarily large despite the fixed negative component. An all-nonpositive special case does not restore continuity.

The radial toe formulation instead produces `(-0.07101,1.78e-12,0)` for the same vector, preserving direction without the blow-up. A `1e-20` positive component has the same bounded radial gain. Uniform negative inputs remain negative; nothing is clamped out of working data.

The partial radial curve was monotonic over 65,537 neutral nodes. Working RGB direction changed by at most `3.85e-16` on the positive colour fixtures. Neutral inputs `0.18`, `1`, `8`, `1024` and `65536` were exactly unchanged above the partial-lift pivot. These are CPU/reference checks, not GPU acceptance.

### Shoulder/domain finding

A literal full lift of `F(t)=c*t` is not defined over arbitrary HDR values: the desired display brightness eventually exceeds the invertible rendering domain. The study explicitly rejects inverse requests beyond `R(64)`; it does not silently clamp them or use an identity fallback. Even beyond this conservative study domain, a display-limited rendering response cannot restore arbitrary above-white reference brightness.

The partial toe variant avoids this by leaving the upper working range unchanged. It consequently **does not solve shoulder reconstruction**. A full toe/shoulder design still needs an explicit bounded positive reference or another monotonic HDR-preserving reference construction. The neutral inverse alone does not make that decision.

## Image comparison

Two source files were developed with fixed first-import WB `[2133,1024,1771,1024]`:

- `DSC04393.ARW`: the dog image identified from the user's screenshot.
- `DSC04399.ARW`: a second same-roll life-ring image for colour/shadow stress.

The screenshot-derived inputs are rounded and Photo settings are unknown: Dmin `[0.140103,0.113156,0.102145]`, Dmax `[0.008061,0.003409,0.001442]`, gamma `0.65`, gamma balance warmth `-0.41`/tint `-0.01`, assumed Photo WB/exposure zero. These are diagnostic reconstructions, not exact user grades.

Each output has 720×480 nearest-centre samples from the same source rectangle `(192,96,8136,5424)`. This excludes scan margins, is not full-resolution area-averaged preview acceptance, and changes no source/editor geometry. No sampled input hit the precision floor.

Exposure was explicitly matched to the current inversion's EV +3 rendition using one fixed source-region comparison patch `(4700..4900,4200..4400)`, 324 sampled points. The patch is a brightness reference, not an assumed neutral target. Matching uses mean rendered P3 luminance. Additional ±1 EV and unchanged-original-EV outputs separate conditioning from overall brightening.

| Image / variant | Matched EV | Exactly black pixels | Max encoded channel below 1/255 |
| --- | --- | --- | --- |
| Dog / current | +3.000 | 4,974 | 13,257 |
| Dog / radial toe lift | +1.531 | 4,974 | 6,563 |
| Life ring / current | +3.000 | 5,256 | 24,194 |
| Life ring / radial toe lift | +2.072 | 5,256 | 7,474 |

Counts are out of 345,600 sampled pixels and measured before TIFF quantization. The conditioner opens surviving low-level detail after brightness matching, but does not move the inherited zero boundary. The image comparisons also show reduced bright/dark separation; shadow opening is not free of a contrast/look tradeoff. Rendering colour can change with intensity even when working RGB direction is preserved.

## Separate black-placement counterfactual

To distinguish toe compression from the existing black boundary, one variant applies an explicit neutral offset **before** the radial toe lift:

```
a = 0.001
q_shifted = (q + [a,a,a]) / (1+a)
```

This keeps normalized white `[1,1,1]` unchanged but stops declaring Dmin absolute scene zero. It is an illustrative look/baseline adjustment, not a sensor-noise estimate, automatic calibration, or recovered film response. It is not hidden inside the toe function.

At matched brightness, exactly black counts drop to zero in both sampled crops; below-1/255 counts become 77 for the dog and 10 for the life ring. Matched EVs are +1.438/+1.928. This supports the distinction between zero placement and positive-shadow shaping, **not proof that the offset is correct**. It introduces a neutral pedestal, may expose grain/noise and reduce desired black depth, and needs visual acceptance.

## Assessment and remaining work

The DRT-aware lift concept is numerically workable and can improve positive shadow separation without knowing the exact film curve. The signed-safe radial formulation is a better experimental carrier than clamped-positive luminance or independent RGB inverses.

This is not yet a replacement inversion model or a definition of film-native identity. Before adoption:

- Choose the positive reference, black-placement semantics and intended toe/shoulder behavior; preserve true distinctions between intensity, luminance and display appearance.
- Evaluate grain, near-zero colour, high-density areas, exposure sweeps and varied photos with exact user settings. Two static diagnostic crops are insufficient.
- Establish a monotonic shoulder/headroom construction rather than hiding inverse-domain errors.
- Implement/verify GPU parity, stable near-zero mathematics and f32 bounds; f64 magnitude code is not automatically safe in f32.
- Ensure neutral fitting samples the actual proposed processing, rather than silently fitting the old path.
- Define resets/default equivalence, version/persistence compatibility, source bypass, TIFF/EXR semantics and fixed-reference behavior. These approximate grading intensities are not calibrated scene radiometry.

## Evidence and reproduction

Preserved experiment sources ([tool status and reproduction](tools/README.md)):

- `docs/research/tools/native/drt-lift-neutral.cpp`: actual OCIO neutral sweep and batch public inverse.
- `docs/research/tools/film/src/bin/drt-lift.rs`: scalar candidates, signed/HDR/ramp checks, image comparison and managed P3 TIFFs.
- `docs/research/tools/drt-lift/contact-signed.swift`: managed-image comparison overview.

Evidence under `.cache/probes/drt-lift/`: `neutral-check.txt`, `neutral.csv`, initial `comparison.png`/`exposure-dog.png`, and `run2`/`run3` logs, ramps, colour CSVs and P3 TIFFs. The final overview is `run3/comparison-signed.png`; individual TIFFs are the colour-managed reference outputs. Overviews are not physical-monitor acceptance.

The native source compiles with Apple Clang, OCIO's existing local include/lib directories, `-std=c++17 -O2 -Wall -Wextra -Werror`, and the existing OCIO rpath. The historical Rust source predates current export/GPU APIs; do not treat it as a currently verified executable. Its manifest is `docs/research/tools/film/Cargo.toml`. At the original checkpoint, the build used the Rust manifest with the checkout-local `scripts/cargo.sh`, root `CARGO_TARGET_DIR`, and `--offline --bin drt-lift`; run with the local OCIO directory in `DYLD_LIBRARY_PATH`. TIFF outputs refuse overwrite; select a fresh ignored `DIR` in the experiment before repeating an image run. No production tests were changed or rerun for this offline-only study.
