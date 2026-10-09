# ACES 2 yellow-to-gray collapse with some output targets

Research finding supporting [the accepted SDR output decision](../decisions/sdr-output-and-presentation.md). For current behavior use [output contract](../contracts/output.md); ongoing work lives in [the roadmap](../../PLAN.md).

## Summary

Using **OpenColorIO 2.6.0**, the unmodified ACES 2 output transform can turn ordinary nonneutral AP1 yellow inputs into essentially neutral gray for:

- **ProPhoto/D50 at 100 nits**;
- **Rec.2020/D65 at 100 nits**.

The traced cause is a negative estimated upper-gamut intersection that wins the boundary minimum and triggers the transform's zero-chroma branch. This is not a TIFF/ICC, display-encoding, monitor-conversion or negative-inversion failure: it reproduces through OCIO's public CPU API with synthetic inputs, before those stages.

Antitone now uses **sRGB + P3-D65 at 100 nits**, with matching sRGB/Display P3 TIFF encodings. Neither ACES mathematics nor scene-linear EXR was changed. The user identified yellow collapse as matching the observed image problem and approved the P3 replacement. Actual-image visual acceptance and the remaining color/tonal diagnosis are separate from these numerical gates.

## Minimal numerical examples

Input is **scene-linear AP1/D60 (ACEScg)**. The pipeline is the OCIO built-in `ACEScg_to_ACES2065-1`, followed by `FIXED_FUNCTION_ACES_OUTPUT_TRANSFORM_20`. Outputs below are **linear target RGB**, before transfer encoding or any final application clamp.

| Limiting target | Peak | AP1 input | Linear target output |
| --- | --- | --- | --- |
| ProPhoto/D50 | 100 nit | `(0.4, 0.4, 0.2)` | `(0.213416412, 0.213416278, 0.213416368)` |
| sRGB/D65, control | 100 nit | `(0.4, 0.4, 0.2)` | `(0.239203766, 0.232830852, 0.110550933)` |
| Rec.2020/D65 | 100 nit | `(0.18, 0.18, 0)` | `(0.0797412843, 0.0797412172, 0.0797412395)` |
| sRGB/D65, control | 100 nit | `(0.18, 0.18, 0)` | `(0.104848325, 0.103158675, -0.00111583818)` |

The nearly equal output channels identify the unwanted gray collapse; equality of RGB codes between different target spaces is **not** the acceptance criterion. ProPhoto's output is already gray before D50→D65 adaptation. The original ProPhoto example also collapses with the same primaries and D65 or D60 whites, ruling out D50 adaptation as its cause.

The nine fixed-function parameters are ordered as `[peak_nits, Rx, Ry, Gx, Gy, Bx, By, Wx, Wy]`:

```text
ProPhoto: [100, .7347, .2653, .1596, .8404, .0366, .0001, .3457, .3585]
Rec.2020: [100, .708,  .292,  .170,  .797,  .131,  .046,  .3127, .3290]
```

## Reproduction in this checkout

After building the pinned OCIO dependency:

```sh
./scripts/build-ocio.sh
./scripts/probe-gamut.sh --compare-rec2020
```

The public-API probe in [`native/gamut_probe.cpp`](../../native/gamut_probe.cpp) has no RAW, inversion, input/output clamp, GPU, profile or monitor dependency. It tests 990 nonneutral combinations from ten AP1 channel levels between 0 and 1. Expected results with OCIO 2.6.0:

| Target | Peak | Gray collapses / colored samples |
| --- | --- | --- |
| sRGB/D65 | 100 nit | `0 / 990` |
| P3/D65 | 100 nit | `0 / 990` |
| Rec.2020/D65 | 100 nit | **`4 / 990`** |
| Rec.2020/D65 | 500 nit | `0 / 990` |

**Exit status 1 is expected** in comparison mode: it exposes the rejected SDR Rec.2020 target. The default command, `./scripts/probe-gamut.sh`, checks only the active sRGB/P3 pair and passes. The current Rust rendering probe's `--gamut-check` likewise checks the active pair, not retired ProPhoto.

The four Rec.2020 failures are `(0.18, 0.18, 0)`, `(0.18, 0.18, 0.01)`, `(0.2, 0.2, 0)` and `(0.2, 0.2, 0.01)`. A larger internal sweep found eight collapses among 3,360 eligible samples at 100 nits, versus none in the tested sets at 500/1000/2000/4000 nits. These finite tests do not establish all-domain correctness of the controls.

