# Film-response integration: GPU precision and RGB16 source gate

## Status

Historical AP0/model-2/3 response/source checkpoint. Its compatibility and source-space conclusions were superseded by the [native-camera replacement](../plans/completed/camera-inversion.md); current model-4 behavior and tolerances are in the [processing contract](../contracts/processing-model.md). The measurements below preserve their original scope, not current-model certification.

The user approved the production integration plan, including a separate toe/shoulder section, logical section ordering and the tested signed continuation. The initial numerical gate led to the explicitly approved compact RGB16 source integration below. The scalar/state/GPU/control/export response integration is now complete for the tested domain. New grades explicitly use model 3; model-2 grades remain original. Physical-monitor acceptance and a universal domain proof remain open.

**Rounding the converted AP0 source to one f32 loses needed near-Dmin differences. It does not follow that the GPU must store a wider source.** LibRaw's developed RGB16 contains all the source information. The user asked to retain those integer samples and perform accurate conversion on the GPU, then explicitly authorized an isolated confirmation. That compact-input prototype now passes the tested source/new-response gates: RGB16 stays at 8 bytes/pixel; the extra arithmetic precision exists only in shader temporaries and small matrix/calibration constants. This supersedes the proposed 48-byte AP0 source cache.

Production source uploads now use RGB16 with on-the-fly conversion. The superseded float-AP0 upload API was removed, not kept as a compatibility path. Original model-2 processing/grade defaults, schema 3, ACES assets and development/output contracts remain unchanged. New grades use model 3 / toe 2.5 / shoulder off, with actual-response export metadata.

## Production film-response checkpoint

The remaining prototype failures were response/colour-transform arithmetic precision, not missing RGB16 source information. `film-wide-precision.rs` retains the same 8-byte source and exact conversion, but splits small response/post constants and evaluates logs/exponentials and AP1 sums using compensated f32 pairs with explicit FMA. This passes **all 57 accepted / 15 rejected** initial configurations, including original-formula beta 1 / gamma 3 / +16,+32 EV. Tolerances are unchanged; floor flags remain exact. Another **720** configurations span three source matrices, intermediate toe/shoulder/gamma values, extreme/intermediate roll balances, photo WB and fractional/endpoint exposures: **531 accepted / 189 domain-or-range rejected**, zero failed channels/flags, worst tolerance multiple `2.979836448e-4`.

Production uses that arithmetic for **model 3**, including its beta-1/shoulder-off setting. The original/model-2 shader deliberately keeps its prior execution/grades, not a silent numerical rewrite. Thus the candidate's cancellation failures are resolved; the known model-2 expanded high-EV limitation remains documented below, not claimed fixed for preserved original grades.

