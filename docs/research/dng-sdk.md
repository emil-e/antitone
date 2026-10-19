# Pre-render DNG SDK evaluation

## Outcome

Adobe DNG SDK 1.7.1 build 2768 supplies the editor's bounded DNG backend directly, without LibRaw's SDK fallback/handoff. It retains original CFA and publishes one immutable camera development through the ordinary shared source consumers. Non-DNG/Sony development remains unchanged. Supplied M9/Q2 and Ricoh GR III/IIIx files develop and cold-roundtrip through shared projects; that is not universal DNG or photographic-quality acceptance.

The important limitations are not just integration effort: the supplied SDK uses bilinear CFA interpolation, can return signed/above-one floating camera samples without opcodes, and clips during opcode processing. It supplies no complete original-sensor dependency map for corrected eligibility. The adapter composes conservative masks for its restricted operation set; broader support needs matching dependency and range treatment, not merely SDK decoder availability. [Remaining acceptance](../plans/raw-lens-correction.md#dng-backend-follow-up) is separate from this evaluation.

## Maintained direct-SDK adapter

The `dng-sdk` feature exposes [`dng::Source`](../../src/dng.rs), the [native adapter](../../native/dng_bridge.cpp), decode-free header/opcode inspection and read-only `antitone-dng-probe`. The editor includes it and selects DNG by decoded header, not extension. [Build/run instructions](../../README.md#direct-dng-sdk-setup) require an explicitly supplied SDK. Core and LibRaw-only builds remain independent.

| Input/operation | Current admission |
| --- | --- |
| Sensor | Unsigned 8–16-bit, uncompressed or lossless-JPEG single-sample rectangular 2×2 RGB Bayer, subject to the precision bound; scalar white and uniform black patterns up to 2×2, no linearization table or row/column black deltas. Authored/real coverage exercises 14/16-bit input |
| Geometry | Native square pixels, bounded main-sensor pixel count supplied by the caller, integral active/default crop, TIFF orientation retained as metadata; no auxiliary image processing |
| OpcodeList1 / OpcodeList2 | GainMap before/after linearization; FixBadPixelsConstant additionally in list 1 for markers at/beyond original black/white limits, with original-site dependency propagation |
| OpcodeList3 | WarpRectilinear, including plane-specific/off-centre/tangential coefficients and ordered repeated warps |
| Unsupported required instruction | Explicit rejection, with no native-decoder fallback or partial result |
| Unknown/future or inadmissible-stage optional instruction | Skipped and reported; recognized supported instructions still validate their parameters and capture/range domain |
| Float/linear/non-Bayer input, other compression, nonuniform black, fractional crop/scale, other operations | Rejected, not silently adapted |

These are current implementation limits, not product requirements or claims of universal DNG support. It currently bounds each list to 64 instructions and each per-pixel warp read rectangle to 64×64 sites. Pixel admission is not a hard process-memory or metadata-parsing budget. In-range bad-pixel markers reject: the current original-limit flags cannot represent their declared invalidity independently of black/white clipping, so accepting them could make synthesized capture samples measurement-eligible.

Development uses SDK linearization into float Stage2 and bilinear Stage3, followed by **one fixed, positive camera WB after every opcode**. Automatic selection reads AsShotNeutral from parsed metadata without the additional six-decimal rounding performed by `dng_negative`; explicit gains override it. Gains are normalized by their minimum, bounded to a 256:1 ratio, and multiplied without another clip. Camera RGB is finite, nonnegative and at most 256 normalized units. The SDK's per-opcode clips remain intact. This is a deliberate DNG development basis, not a reproduction of LibRaw's WB-before-AHD development. `CameraToPCS`, computed for the fixed neutral, has those explicit gains divided out of its columns, producing a matching camera-to-XYZ-D50 matrix without double WB.

The owner retains original pre-OpcodeList1 CFA codes and capture limits separately. Its four conservative flags distinguish original white, original black, repeated-border dependency and intermediate processing limits. Original-site flags survive pointwise gain/linearization; integer bad-pixel repair unions its original 5×5 neighbourhood before a two-site Bayer halo conservatively covers the admitted generic demosaicer. Each SDK warp's per-pixel, all-plane source-buffer bounds propagates the existing flags to the next raster. Out-of-bounds reads mark border dependence and include clamped-edge support. Intermediate zero/white values conservatively mark processing limits after each instruction, so a later attenuation cannot hide earlier loss. Post-WB headroom is not classified as capture clipping. Overinclusive masks deliberately trade eligible area for safe measurement; this is not a general provenance engine for other DNG operations.

SDK transactions remain serialized with process-scoped XMP lifetime. Within one transaction, the admitted SDK warp uses disjoint tile-aligned worker stripes; repair, bilinear and warp eligibility propagation use independent output rows. The editor supplies its existing CPU/pressure budget (at most eight), while standalone `Source::develop` remains serial and `develop_with_threads` accepts an explicit budget. Other SDK area tasks retain serial execution. Small workloads avoid thread startup. Warp concurrency is additionally limited so SDK source/destination tile buffers total at most 64 MiB, or one serial buffer pair if that pair alone exceeds the cap; this is not a full-image/process-memory bound. A non-null SDK progress callback uses serial SDK execution rather than invoking an unproven callback concurrently.

Cancellation uses a borrowed atomic token at SDK sniff points and adapter rows. Worker failures stop peers and join them before rethrowing the original error; SDK task cleanup follows those joins. No callback or worker outlives the synchronous call, and no failed job publishes a source or falls back. A job waiting for the transaction mutex is not immediately interruptible. Completed owners contain plain immutable buffers, not SDK handles, and support shared cross-thread reads/destruction. Retained-byte estimates cover explicit owner/vector allocations, not peak SDK scratch or allocator overhead. Execution budgets/timings do not participate in interpretation identity.

Interpretation IDs cover adapter/SDK version, CFA layout, geometry, original limits, fixed WB/matrix, precision and canonical opcode program. They do not hash capture pixels or establish source-file freshness. The [separate saved SDK contract](../projects.md#compatibility-is-about-the-processing-basis) and per-source identity reject incompatible developments, including old LibRaw-developed DNGs, rather than reinterpret saved calibration.

The precision floor starts with the original code quantum after black subtraction, bounded below by the shared native base floor. Positive WB and gain-map maxima amplify it; list-1 integer gains use ceiling amplification for integer rounding, while repair and cubic warp each conservatively contribute a factor of two. Admitted floors are at most 256 native codes. This is a conservative processing choice, not a camera-noise estimate. Saved calibration keeps the native base floor; residual flat-field gains multiply the source bound in a runtime copy.

Maintained [adapter tests](../../src/dng_tests.rs) and [shared-source tests](../../tests/dng_integration.rs) use the [authored fixture builder](../../tests/support/dng_fixture.rs), including a minimal independent 14-bit lossless-JPEG encoder. They protect opcode order/clips, fixed WB/D50-neutral consistency, repaired original black/white dependencies, sequential warp borders, header-only versus completed-source admission, required/optional failures, cross-thread ownership, cache/project identity and flat-reference matching. The cold-project export test reopens TIFF/AP0 EXR against analytic Bayer samples and scalar processing, including a developed sub-quantum site that is not capture-black. Composed authored editor workers cover import, thumbnails, analysis, repairs and export without private photographs. Software correspondence does not establish photographic quality.

### Supplied-camera evidence

One read-only file per camera was exercised separately through the SDK and shared `Scan`/project path. Dimensions are developed active rasters, before default crop. Local release observations, not latency budgets:

| Camera | Input | Developed dimensions | Cold shared development | Explicit retained source |
| --- | --- | --- | --- | --- |
| Leica M9 | Uncompressed 16-bit, scalar black | 5216×3472 | ~0.24 s | ~260 MiB |
| Ricoh GR III | Lossless JPEG 14-bit, uniform 2×2 black | 6020×4024 | ~0.44 s | ~348 MiB |
| Ricoh GR IIIx | Lossless JPEG 14-bit, uniform 2×2 black | 6020×4024 | ~0.42 s | ~348 MiB |
| Leica Q2 | Uncompressed 14-bit, uniform 2×2 black; required bad-pixel repair and warp | 8392×5632 | ~1.9 s | ~677 MiB |

These observations use the eight-worker execution budget, fresh full-resolution development and warm filesystem caches. Repeated Q2 native timings were 7.8–8.2 s serial, 4.3–4.5 s at two workers, 2.6–2.8 s at four and 1.85–1.94 s at eight. Serial warp eligibility took 3.9–4.0 s versus SDK resampling's 1.95–2.19 s; at eight workers these took ~0.6 s and ~0.35 s. All camera RGB bits, original CFA, eligibility flags and interpretation digests matched across budgets for all four cameras. Standalone full-resolution probes observed peak RSS around 650 MiB (M9), 891 MiB (Ricoh) and 1.8 GiB (Q2); Q2 rose from ~1815 MiB serial to ~1834 MiB at eight workers, with zero reported swaps. These include SDK scratch and are not retained-cache charges or aggregate editor residency. Shared checks cold-reopen the saved fixed WB/geometry/program, compare camera values, create bounded proxies and publish the immutable source into the normal store. The optional `supplied_cameras_share_editor_source_and_saved_project_contracts` test uses `ANTITONE_DNG_SAMPLE_DIR`; it is ignored during ordinary tests. The separate `supplied_m9_editor_import_save_open_and_bounded_exports` test, using `ANTITONE_M9_SAMPLE`, exercises actual M9 import and cold Open through the editor worker and reopens 32×32 TIFF/AP0 EXR exports. Its declared anchors/crop are plumbing inputs, not photographic grading or appearance approval. GUI/physical interaction, negative-scan quality, vendor-reference appearance and real-camera interruption latency remain unestablished. Editor cancellation retains the existing between-native-call publication boundary; standalone SDK sniff-token cancellation is not a promise that a running editor decode interrupts immediately.

## Release, build and licensing

The cached [Adobe download page](https://helpx.adobe.com/camera-raw/digital-negative.html) identifies 1.7.1 build 2768, October 13, 2026, through [Adobe's SDK redirect](https://www.adobe.com/go/dng_sdk). The selected archive's `DNG_ReadMe.txt` and `dng_sdk/source/dng_flags.h` agree. The archive SHA-256 is `c1d3a545ed45b150e59af2e6aa0695c1551f0e2bed34ad87436898a05194d2b1`. Version 1.7.1 alone does not identify the build; an older archive with the same semantic version was not used for these results.

The package supports C++14/17 and includes macOS/Windows validator projects. Its readme lists Xcode 12.4–26 and Visual Studio 2022 with clang. The local Apple-Silicon release validator and probes build using Apple Clang, arm64 and a macOS 14 deployment target. Initial tool selection picked a missing Homebrew GCC; explicitly selecting Apple's compiler/linker resolved that without installing anything. The successful build had unrelated simulator-version warnings. A deployment target is not testing on macOS 14 or a clean machine.

The validator project includes libjpeg, libjxl (with Brotli/Highway) and XMP (with additional dependencies including Expat/zlib). The evaluation reused its compiled objects, excluding validator `main`, for two C++17 probes. It separately built pinned LibRaw 0.22.0 with `USE_DNGSDK`, a caller-supplied host and the existing local OpenMP dependency. Shared Antitone dependency builds were not modified. This proves a local build route, not a minimal production dependency selection or distributable package. Production setup belongs in the [normal native workflow](../../README.md#build-and-run), not in a second bootstrap path.

The archive's `LICENSE.txt` grants worldwide royalty-free use, modification, distribution and sublicensing, but it is not an unqualified permissive-license conclusion:

- Using/downloading/installing the package constitutes acceptance under its notice.
- Copyright and other notices must be preserved; human-readable distributions must include the notices.
- Documentation copying/distribution has separate terms and documentation may not be modified.
- Commercial distribution includes an Adobe defense/indemnity obligation; trademark use is restricted.
- Bundled dependencies have their own licenses/notices. The SDK license does not replace their review.

Release needs an explicit license/dependency inventory and assessment of the commercial-distribution terms. This evaluation neither establishes legal clearance nor bundles the SDK into Antitone.

## Experiment and evidence boundary

All sources, builds, authored DNGs and results are disposable worktree-local files under `.cache/dng-sdk-evaluation/`; no private photograph was needed. The experiment has 21 small, uncompressed 64×64 Bayer DNGs, with active area `(top=4, left=6, bottom=60, right=62)`, default crop origin `(x=3, y=5)` and size `(48,46)`. Most declare black 64, white 4160 and spatially constant camera-channel values `(0.25,0.375,0.5)` after black subtraction; geometry cases add an affine ramp. A float pair declares camera values `(-0.25,0.375,1.5)`.

Two direct modes were compared: stock integer Stage2 for integer input, and an experimental `dng_negative::DoBuildStage2` override allocating float Stage2 while reusing SDK linearization. The latter tests precision without patching vendor source and is the maintained adapter's selected Stage2 mode. Both retain Stage2 for inspection and clone Stage1 before processing. Neither calls `dng_render`, performs output-space conversion or runs LibRaw demosaicing after SDK Stage3.

The checked runs comprise 42 direct fixture cases, eight injected-cancellation cases, 42 LibRaw fixture cases and independent geometry/range/capture/handoff checks. Local replay after these builds is `fixtures.py`, `run_probes.py`, `run_libraw.py`, then `check_boundaries.py` in that cache directory. `results.json`, `summary.json`, `libraw-results.json` and `boundary-summary.json` retain the detailed evidence. These are exploratory checks; the maintained prototype's authored tests are separate from that cache evidence.

## What the stages actually expose

| Boundary | Observed result and implication |
| --- | --- |
| Stage1 | Original 64×64 single-plane sensor samples before OpcodeList1. A clone remains bit-exact throughout processing; `BuildStage2Image` releases the original Stage1 object in these runs. Retain capture authority before calling it. |
| Stage2 | Active area becomes a zero-origin 56×56 CFA. OpcodeList1 runs before linearization/black subtraction; OpcodeList2 follows. Integer input is ordinarily normalized into u16; the override avoids that quantization. |
| Stage3 | 56×56 three-plane camera RGB, before WB, camera-matrix conversion or baseline-exposure rendering in these single-plane Bayer cases. Default crop remains metadata, here `[top=5,left=3,bottom=51,right=51]`; orientation 6 does not rotate the samples. |
| Original exposure basis | Changing AsShotNeutral and adding three stops of BaselineExposure leaves Stage3 values unchanged; `Stage3Gain` is 1 for these fixtures. Other DNG layouts, including multi-sample CFA, were not established by that check. |
| Linearization | A declared doubling table with correspondingly doubled black/white levels reproduces the base result. Raw code, linearized value and normalized camera sample are distinct quantities. |

The stock base result differs from the analytic values by at most `7.63e-6` normalized; float Stage2 is exact for these binary-fraction fixtures. Stock gain cases stay within one normalized u16 code of analytic expectations. These results establish stage placement/precision for the declared cases, not general photographic accuracy.

Fixed roll WB would have to be explicitly applied at a chosen, stable boundary, with matching camera conversion metadata. Leaving the SDK unbalanced does not by itself reproduce the current WB-before-AHD developer; applying gains after a per-channel warp or nonlinear/clipping operation is not generally interchangeable with applying them before it.

## Opcode order, geometry and range

Required GainMap (ID 9) works before demosaicing. A gain in OpcodeList1 acts on encoded samples before black subtraction, unlike the same gain in OpcodeList2. Two gains of ×4 then ×½ yield `(0.5,0.5,0.5)`; reversing them yields `(0.5,0.75,1)`. Both integer and float Stage2 show this. Fusing the gains into ×2 would be wrong because clipping occurs between opcodes. DNG specification 1.7.1, chapter 7, explicitly describes ordered processing and per-opcode logical-range clipping; retaining Sony shading headroom is a separate development choice, not a reason to silently alter DNG semantics.

Identity WarpRectilinear (ID 1) is exact. Common radial, three independent plane coefficients, and off-centre `(0.35,0.65)` radial-plus-tangential cases run successfully. Independent chapter-7 coordinate equations and an affine camera ramp check 4,800 interior channel samples per case. Maximum normalized discrepancies are `1.117e-4`, `1.117e-4` and `1.021e-4` respectively. The bound `1.173e-4` accounts for the SDK's 32-phase coordinate table, the first-moment error of its cubic kernel (`A=-0.75`), ramp slope and float accumulation; this is not a bit-exact resampler oracle. The unwarped interior ramp is exact. Edge quality, arbitrary coefficients and photographic lens accuracy remain untested.

Source inspection explains consequential behaviour in `dng_lens_correction.cpp`: the SDK uses source-image bounds to locate the normalized optical centre, bicubic reconstruction, clamps mapped positions to the source border, and pins reconstructed values into `[0,1]`. It does not implement Sony's conservative valid rectangle/no-replicated-border convention. The maintained source keeps default crop metadata and marks repeated-border dependencies explicitly rather than adopting Sony's trimming convention.

Float-input camera samples retain `[-0.25,1.5]` through Stage2 and Stage3 **without opcodes**, but the nonidentity warp produces `[0,1]`. Float storage does not imply unclipped reconstruction, and unwarped signed camera values do not fit Antitone's present nonnegative continuous-source admission. A bounded initial input domain or a deliberate signed-camera design is needed; adding a hidden pedestal/clamp is not an adequate adaptation. No all-opcode or malformed-input coverage is claimed.

Unknown/future-version optional opcodes are skipped and match the base. Unknown/future required opcodes throw directly. Successful decode alone is not evidence that all required instructions executed.

## LibRaw's existing integration

The SDK-enabled LibRaw fixture runs request Stage3, permit size changes, and clear `LIBRAW_RAWOPTIONS_CONVERTFLOAT_TO_INT`. On the 38 supported direct cases, LibRaw's `color3_image`/`float3_image` handoff matches direct Stage3 exactly. It resets black to zero and maximum to 65535, changes raw/active dimensions to 56×56, clears CFA filters and does not expose original CFA in `raw_image`. Normalized float values are still normalized float values despite the integer maximum metadata. The present integer-Bayer bridge cannot simply consume this as ordinary LibRaw unpacking.

Two concrete hazards favour direct SDK ownership:

1. **Required-opcode failure becomes successful unpack.** `try_dngsdk` catches the SDK exception and marks DNG not processed; LibRaw then falls back to its native CFA decoder. The four unknown/future-required cases return successful unpack with uncorrected CFA. A caller would have to require both SDK-processed and Stage3-applied status and reject fallback; cancellation must not silently take that path either.
2. **Default float-to-integer conversion is unsafe for this handoff.** With float Stage2 and LibRaw's conversion option retained, `convertFloatToInt` allocates/iterates using the original TIFF sample count (one), while advertising three-plane Stage3 RGB. The guarded probe finds an 8,192-byte allocation where 18,816 bytes are needed. It stops before reading that buffer as RGB. Conversion also introduces a different normalization/quantization. This result is specific to pinned LibRaw 0.22.0 and the tested configuration; no shared dependency patch or upstream action was made.

These are objections to adopting the stock integration blindly, not evidence that reusable integration is impossible. A validated custom host, disabled conversion, explicit failure gating and independent capture ownership could address parts of it, but remove much of its apparent simplicity for this boundary.

## Capture authority, execution and resources

Original CFA access is feasible: cloning before OpcodeList1 preserves the actual codes, including a sensor-white marker at code 4160. Its warped developed peak is only `0.9994564`; testing corrected RGB against white would miss it. Keep original capture eligibility separate from brightness, opcode clipping and border extrapolation.

A custom host can observe filter `SrcArea` rectangles. The off-centre warp reports `[top=26,left=25,bottom=32,right=31]` for destination `(28,28)`, covering interpolation support across planes. This is an intermediate read bound, not an original-CFA eligibility map. `dng_mosaic_info::InterpolateGeneric` directly tiles and bilinearly interpolates with CFA-pattern padding and edge repetition rather than routing those dependencies through the host's filter-task hook. Correct masks need conservative composition through active-area rebasing, demosaicing and every supported opcode, including earlier geometric/repair operations. No general provenance propagation was established here.

The default host's area-task implementation is serial and reports one worker. It can be overridden, so a future integration should supply an explicit execution policy rather than inherit an unknown thread pool. The abort sniffer throws cooperatively; stops at sniff calls 1–4 cover read, linearization, demosaicing and warp work in the small warp fixture. They terminate with user-cancelled status and no completed Stage3 publication. This establishes those checkpoints, not bounded interrupt latency for codecs, arbitrary files or LibRaw fallback. Global XMP initialization/termination also needs process-level lifetime ownership.

The tiny-fixture runtimes do not predict real-camera latency or aggregate memory. Original CFA, Stage2, prewarp Stage3 and warped Stage3 can overlap; float Stage2 and an independent capture clone add full-image allocations. For scale, 50 million three-channel f32 pixels alone are 600 MB (572 MiB), before those overlaps and SDK scratch. Production needs measured high-resolution peak residency, interruption latency and frozen-source charges. The inspection probe deliberately retains additional snapshots and is not a resource baseline.

## Remaining acceptance

Photographic assessment of the post-opcode WB/bilinear basis, edge/repair quality and vendor-reference correspondence remains separate from successful decode and numerical/export consistency. Real-camera interruption latency, mixed-roll aggregate pressure and broader hardware/firmware coverage remain open. Broader sensor/opcode admission should follow concrete inputs with maintained capture/range coverage.

Licensing, notice inventory and dependency/build minimality remain release work. The local validator-derived SDK dependency is not a reviewed distributable package; application integration does not waive those obligations.

### Source references

- [Adobe DNG SDK and specification](https://helpx.adobe.com/camera-raw/digital-negative.html): selected archive `LICENSE.txt`, readmes, `dng_flags.h`; specification 1.7.1 chapter 7, pp. 105–107 (order/range and WarpRectilinear).
- Selected SDK `dng_negative.cpp`: `DoBuildStage2`, `BuildStage2Image`, `DoInterpolateStage3`, `BuildStage3Image`; `dng_mosaic_info.cpp`: `InterpolateGeneric`; `dng_lens_correction.cpp`: `dng_filter_warp`; `dng_gain_map.cpp`: `dng_opcode_GainMap::ProcessArea`; `dng_host.cpp` and `dng_abort_sniffer.h`: execution/cancellation hooks.
- Pinned [LibRaw 0.22.0 integration](https://github.com/LibRaw/LibRaw/blob/0.22.0/src/integration/dngsdk_glue.cpp), [float conversion](https://github.com/LibRaw/LibRaw/blob/0.22.0/src/decoders/fp_dng.cpp) and [SDK integration notes](https://github.com/LibRaw/LibRaw/blob/0.22.0/README.DNGSDK.txt).