## Traced mechanism

Relevant functions in [OCIO 2.6.0 `ACES2/Transform.cpp`](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.6.0/src/OpenColorIO/ops/fixedfunction/ACES2/Transform.cpp) are `estimate_line_and_boundary_intersection_M`, `find_gamut_boundary_intersection` and `compressGamut`.

The approximate intersection is:

```text
M = shifted_intersection * M_max / (J_max - slope * M_max)
```

For the flipped upper hull, its denominator becomes:

```text
(target_J_max - cusp_J) + slope * cusp_M
```

In the failing cases this denominator is negative. The implementation applies a smooth minimum to the lower and upper estimates without first excluding the negative upper estimate. The later branch then explicitly sets chroma to zero:

```cpp
if (gamut_boundary_M <= 0.0f)
{
    return {J, 0.f, h};
}
```

Here `J` is the lightness coordinate, `M` is chroma magnitude and `h` is hue. Instrumented values immediately before gamut compression:

| Quantity | ProPhoto, AP1 `(0.4, 0.4, 0.2)` | Rec.2020, AP1 `(0.18, 0.18, 0)` |
| --- | --- | --- |
| Tone/chroma-compressed JMh | `(48.68013, 11.24486, 102.42621)` | `(30.6234627, 29.3046207, 106.006279)` |
| Cusp J/M | `(94.95605, 77.34206)` | `(91.2953949, 72.3279037)` |
| Ray slope | `−0.118814401` | `−0.128063783` |
| Lower intersection | `40.1366005` | `27.8146515` |
| Upper intersection | **`−1363.61035`** | **`−11265.6318`** |
| Output M | **`0`** | **`0`** |

The collapse is brightness-dependent: other exposure levels can retain yellow. A global saturation adjustment cannot repair this discontinuity. CPU/GPU agreement alone also cannot detect it, because both execute the same faulty boundary handling.

## Reference status and limits of the finding

The [official ACES package `v2.0.0+2025.04.04`](https://github.com/aces-aswf/aces/tree/v2.0.0%2B2025.04.04) pins `aces-core` to `2d7af39344725aaa8ac3bf1746693c9a1d6c4792`. Its [output-transform library](https://github.com/aces-aswf/aces-core/blob/2d7af39344725aaa8ac3bf1746693c9a1d6c4792/lib/Lib.Academy.OutputTransform.ctl) contains the same intersection equation, unqualified smooth minimum and nonpositive-boundary zero-chroma branch. The CTL source was inspected, **not executed**; this is not established as an OCIO-only port error or as complete CTL/OCIO numerical conformance.

The [official ACES preset table](https://docs.acescentral.com/system-components/output-transforms/parameters/) includes SDR/100-nit P3 and Rec.709, but lists Rec.2020 at 500/1000/2000/4000 nits. Thus the tested 100-nit Rec.2020 configuration is an SDR extension, not a supplied preset. ProPhoto is also not a supplied preset. Custom parameters are allowed in principle; these results demonstrate a limitation for the tested combinations, not a blanket prohibition on wide gamuts. The Rec.2020 reproduction also shows that imaginary ProPhoto primaries are **not necessary** for the failure.

A cache-only experiment that excluded the backward linearized upper intersection restored the original ProPhoto yellow. That supports the traced cause but is not a validated general correction. No such patch was deployed, and no upstream fix or issue submission is claimed here.

## Antitone resolution and follow-up

The explicitly approved resolution was to retire these failing SDR targets and use sRGB/P3, **not** to render P3 into a misleading ProPhoto or Rec.2020 container. See [P3 implementation and verification](p3-output-update.md): CPU/GPU yellow/exposure regressions, out-of-sRGB color preservation, ICC/TIFF checks, real editor smoke and the 56-test suite pass. Output compatibility was versioned to reject obsolete ProPhoto state rather than reinterpret it.

Any future SDR Rec.2020 or ProPhoto rendering must independently validate boundary-domain handling, near-singular intersections and hue/exposure continuity. An algorithm correction would require explicit versioning, shared CPU/generated-GPU behavior and comparison against the unmodified reference—not a viewer-only shader patch, hidden exposure offset or weakened test.

Detailed investigation records and ignored trace locations: [ProPhoto diagnosis](prophoto-gamut-diagnosis.md) and [SDR Rec.2020 gate](rec2020-rendering-gate.md).