- Source remains RGB16/8 bytes. Calibration/source temporaries preserve near-Dmin differences; response/colour temporaries use pairs. Working/target textures remain f32 and presentation RGBA16Float. No wide source/intermediate cache, float-upload API or CPU-viewer fallback was added. The cached processing signature is now 224 bytes; immutable camera constants remain 144 bytes.
- Scalar/model identity: new-grade factory/model 3 beta 2.5/off; schema-3 absent response retains Original/model 2. Explicit opt-in and exact f64 shape round-trips; model/response mismatches reject. Beta 1/off is bit-identical to original scalar execution. Actual-response full-source fitting preserves unrelated state. TIFF/EXR identify actual model/shape; original metadata is unchanged.
- GPU preflight rejects inverse poles, unsupported f32 ranges/precision constants and floor response exponent >4096, including poles concealed by Dmax=floor. Floor-anchored 80/400/1000/4000 exponents pass. Production sweep: **291 accepted / 69 domain-or-range rejected** over 520 RGB16 samples each, three matrices and shape/gamma/balance/Photo-WB/EV combinations, worst tolerance multiple `2.978629948e-4`, no flag differences.
- Production real-camera probe: **5,120 full-resolution dog fur/sky/wood samples × 7 grades**, unchanged f64 working tolerance; 14 complete sRGB/P3 comparisons. Model-3 ordinary-grade working max error <`6e-8`; including +32 EV, errors are ordinary f32 output rounding (worst tolerance multiple <`2.965e-4`). Target-linear max `3.159046173e-6`, encoded max `4.194800936e-6`, zero GPU-black/CPU-nonblack cases, exact floor flags. Both known original-formula extreme cases pass in model 3. This is diagnostic Dmin/settings, not the unknown exact user session or a calibrated NLP match.
- Original real-camera regression remains `1.33514404e-5` working; 4,816 complete-rendering pixels/target retain sRGB `3.75509262e-6` / P3 `2.86102295e-6`. ACES/profile/development assets are unchanged.
- **88 all-feature tests**, strict all-target Clippy, formatting and release build. Scalar anchors/monotonicity/forward-density/signed continuation, actual-model fitting, original/new project round-trips, TIFF/AP0 EXR pixels/metadata, original opt-in, true resets/order/fine shoulder stepping/scrolling/pinned Output all pass. Independent handoff review found only probe logs reporting latest model 3 while executing original defaults; both probes now report the actual settings' version.
- M5 editor smoke: 14.61 s / 567 frames, full-source calibration/both targets/source bypass/refinement/reference switching/return. Layout smoke (including four holds) 31.49 s / 1,576 frames; standalone fit refinement **2.572 s** for 8640×5760 → 1538×1026 (0.069 s sampling / 2.491 s GPU), native region 0.164 s. The first 3.065-s fit run overlapped another GPU probe and is not a controlled cost comparison. Normal/narrow screenshots and event tests inspected; lower Roll response/exposure rows are scrollable while Output remains pinned.

The isolated wide-response microbenchmark is approximately **2.6 ms per 1024×256 tile**, including submit/drain but excluding ACES/resampling/UI, versus about 0.85 ms for the earlier narrower prototype. This precision cost is in arithmetic, not retained source/output memory. macOS-14 and physical-monitor validation remain unavailable; finite M5 evidence is not blanket domain/device acceptance.

## Historical compact-input alternative

The new ignored `camera-u16-precision.rs` / `camera-u16-precision.wgsl` probe starts after unchanged LibRaw development, not from the original CFA mosaic. The native camera image has four interleaved u16 slots per pixel; RGB occupy the first three. Two packed u32 words preserve that 8-byte stride, and the shader extracts each RGB integer exactly. The fourth slot is unused by processing.

Per pixel, the GPU normalizes the integers, applies the camera→AP0 matrix with temporary f32 expansions and explicit fma, then performs inversion/photography and writes one ordinary f32 AP1 result. The source conversion rounds after each operation to match the scalar f64 normalization/matrix path. It does not add photographic information, change LibRaw development or widen the working/output buffers. Conversion-only readback of expansion components is a diagnostic, not a proposed retained intermediate image.

### Numerical evidence

- Reopened/developed DSC04393 with the same fixed WB and camera→AP0 matrix. Extracted 3,072 actual RGB16 fur samples; their CPU AP0 values agree exactly with the archived full-f64 source fixture.
- Added 69 integer-source tests: near-Dmin calibration perturbations, exact Dmin/Dmax anchors, full-scale/zero/saturated camera channels and floor comparisons at ±1e-12/exact equality. Since arbitrary fractional AP0 rays are not necessarily representable as RGB16, the near-base tests keep the integer source fixed and vary Dmin instead; no real source data are rounded or excluded.
- GPU source conversion matches the CPU **bit for bit** over those 3,141 samples and another 131,072 synthetic RGB16 samples: every neutral code 0..65535 plus 65,536 seeded coloured tuples. Maximum reconstructed AP0 error is zero over 402,639 checked channels. The converter's high component also matches the existing f32 AP0 source bits exactly across this finite set, supporting an unchanged legacy input path.
- The same 72 shape/gamma/exposure combinations yield 57 accepted / 15 explicitly domain-or-range-rejected configurations. All **46 non-legacy response configurations** pass the unchanged `2e-5 + 2e-4*abs(f64_reference)` working gate; all floor flags are exact.
- The two existing legacy gamma-3 / +16,+32-EV AP1-cancellation discrepancies remain, at the same worst tolerance multiples as the earlier converted-source prototype. The probe deliberately exits 1 for these failures; the complete expanded numerical gate is **not** declared passed.

