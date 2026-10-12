# ProPhoto gamut-boundary failure

Status: cause of the synthetic color collapse identified. The user subsequently evaluated sRGB + Rec.2020, encountered its [SDR gate](rec2020-rendering-gate.md), and explicitly approved the implemented [sRGB/P3 replacement](p3-output-update.md). This document records the retired ProPhoto failure; ACES mathematics, calibration and native dependency builds were not patched. This finding does **not** establish the complete cause of the user's reported excessive saturation in their current grade.

## Replacement-target acceptance check

This current check uses the replacement sRGB/P3 targets, not the retired ProPhoto parameterization. With native setup complete, no RAW or generated fixture is needed:

```sh
cargo run --locked --offline \
  --features render --bin antitone-rendering-probe -- --gamut-check
```

At the time of the ProPhoto investigation, this checked an ordinary AP1/D60 yellow `(0.4, 0.4, 0.2)` and neutral `(0.4, 0.4, 0.4)` through the production targets in a common adapted linear-sRGB basis and **exited 1** because ProPhoto turned yellow gray. The current command checks the replacement sRGB/P3 pair and passes; it no longer reproduces the retired target. The acceptance criterion is merely retained visible chroma, not identical sRGB/ProPhoto rendering. Neutrality is checked separately. The original failure was an acceptance gate, not a passing regression endorsing broken output.

| Target | Yellow, linear target RGB | Neutral, linear target RGB |
| --- | --- | --- |
| sRGB | `(0.239203766, 0.232830852, 0.110550933)` | approximately `(0.230550, 0.230550, 0.230550)` |
| ProPhoto | `(0.213416412, 0.213416278, 0.213416368)` | approximately `(0.230550, 0.230550, 0.230550)` |

ProPhoto yellow is already gray **before** profiles, Bradford display adaptation or transfer encoding. The adapted linear-sRGB result remains gray. Existing bulk/scalar fixture agreement and CPU/GPU tests reproduce the same algorithm and cannot detect this defect by consistency alone.

A separate ignored public-API C++ isolation probe omits Antitone's inversion, input/output clamps, profiles, GPU and presentation. The yellow retains color with sRGB, P3 and Rec2020 primaries, but collapses with ProPhoto primaries using **each of D50, D65 and D60**. Thus D50 adaptation is not the cause of this case.

## Instrumented cause

An isolated executable links the already-built OCIO object files, replacing only the ACES2 transform translation unit with the **unmodified source included into the diagnostic translation unit** so its intermediates are accessible. It does not rebuild/install/modify the OCIO dependency or the app. Trace:

- Tone/chroma-compressed input JMh: `(48.68013, 11.24486, 102.42621)`.
- ProPhoto cusp J/M: `(94.95605, 77.34206)`.
- Compression-ray slope: `-0.118814401`.
- Positive lower-hull boundary: `40.1366005`.
- Flipped upper-hull denominator: `(100 - cusp.J) + slope * cusp.M`, approximately **−4.1454**.
- Upper-hull estimate: **−1363.61035**.
- Smooth minimum selects that negative upper estimate.
- `compressGamut` sees `gamut_boundary_M <= 0` and explicitly returns `(J, 0, h)`: zero chroma.

Relevant OCIO 2.6.0 source: `src/OpenColorIO/ops/fixedfunction/ACES2/Transform.cpp`, `estimate_line_and_boundary_intersection_M`, `find_gamut_boundary_intersection`, and `compressGamut` (approximately lines 969–1058). The generated ProPhoto shader contains the same calculation and branch. More yellow examples `(0.4, 0.4, 0)` and `(0.4, 0.4, 0.1)` also collapse; `(0.8, 0.8, 0.2)` has a positive upper estimate and retains color. The failure is brightness-dependent, not a constant saturation offset.

## Reference status

The [official aggregate ACES package `v2.0.0+2025.04.04`](https://github.com/aces-aswf/aces/tree/v2.0.0%2B2025.04.04) pins `aces-core` to **`2d7af39344725aaa8ac3bf1746693c9a1d6c4792`**. Its [output-transform library](https://github.com/aces-aswf/aces-core/blob/2d7af39344725aaa8ac3bf1746693c9a1d6c4792/lib/Lib.Academy.OutputTransform.ctl) uses the same intersection equation, unqualified smooth minimum, and nonpositive-boundary zero-chroma branch. Source inspection of current ACES core and [OCIO main](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/main/src/OpenColorIO/ops/fixedfunction/ACES2/Transform.cpp) also finds these equations unchanged. This is not established as an OCIO-only port error. The CTL reference was inspected, not executed; exact full CTL/OCIO conformance is not claimed.

## Diagnostic candidate and decision required

A **cache-only experiment**, not production code, excludes a backward/nonpositive-denominator linearized hull intersection from the forward-ray boundary minimum, using the other forward intersection. For the original yellow it selects `40.1366005` and preserves tone/chroma-compressed M `11.2448568`. Native ProPhoto output becomes approximately `(0.215808466, 0.225540757, 0.125007942)` instead of gray. Red, neutral and the brighter yellow controls are unchanged in the small tested set. This confirms the causal role of the negative intersection; it is not broad validation of a finished gamut mapper.

**Earlier experimental approach, not approved or implemented:** allow a narrowly scoped, documented correction to the ACES gamut-boundary domain handling, retaining the full ProPhoto limiting gamut. Implement it once in the pinned native generation path so CPU export/reference and generated GPU preview share it; do not hand-edit just the viewer shader. Verify forward-ray geometry against an independent numerical intersection diagnostic, singular/near-singular cases, hue/exposure continuity, neutral/near-black/HDR ramps, both targets and representative camera fixtures. Regenerate/checksum assets only after those checks, and version the output contract so saved projects cannot silently change rendering. Retain separate upstream-reference comparisons with the deviation explicitly identified.

This would depart from the literal ACES 2 reference equations and needs an exception to the existing **no rewritten ACES mathematics** contract. The alternatives are to retain the known-broken custom target while investigating an upstream correction, or explicitly reconsider rendering policy. A narrower-gamut render in a ProPhoto container, saturation compensation, silent bypass of compression, and CPU-only preview are not fixes under the current contract.

## Evidence and verification

Ignored evidence: `.cache/probes/prophoto-gamut-check.txt`, `prophoto-target-isolation.cpp/.txt`, `prophoto-internals.cpp/.txt`, `build-internals.py`, `prophoto-boundary-candidate.cpp`, `prophoto-candidate-internals.cpp/.txt`, and the earlier `prophoto-feedback-diagnosis.txt`.

At this diagnostic stage, the gamut check reproduced the failure while the four existing consistency/profile tests passed; targeted strict Clippy and formatting also passed. The subsequent P3 implementation adds color-preservation regressions and records fresh full-suite/editor/export verification separately. No user-grade reconstruction or photographic-quality acceptance is claimed.
