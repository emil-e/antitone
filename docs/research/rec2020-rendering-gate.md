# SDR Rec.2020 replacement — validation gate

Status: the user approved evaluating **sRGB + Rec.2020** and retiring ProPhoto as a rendering target, with matching TIFF encoding. **The 100-nit Rec.2020 validation failed before replacement.** At this gate, application targets/assets/contracts were unchanged. The user then explicitly approved P3 instead; see [implemented sRGB/P3 update](p3-output-update.md). This document preserves the rejected Rec.2020 candidate, not current application behavior.

## Plan and outcome

1. [x] Validate the unmodified 100-nit Rec.2020/D65 ACES 2 transform before deployment — **failed**, as detailed below.
2. [ ] Replace the preview/TIFF target and defaults only after an acceptable rendering policy is agreed.
3. [ ] Define the SDR transfer/ICC together, regenerate/checksum native GPU/profile assets, and version the output/project compatibility contract. No ProPhoto curve or relabelled asset reuse.
4. [ ] Verify CPU/GPU, gamut/hue/exposure continuity, neutral/near-black ramps, profiles, TIFF reopening and editor target switching; preserve scene-linear EXR and source/calibration behavior.

This remains within feedback step 1; project Save/Open and the rest of the feedback plan are retained, not displaced by an unapproved rendering fork.

## Reproducible public-API acceptance probe

```sh
./scripts/probe-gamut.sh --compare-rec2020
```

`native/gamut_probe.cpp` uses the installed OCIO 2.6.0 **public API**, AP1→AP0 conversion and the unmodified ACES 2 fixed-function output transform. There is no inversion, application input/output clamp, GPU, ICC profile or monitor conversion. It evaluates 990 nonneutral combinations from ten AP1 levels between 0 and 1 for each target. It checks finite output and rejects essentially gray output from these moderate-exposure colored samples; it does not require equal target RGB or equal saturation across gamuts.

| Target | Peak | Colored samples | Color-collapse failures |
| --- | --- | --- | --- |
| sRGB/D65 | 100 nit | 990 | 0 |
| P3/D65 | 100 nit | 990 | 0 |
| Rec.2020/D65 | 100 nit | 990 | **4** |
| Rec.2020/D65 | 500 nit | 990 | 0 |

The comparison mode intentionally exits **1** on the rejected Rec.2020 SDR candidate. The default command now checks only the active sRGB/P3 pair and passes. Its diagnostics identify each failed input. Example:

- AP1 `(0.18, 0.18, 0)` is an ordinary yellow input.
- Rec.2020/100-nit output: `(0.0797412843, 0.0797412172, 0.0797412395)` — effectively gray.
- sRGB/100-nit output retains color: approximately `(0.104848325, 0.103158675, -0.00111583818)` before the application's specified final output clamp.
- P3/100-nit output also retains color before its final clamp.

A larger cache-only internal sweep used 16 channel levels, including near-black and HDR values through 1024, and examined pixels with positive tone-mapped J below the target white and nontrivial pre-gamut-compression M. At 100 nits it found **8 failures in 3,360 eligible samples** for Rec.2020, versus none for sRGB or P3. At 500/1000/2000/4000 nits, Rec.2020 had none in the corresponding tested sets. These finite sweeps do not prove full conformance or all-domain correctness.

## Cause and significance

The isolated internal trace for AP1 `(0.18, 0.18, 0)` at Rec.2020/100 nits shows:

- Tone/chroma-compressed JMh: `(30.6234627, 29.3046207, 106.006279)`.
- Target cusp J/M: `(91.2953949, 72.3279037)`.
- Compression-ray slope: `-0.128063783`.
- Lower-hull intersection: `27.8146515`.
- Upper-hull intersection: **−11265.6318**.
- The smooth minimum selects the negative intersection, triggering zero chroma.

This is the same failure mechanism as the [ProPhoto diagnosis](prophoto-gamut-diagnosis.md), but **does not require imaginary ProPhoto primaries**. Rec.2020's standardized primaries alone do not establish validity of every peak-luminance parameterization.

The [official ACES 2 preset table](https://docs.acescentral.com/system-components/output-transforms/parameters/) lists Rec.2020 rendering targets at **500/1000/2000/4000 nits**, whereas P3 and Rec.709 include SDR/100-nit presets. The tested results support the distinction between the stock HDR range and this SDR extension. They do not establish that HDR output can be used in the existing SDR host by merely changing the peak parameter or clamping it.

## Decision required

- **Standard SDR route:** use sRGB + P3-D65, then matching TIFF encoding. P3/100 nits is an official rendering preset and passed these focused tests; GPU/profile/export integration would still need verification. This is a change from the requested Rec.2020 pair, not an automatic fallback.
- **Keep SDR Rec.2020:** investigate an explicitly versioned correction/extension to the ACES boundary domain handling, shared by native CPU and generated GPU. This requires an exception to the unchanged-ACES-mathematics contract; no correction has been deployed.
- Rec.2020 HDR is a separate possible future workflow, not a replacement for the current managed SDR presentation/export contract.

## Evidence

`.cache/probes/rec2020-public-gamut-gate.txt`, `rec2020-public-isolation.txt`, `rec2020-internals.cpp/.txt`, `rec2020-validation.cpp`, and `rec2020-sweep-{100,500,1000,2000,4000}.txt`. Internal diagnostics link existing OCIO objects with its unmodified ACES2 source accessible in the diagnostic translation unit; no dependency source/build was patched or reinstalled.

At this gate, the public probe compiled and reproduced the expected failure without production changes; no full Cargo suite or UI smoke was run then. The later P3 implementation and its full verification are recorded separately.