This is a finite Sony-matrix/source and working-value gate, not a full parameter-domain or complete rendering/editor gate. The temporary conversion implementation assumes normal-range camera values/matrix products; other metadata/matrices and the integration's GPU representability checks still need validation. No Photo WB sweep is included here.

### Measured local cost

M5 Pro / Metal, one 1024×256 tile, beta 2.5 / gamma 0.65 / shoulder off / combined +3 EV. Latest run medians:

| Measurement | Packed RGB16 + on-the-fly conversion | Cached three-part AP0 prototype |
| --- | ---: | ---: |
| Retained source bytes/pixel | 8 | 48 |
| Source tile | 2 MiB | 12 MiB |
| CPU source packing | 0.098 ms | 1.719 ms |
| Upload plus queue drain | 0.604 ms | 2.111 ms |
| Compute, submit and drain per dispatch | 0.848 ms | 0.076 ms |

The superseded single-f32 AP0 tile was 4 MiB: packed RGB16 **halves source storage**, rather than tripling it. The 4-MiB working buffer and target/presentation texture formats are unchanged. Under the existing two-preview allocation, source storage would likewise halve; already-rendered settled output should have no source allocation.

CPU timings exclude decode/demosaic and compare straightforward RGB16 serialization with CPU matrix conversion/three-part AP0 packing. GPU compute timings use five dispatches per batch after warm-up; they include host submission/wait overhead and exclude ACES rendering, resampling and UI presentation. On-the-fly accurate conversion costs about 0.8–1 ms per tested tile on this machine, while packing/upload costs fall. These are microbenchmarks, **not end-to-end interactive performance**; do not sum separately measured medians into a claimed editor latency.

## Production RGB16 upload checkpoint

`src/raw.rs` exposes exact developed integer pixels; `src/scan.rs` extracts integer preview/tile payloads. `ProcessingSource` owns only that 8-byte/pixel input plus a 144-byte immutable matrix uniform. `camera_conversion.wgsl` implements temporary precision; the model-2 shader consumes the same f32 AP0 value as the former CPU-cast path and retains its original inversion/photographic equations. There is one production source format, not dual float/integer kernels or adapters.

`GpuImage::new_rendered` omits source, working, compute and unused ACES-processing resources. Provisional images and reusable refinement tiles retain RGB16; actual settled outputs retain only target/presentation resources. Source replacement invalidates the parameter cache. CPU full-resolution calibration, fitting and exports remain unchanged.

Production tests extend conversion/readback to 393,216 RGB16 samples over identity, the Sony camera basis and a signed/HDR test basis; both f64 conversions and original f32 source bits match exactly. Matrix preflight rejects unsupported range, lossy parameter decomposition, subnormal constants and coefficients whose smallest nonzero integer-code product would underflow. Independent handoff review identified the underflow case; an explicit-rejection regression now covers individually underflowed terms with a nonzero aggregate f32 result. No gate was relaxed.

All **72 all-feature tests**, strict all-target Clippy, formatting and release build pass. The regenerated 3,072-camera/4,816-complete-rendering probe retains max working error `1.33514404e-5`, sRGB error `3.75509262e-6` and P3 error `2.86102295e-6`. Cached full-source/grid-centre sampling checks pass. The M5 real-image editor smoke passes in 13.64 s / 519 frames with full-resolution calibration, GPU edits/both targets/source bypass, fit/native-pixel refinement and reference source switching/working-photo return. Fit P3 refinement is 2.174 s for 8640×5760 → 1538×1026; native 1538×1320 refinement is 0.135 s. This is not physical-monitor acceptance or a controlled comparison to earlier view dimensions.

The two expanded original-model extreme-EV discrepancies still block a claim of complete domain parity. They are not caused by storage migration and are not hidden by keeping old source APIs. Toe/shoulder production state/math/controls are not enabled in this checkpoint.

## Historical converted-source probe conditions

Ignored `toe-precision.rs` and WGSL prototype evaluate:

- The corrected whole-picture dog reference, screenshot-rounded Dmax and existing floor.
- Global gamma `0.1`, `0.65`, `3`; the actual fitted current-model balance from the corrected image pass.
- Toe beta `1`, `2.5`, `4`; shoulder `0`, `0.02`.
- Combined exposure `-32`, `+3`, `+16`, `+32` EV. Combined ±32 means each existing Roll/Photo scope is at its permitted ±16 endpoint, not that either scope supports ±32 alone.
- 3,072 actual full-resolution fur samples, plus 57 constructed near-base/anchor/floor samples. Near-base rays include ±1e-3, ±1e-5, ±1e-7, ±1e-8, ±1e-9, ±1e-12, exact base, and single-channel perturbations.
- Existing AP0→AP1 conversion, no Photo WB. This is a working-value gate, not complete display rendering or a full parameter-domain proof.
- Existing per-channel working tolerance `2e-5 + 2e-4*abs(f64_reference)`, with exact floor flags. Gates are not relaxed.

Of 72 parameter combinations, 57 pass the f64 floor-domain/f32-range preflight; 15 are rejected for a shoulder pole or unsupported range. Rejecting them is explicit, not a fallback or silent clamp.

## Identified loss in the superseded float-AP0 upload

At beta 2.5, gamma 0.65, shoulder zero, combined +3 EV, a red-only source perturbation about 1e-8 below measured base is:

```
original red scan = 0.1685663484690062
reference Dmin R = 0.1685663501546697
```

Both become the same single f32 value when uploaded. The GPU therefore sees exact Dmin and returns zero, whereas the f64 reference gives working AP1 approximately:

```
[4.33198e-5, -2.28483e-6, 2.48205e-7]
```

That exceeds the unchanged absolute tolerance. At larger toe/gamma settings, the discrepancy grows. Comparing the shader against a control reference using the same single-f32 inputs/parameters passes the new-shape cases: changing only the shader's inverse cannot recover information already rounded away at upload.

The new candidate beta-2.5 and beta-4 cases pass all 3,072 actual fur samples in this finite sweep. The failures are in the deliberately close base-boundary rays. They must nevertheless not be silently dropped from the gate.

## Tested higher-precision representation

The experiment keeps each source/calibration value as high plus residual f32 components:

1. Two components preserve more source information and resolve ordinary +3-EV cases, but some ±1e-12 boundary rays still fail at large exposure.
2. Three components preserve the original f64 source/ref differences more accurately. A stable log-ratio series over `abs(relative_difference)<0.5` avoids subtracting two nearby rounded logarithms.
3. Explicit `fma(1, tail_difference, high_plus_low_difference)` retains the final correction in the complete kernel. With ordinary final addition, the full kernel still failed some extreme boundary tests; the explicit fused operation resolves them.

A separate delta readback verifies that the stored high/low/tail data and their f32 difference components represent the original near-base difference. It does not prove precisely which compiler transformation affected the full inverse. Installed wgpu/Metal sources use default compile options, whose documented fast-math defaults permit reassociation; that is relevant context, **not proof of a compiler bug**. No private Metal compilation hook or dependency fork is used by the working prototype.

The final three-component/fma prototype passes all 46 tested non-legacy response combinations: beta 2.5/4 at either shoulder amount, plus beta 1 with shoulder 0.02. This includes near-base rays, all actual fur values, floor/anchor checks and supported exposure endpoints. There are no wrong floor flags. This is evidence of a fixable source/arithmetic precision requirement, not full implementation acceptance or proof over every intermediate setting/calibration.

## Separate pre-existing extreme-exposure finding

Beta 1/shoulder zero uses the original formula as the reference. The expanded sweep finds per-channel tolerance failures in the existing-style GPU path at gamma 3 and combined +16/+32 EV, including a sampled fur pixel with near-cancelling AP1 components. The original single-f32 prototype already fails those cases.

After the higher-precision source experiment, one channel still fails at each of those two configurations. Trying the generalized log evaluation instead of the legacy inverse does not remove that discrepancy. It is not the same as the original input-rounding loss, and is not declared fixed or silently swept into a new tolerance.

All tested legacy combinations at ordinary +3 EV pass. Existing production regression fixtures are untouched; these newly expanded extreme probes are separate evidence, not a claim that the previous finite gates failed. Legacy execution/grade preservation and the remaining extreme discrepancy must be handled explicitly before claiming a full-domain production gate.

## Scope consequence

The preferred integration layout is now packed **LibRaw-developed RGB16**, converted accurately on the GPU, not three retained AP0 float rows. Keep the full-f64 CPU reference, existing tolerances, signed policy and unchanged output precision.

The approved compact source, versioned scalar/state, actual-model fitting, precise model-3 GPU path, controls and response metadata are implemented and verified above. No superseded float-source API/caller remains. Original grades keep their execution and documented expanded-domain limits; opting into model 3 is explicit. No tolerances/reference precision were weakened. ACES, profiles, LibRaw development and flat-field correction remain unchanged. Broader device/domain/physical acceptance and deferred saved-editor work remain separate.

## Evidence

Preserved Rust sources in `docs/research/tools/film/src/bin/` ([status/reproduction](tools/README.md)). Some are historical API snapshots; production and wide-response probes are checked separately:

- `toe-precision.rs`: ordinary single-f32 upload, unchanged formula control, quantized-input control reference.
- `toe-precision-split.rs`: two-component source/reference experiment.
- `toe-precision-triple.rs`: three-component experiment.
- `toe-precision-debug.rs`: direct delta-component readback.
- `camera-u16-precision.rs`: initial compact developed-camera RGB16 prototype, including its separate original-formula failures.
- `film-wide-precision.rs`: resolved initial/broader candidate arithmetic gate and timing.
- `film-production-check.rs`: actual production scalar/GPU/complete-rendering comparison on 5,120 full-resolution camera pixels / seven grades.

WGSL source is preserved under `docs/research/tools/film/`. Selected final logs are retained under [evidence/film-model-images](evidence/README.md); other original logs and photographic outputs remain local under `.cache/probes/film-model-images/`. The original artifact names were:

- `toe-precision.wgsl`, `toe-precision.txt`, `toe-precision-build.txt`
- `toe-precision-split.wgsl`, `toe-precision-split.txt`
- `toe-precision-triple-fma-legacy.wgsl`: tested final three-component/fma kernel with the legacy-formula branch.
- `toe-precision-triple-fma.txt`: final new-response success and the two separate legacy failures.
- `toe-precision-triple.wgsl`, `toe-precision-triple-fma-general.txt`: generalized-evaluation control, which still exposes the legacy-like cancellation discrepancy.
- `toe-precision-debug.wgsl`, `toe-precision-debug.txt`: uploaded/difference-component checks.
- `camera-u16-precision.wgsl`, `camera-u16-precision.txt`, `camera-u16-build.txt`: isolated compact-input prototype, complete numerical/timing results and build log.
- `rgb16-{all-tests,clippy,release-build,conversion-tests,gpu-viewer-tests,camera-fixture,complete-gpu-chain,cached-scan,editor-smoke}.txt`: preceding source-upload checkpoint verification.
- `film-wide-{build,precision}.txt`, `film-wide-precision.wgsl`: candidate arithmetic gate.
- `film-production-{build,check}.txt`: production real-camera/complete-rendering gate.
- `response-{all-tests,clippy,release-build,gpu-tests,scroll-test,original-camera-fixture,original-complete-chain,editor-smoke,controls-smoke}.txt` and `response-controls-{roll,photo}-{normal,narrow}.png`: final production verification/layout evidence.

The gate bins deliberately exit nonzero when any tested accepted configuration violates tolerance. These local probe failures are not CI failures. Use the named-bin/local-toolchain build pattern in [the tools guide](tools/README.md) and the checkout-local OCIO runtime library path. Historical pre-integration bins are not automatically compatible with current APIs; do not build the entire research crate as a production test suite. No RAW samples are modified.
