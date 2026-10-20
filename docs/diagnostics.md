# Specialist diagnostics

Use [development checks](development.md) for the ordinary contributor loop. These recipes investigate native, numerical, image or presentation boundaries; they are not prerequisites for every change.

Run from the repository root after [setup](../README.md#build-and-run). Examples use `--offline` with populated caches. Private RAWs are read-only local inputs, not downloadable fixtures. Keep generated products under ignored `.cache/` and use fresh export/report directories.

Choose a diagnostic for a concrete question. A command's presence here does not make it a routine gate. Numerical agreement, offscreen input, a visible native window and physical photographic acceptance are different evidence. Timings and captures are observations, not acceptance thresholds. Sample-dependent checks can be expensive; do not run this entire page reflexively.

## Editor and layout checks

Build and open a saved project for real-use inspection:

```sh
cargo build-app --offline
target/release/antitone /path/to/grade.antitone
```

Calibration, grade/source controls, physical-pixel viewing, picker-source return and normal/narrow layouts use authored [framework UI tests](development.md#framework-ui-regressions). Project tests cover Save/New, transition decisions, restored settings/pixels and failed-Save/Open preservation with real workers and temporary files. Native dialogs and photographic Save/Open inspection remain real-use checks.

### Viewer-tool evidence

Framework tool tests render Crop/Dmin/WB and flat-field support at normal/narrow sizes. Optional `ANTITONE_UI_CAPTURE` products are offscreen evidence, not native-window appearance approval. [Native-host checks](development.md#explicit-native-host-checks) separately exercise focus, size forwarding, surface paint and completion/close failures. Do not automate Lightroom/Resolve or bypass window occlusion.

## Flat-field correction

For authored reference/area transitions, corrected picks, both targets/thumbnails, Enabled/Clear and Save/Open:

```sh
cargo test --locked --offline --features ui-tests --lib \
  editor::kittest_tests::correction -- --test-threads=1
cargo test --locked --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

The second exercise exports/reopens full-size images and is more expensive. These check implemented [correction](flat-field.md), not physical reference suitability or native-dialog gestures. Inspect a supplied negative/reference pair through the normal editor.

## Analyze roll

Use [authored analysis checks](development.md#roll-analysis) for transaction and estimator regressions. They cover per-photo review and exception edits, normal/narrow layouts, preparation/analysis cancellation and failure, atomic support/grade installation, Save/Open and both targets, including real correction. They are offscreen; [native-host completion checks](development.md#explicit-native-host-checks) are separate.

An optional regression checks opening recovery and holder-tail exclusion on one twelve-frame feedback scan set:

```sh
ANTITONE_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib real_ambiguous_first_frame_uses_roll_opening_instead_of_holder \
  -- --ignored --nocapture --test-threads=1
```

Its named files and area/Dmax assertions apply only to that case. Other supplied rolls use an explicit manifest or the editor; neither route certifies [photographic success](plans/roll-assistance.md) or a general opening detector.

### Retained area evidence and complete native review

The optional retained-evidence comparison pins native line extraction, the frozen **local** rectangle baseline and canonical original-centre membership. The current shared-holder provider intentionally differs from that local baseline. This check does not redevelop RAWs or certify material truth:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence ANTITONE_REGION_NATIVE=1 \
  cargo test --locked --offline --release --features raw,project --lib \
  retained_rectangle_baseline_parity -- --ignored --nocapture
```

Compare the current shared-holder provider with that frozen local baseline across the retained corpus, writing fresh private reports under `.cache/`:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence \
ANTITONE_REGION_OUTPUT="$PWD/.cache/holder-comparison" \
ANTITONE_REGION_IMAGES=DSC04331,DSC01635 \
  cargo test --locked --offline --release --features editor --lib \
  compare_shared_holder_with_frozen_local_baseline \
  -- --ignored --nocapture --test-threads=1
```

The output directory must be new and outside the frozen evidence tree. The comparison checks expected shared-support/abstention counts, one common opening for normal photos, and spatial population adequacy before numerical rejection. Optional PNGs show the old local opening in red, new conservative opening in gold and picture statistics in cyan, over ungraded camera-code previews. This is evidence for software behavior, not photographic appearance; recurring scene darkness, bounded resolution and unobserved edges remain limitations.

For one explicitly named complete retained roll, the Editor worker regression performs real native preparation, temporary browsing, Analyze, return to the working photo, Save/Open and accepted-support reuse. It preserves RAW size/mtime snapshots and writes only temporary project data under `.cache/`:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence \
ANTITONE_RAW_ROOT=/absolute/read-only-samples ANTITONE_REGION_ROLL=roll-name \
  cargo test --locked --offline --features editor --lib \
  complete_private_roll_prepares_reviews_analyzes_and_round_trips \
  -- --ignored --nocapture --test-threads=1
```

These inputs remain private and unshipped. Neither software parity nor a successful numerical proposal establishes photographic quality, boundary accuracy or physical reference compatibility.

### Bounded area-preparation profiling

The ignored `profile_complete_roll_area_preparation` diagnostic calls the production preparation provider directly, without creating an Editor, developing thumbnails, installing areas/grades or launching a window. It uses the existing headless resource monitor (an idle Metal device supplies allocation telemetry), requires Normal pressure and 2 GiB non-speculative free memory, and retains the existing swap/footprint/stale-telemetry stops. Each invocation is bounded to one complete 2–40-source roll and 240 seconds at cooperative checkpoints; native calls cannot be interrupted mid-call. Do not truncate a larger roll or relax stops to obtain a result.

Run the two arms as separate processes, with fresh output directories under this checkout's `.cache/`. The cold arm means an empty application source store, **not** an uncached filesystem. The warm arm seeds only the last ordered source under a 600 MiB optional budget before timing preparation; it is a partially warm document, not a fully resident roll. Analysis still requests no additional native retention. Source revisions, ordered entries and retained evidence must match the cold report exactly.

```sh
mkdir -p .cache/region-scratch
export TMPDIR="$PWD/.cache/region-scratch"
export ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence
export ANTITONE_RAW_ROOT=/absolute/read-only-samples
export ANTITONE_REGION_ROLL=roll-name
ANTITONE_REGION_PERF_ARM=cold ANTITONE_REGION_OUTPUT="$PWD/.cache/region-cold" \
  cargo test --locked --offline --release --features editor --lib \
  profile_complete_roll_area_preparation -- --ignored --nocapture --test-threads=1
ANTITONE_REGION_PERF_ARM=warm ANTITONE_REGION_OUTPUT="$PWD/.cache/region-warm" \
ANTITONE_REGION_PERF_COMPARE="$PWD/.cache/region-cold/report.json" \
  cargo test --locked --offline --release --features editor --lib \
  profile_complete_roll_area_preparation -- --ignored --nocapture --test-threads=1
```

Capture stdout under `.cache/` to retain sampled process-memory summaries as well as each private JSON report. Reports contain individual stage samples, aggregate nearest-rank p50/p95/max, actual per-source native development counts and nested open/unpack/develop timings. These distributions are across sources/stage invocations in one run, not repeated whole-roll latency percentiles. Each acquisition carries its source identity, so expected native development counts can be checked exactly per source even when some measurements need reacquisition. `measurement_write`/`measurement_read` cover optional job-local inference scratch; full or unavailable scratch falls back to RAW acquisition. Native times are included in acquisition; nested material/holder timings are included in their parents. Sum **exclusive** stage times and inspect residual wall time, not the overlapping inclusive totals. Normal builds compile markers to their original expressions with no clocks/storage; tests take clocks only during an explicit recording.

A successful run preserves source file revisions and accepted document state. It measures preparation, not import, the seed development, numerical Analyze, physical UI latency or photographic border accuracy. Do not infer a cache policy or broad hardware/pressure guarantee from one roll.

A bounded baseline on one complete twelve-source Sony roll at `42f8aa4` measured 18.05 s with an empty source store: 24 native developments consumed 17.16 s of acquisition (95% of preparation), including 11.22 s unpacking and 5.93 s development/validation. Native line observation took 0.49 s total; fine preview extraction 0.16 s; fine area inference 0.15 s. With one 511 MiB source retained, preparation measured 16.57 s and 22 developments, with identical ordered areas and frame evidence. Sampled peak process footprint was approximately 666 MiB cold and 1179 MiB partially warm; pressure remained Normal and reported swap counters did not change. These are single-run observations, not latency gates or a fully warm-roll comparison. They prioritize avoiding repeat development over line-search micro-optimization for this workload.

The bounded job-local inference-scratch candidate measured 9.49 s on the same cold twelve-source roll, with 12 developments and exact ordered area/evidence and file-revision agreement against that baseline. Scratch writes/reads consumed 0.087 s of wall time and sampled peak process footprint was approximately 667 MiB, versus 666 MiB before; pressure and reported swap counters stayed stable. This first candidate observation is 47% shorter, not a repeated or isolated-device speed guarantee; filesystem caching is uncontrolled. A later partially-warm candidate run measured 8.40 s versus 16.57 s baseline, with 11 versus 22 developments, exact area/evidence agreement and essentially unchanged sampled peak footprint (1178 MiB). A complete 36-source cold roll exercised the capacity fallback: preparation measured 46.60 s versus 56.84 s baseline (18% shorter), with 58 versus 72 developments and exact ordered area/evidence and source-revision agreement. Fourteen fine rasters fit in scratch and the remaining 22 followed the unchanged reacquisition path; sampled peak footprint was approximately 746 MiB versus 745 MiB baseline. Pressure and reported swap counters stayed stable. These runs establish bounded reuse and fallback on the measured cases, not universal latency, pressure safety or photographic accuracy. Re-measure after relevant changes.

### Marked crop quality and original-pixel inspection

Crop quality is a consumer question: report wanted content lost, unwanted border admitted and abstention separately. The read-only [crop diagnostic](../native/crop_quality/mod.rs) uses normal `.antitone` projects and the production proposal/margin functions, without running Analyze, requiring calibration or accepting edits. It does not verify RAW freshness or re-detect boundaries. Removed/reference-only sources are unavailable for ordinary photo proposals. A complete proposal on a source labelled censored/ambiguous is flagged for review, not hidden by that label.

Supply a private manifest, with project paths relative to the manifest and marks/windows in **original developed-source coordinates**, including any active-area origin—not oriented, cropped or preview coordinates. Example coordinates must be adapted to the actual source:

```json
{"margins_percent":[0,0.5],"cases":[{
  "project":"roll.antitone","source":1,"observation":"visible_frame",
  "marks":{"wanted":[[[100,100],[120,100],[120,120],[100,120]]],
           "unwanted":[[[80,100],[90,100],[90,120],[80,120]]]},
  "windows":[[80,80,128,128]]
}]}
```

`observation` is `visible_frame`, `censored` or `ambiguous`; optional `dense_wanted` marks have their own loss score. Marks are convex quadrilaterals wholly inside the active source. Scores use exact polygon/crop intersection, not bounding boxes. Error areas sum supplied regions, so overlapping marks count repeatedly. Empty categories have a null error, not a successful zero; abstention has no crop-error score. Unmarked/uncertain bands are not assessed, and sparse marked areas must not be presented as whole-picture error percentages. Freeze independent marks before inspecting predictions; record annotation uncertainty and reuse of historical holdouts honestly.

```sh
mkdir -p .cache
ANTITONE_CROP_MANIFEST=/absolute/private/marks.json \
ANTITONE_CROP_OUTPUT="$PWD/.cache/crop-quality-FRESH" \
  cargo test --locked --offline --release --features project --lib \
  report_marked_crop_quality -- --ignored --nocapture
```

For prediction-free native windows, use the same manifest and fresh output, add `ANTITONE_RAW_ROOT=/absolute/read-only-samples`, select `--features editor` and replace the test name with `capture_original_crop_windows`. Each case needs independently chosen windows. Capture is bounded to three explicit sources and twelve windows per source, each at most 256×256 pixels. It uses the existing Normal-pressure/2 GiB free-memory preflight, native geometry bound and swap/footprint/stale-telemetry stops; a cooperative deadline cannot interrupt a native call. Project runtime/development identity, input size/mtime snapshots and original RAW revisions are checked. Original manifest/project/source locators are rechecked for symlink retargeting; relative RAW paths use the normal project locator, not a relocated canonical base. Sources are developed sequentially without native-cache admission or whole-original Rust copies. Partial files without `capture.json` are not a completed observation.

The lossless `.rgb16` files contain little-endian original camera RGB16 in source orientation. This integer-only capture path rejects continuous lens-corrected sources rather than quantizing them. PNGs enlarge those pixels 4× with nearest-neighbour sampling and apply only the sRGB transfer to camera channels, at unity exposure—no colour matrix, inversion, correction, repairs, grade or detector overlays. They are viewing aids, not rendered photographs. Capture writes private artifacts only; it neither derives windows from predictions nor generates marks automatically. Ordinary geometry/project tests need no private inputs or native setup.

A frozen exploratory six-case comparison motivates retaining validation rather than its candidate algorithm: the normal detector proposed all four pictured frames, while a strict all-four-side local refiner proposed none; both abstained on two censored controls. Required local support/quorum or straight-line residual gates failed before complete transition envelopes could be assessed. This is no evidence for admitting that refiner or for envelope quality, and no reason to loosen its experimental gates on those cases. The normal 0.5% margin excluded marked unwanted strips but removed some near-edge wanted strips; gross marked interiors remained included. That illustrates an explicit policy tradeoff, not a mandate to change the default. The assistant-reviewed original-pixel transition bands were not expert-certified boundaries, and the cases were untouched only by local-refinement tuning, not pristine project-wide holdouts. [Remaining physical acceptance](plans/region-detection.md#detector-accuracy-and-latency).

### Private all-roll contacts

The native helper creates ImageIO source previews and contact sheets:

```sh
mkdir -p .cache/roll-check
/usr/bin/clang -fobjc-arc -Wall -Wextra -Werror native/image_contacts.m \
  -framework Foundation -framework ImageIO -framework CoreGraphics -framework CoreText \
  -o .cache/roll-check/image-contacts
.cache/roll-check/image-contacts previews .cache/roll-check/source /absolute/samples/*.ARW
.cache/roll-check/image-contacts sheet .cache/roll-check/source-contact.png .cache/roll-check/source/*.png
```

Source previews are not LibRaw or processing references. Group scans deliberately; frame counts do not establish physical roll identity. Supply a private JSON array:

```json
[{"name":"roll-name","sources":["/absolute/photo.ARW"],"reference":null,"area":null}]
```

`sources` are ordered photos; optional `reference` supplies correction and `area` is `[x,y,width,height]` statistics geometry.

```sh
ANTITONE_ROLL_CASES=/absolute/cases.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_OUTPUT" \
cargo test --locked --offline --release --features editor --lib diagnose_sample_rolls \
  -- --ignored --nocapture --test-threads=1
.cache/roll-check/image-contacts sheet .cache/roll-check/positive-contact.png \
  .cache/roll-check/FRESH_OUTPUT/roll-name/*.tif
```

This diagnostic drives real import/correction with a shared sampling area, bypassing per-photo review preparation. Its approximate contacts do not establish acceptance of the interactive review. Inspect failures and every image. [Implementation](../src/editor_analysis_diagnostics.rs) owns report fields.

## Responsive image scopes

```sh
cargo test --locked --offline --features ui-tests --lib \
  scope_plot_scale_and_target_widgets_produce_valid_completed_reductions \
  -- --test-threads=1
cargo test --locked --offline --features ui-tests --lib \
  compact_scope_header_fits_and_disclosure_preserves_state_at_both_densities \
  -- --test-threads=1
```

The compact-header check covers normal/narrow 1×/2× layouts, disclosure and menu keyboard access, collapsed scheduling, waveform label bounds and unchanged project state. With `ANTITONE_UI_CAPTURE` it writes `scopes-` inspection captures for the plots, menu and collapsed header. These use authored imagery, not photographic acceptance samples.

The authored test exercises [scope modes/amplification](scopes.md), real widgets and completed GPU reductions in both targets. Viewer and scopes must consume the same accepted working result. Source inspection intentionally suspends photographic scopes. Physical interpretation, responsiveness and native appearance remain separate.

## Investigating responsiveness

Use import/development, selected-preview/switch, thumbnail, acquisition and original-view logs for a reported workload. Match phases and inputs; filesystem caching is uncontrolled. Backend Ready or paint-command recording is not GPU completion or physical presentation. No minimum hardware target or general latency threshold is established; [resource acceptance](plans/visual-feedback.md) remains separate.

The `authored_` [worker checks](development.md#feature-specific-checks) use repository fixtures and isolated temporary storage. Their deadlines detect stalled tests, not unacceptable photographic latency.

### Separating input, queue work and presentation

Enable hosted [latency tracing](../src/platform/latency.rs) when kernel timings cannot explain an interaction:

```sh
mkdir -p .cache/probes
ANTITONE_LATENCY_TRACE=1 cargo app > .cache/probes/latency.log 2>&1
# Requires an explicitly authorized, unoccluded desktop window:
ANTITONE_LATENCY_TRACE=1 cargo test --locked --offline --features editor \
  --test native_host -- --run editor
```

Match frame/completion rows by ID. Timestamps separate drawable acquisition, UI handling, encoding, submission and the present call. Input starts at delivery to winit, not the physical gesture. Requested revisions identify demand, not acceptance; no submission means no drawable was rendered.

Completion callbacks attach under the shared submission gate. Tracing polls outstanding work without generating redraws and drains records at exit. Callback observation includes polling/thread scheduling; it is not a hardware GPU timestamp or scanout. Tracing adds overhead and is opt-in.

Metal allocation includes resources retained by native commands, but not all driver/process memory. Sampled peaks are not exhaustive. Measure same-PID RSS separately; on Apple Silicon these views overlap and must not be added. Arc counts and declared bytes do not establish aggregate pressure safety. Missing counters do not support compiler spill/register/occupancy claims.

For native thread policy or immutable-sensor investigation, use [native development](native-development.md), including [shared native builds](native-development.md#reusing-native-builds).

## Local processing and reuse

The [local feature](plans/local-adjustments.md) uses normal device-aware editor admission. These diagnostics isolate original-sample processing and immutable snapshots; the [composed workflow checks](development.md#local-grading-and-shared-consumers) separately exercise controls and normal workers:

```sh
ANTITONE_LOCAL_PERF_QUICK=1 cargo test --locked --offline --release \
  --features editor --test local_adjustment_performance -- --ignored --nocapture --test-threads=1
cargo test --locked --offline --release --features gpu --lib \
  paired_kernel_variants -- --ignored --nocapture --test-threads=1
cargo test --locked --offline --release --features gpu --lib \
  paired_production_local_kernel -- --ignored --nocapture --test-threads=1
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib real_raw_local_preview_tiles_and_refinement \
  -- --ignored --nocapture --test-threads=1
```

For a bounded local-stack ownership observation on the production working view:

```sh
cargo test --locked --offline --release --features editor --lib \
  physical_view_local_stack_resources -- --ignored --nocapture --test-threads=1
```

The [diagnostic](../src/gpu_working_view_local_resource_tests.rs) uses an authored RAW, a 1080p detail grid and the production whole-composition underlay, correction off/on, numeric/geometry edits and both targets. It samples same-process footprint/resident/Metal allocation and checkpoints current/building, deferred encoded consumers and retirement. The deferred texture copy is an encoded-lifetime stand-in, not a native presentation frame. Scopes and native host resources are excluded. Set `ANTITONE_SCREEN_RAW=1 ANTITONE_RAW_SAMPLES=/absolute/samples` for the supplied native-size source instead of the small authored fixture; headers must pass the existing 600-MiB diagnostic source bound. Both variants use the existing Normal-pressure, free-memory, swap, growth and sampler-freshness stops; a preflight stop establishes no workload/resource acceptance. Timings without a coordinated window are indicative, not isolated interaction latency.

Original-sample processing uses direct compensated inversion for integer and continuous inputs. [Continuous-source checks](../tests/gpu_continuous.rs), [film-response references](../tests/gpu_film_response.rs) and [original export references](../src/gpu_export_tests.rs) retain independent numerical and floor/coordinate coverage. Counterfactual exact-code/prefix probes in `gpu_reuse_tests.rs` remain test-only experiments, not production caches or scheduling policies. Existing numerical gates must not be relaxed for a faster candidate.

## Equivalent ACES execution

The [execution comparison](../src/gpu_aces_execution_tests.rs) uses an independent fragment shell with the original constant array as the oracle for production's read-only-buffer layout. It keeps pinned ACES functions, decimal constants, tables and numerical gates unchanged; the reference does not use the [production adapter](../src/renderer_aces.rs).

```sh
cargo test --locked --offline --features editor --lib \
  equivalent_aces_execution -- --nocapture --test-threads=1
ANTITONE_SCREEN_PERF_SIZE=4k cargo test --locked --offline --release \
  --features editor --lib aces_execution_cost -- --ignored --nocapture --test-threads=1
```

Require complete-raster bit agreement with the original shader, plus the existing CPU gate for dense signed/HDR hue/exposure cases in both targets. Compute shells remain independent execution references, not a second production renderer. Timings exclude compilation, upload, readback, reconstruction, scopes and presentation and do not establish a compiler mechanism or broader-device acceptance.

## Physical-view editor prototype

[Editor ownership](editor.md#a-stable-working-view-while-grading) explains the normal view and purpose-bound original comparison/tools. Composed tests exercise controls, source/positive switching, navigation with acquisition blocked, idle stability, shared viewer/scopes and zero optional retention:

```sh
cargo test --locked --offline --features ui-tests --lib physical_view_ \
  -- --nocapture --test-threads=1
ANTITONE_SCREEN_RAW=1 ANTITONE_RAW_SAMPLES=/absolute/samples \
  ANTITONE_SCREEN_PERF_SIZE=4k ANTITONE_SCREEN_CORRECTED=1 \
  cargo test --locked --offline --release --features editor --lib \
  physical_view_underlay_gpu_cost_and_retirement -- --ignored --nocapture --test-threads=1
```

The production diagnostic covers initial detail/whole-composition acquisition, immutable edits/target changes, navigation composition and retirement. It is not input-to-display timing: compilation, scopes, host/presentation, concurrent UI and readback are excluded. `ANTITONE_SCREEN_PERF_SIZE` selects a workload, not a resolution cap. RAW correction is a synthetic 2×2 field, not measured photographic correction.

For scale, the tested Sony/M5 Pro native-width workload traverses 8640×4860 detail originals plus whole 8640×5760 coarse coverage. Acquisition is approximately 0.65–0.67 s uncorrected and 3.8–4.0 s corrected. Selected fixed-view Global medians are approximately 2.1–2.3 ms at 1080p and 10.7 ms at 4K; navigation composition approximately 1.1 ms and 4.5–5.5 ms. Immutable replacement/composition checkpoints reach approximately 280–282 MiB and 950–952 MiB native resources, retiring to approximately 2 MiB after completion/owner release. These describe the positive workload, not source-mode timings or aggregate admission. Re-measure after relevant changes.

[Coverage tests](../src/gpu_working_view_underlay_tests.rs) compare signed/HDR ramps, textures, valid black, missing support and transformed frames against an independent target-linear bilinear oracle. Uniform charts check tone/colour seams. They exercise coalescing, scopes, capacity validation and owner release without introducing a photographic gallery.

[Inspection tests](../src/gpu_working_view_inspection_tests.rs) cover camera-to-AP1/correction, saved-WB normalization, inspection EV including non-power-of-two gain, both targets, reuse and original inspection correspondence. Ordinary inspection must not schedule idle original refinement or acquire a photographic grade. Source scopes remain suspended.

Broader devices, aggregate mandatory/building/encoded/native-retired resources, integrated latency and physical presentation remain [unfinished acceptance](plans/view-working-set.md). Export continues to reconstruct originals.

## Stable float32 view grading

The [production colour tests](../src/gpu_working_view_colour_tests.rs) use the actual `RenderedView` lifecycle and [shared viewer/export colour operators](../src/shaders/photographic_colour.wgsl). Their independent precise oracle applies original photographic arithmetic and mapped source-coordinate masks to identical acquired AP1; it is not another cache, renderer or admission policy.

```sh
cargo test --locked --offline --features editor --lib gpu::view::tests::colour \
  -- --nocapture --test-threads=1
cargo test --locked --offline --release --features editor --lib \
  physical_view_colour_domain -- --ignored --nocapture --test-threads=1
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib physical_view_colour_raw_correspondence \
  -- --ignored --nocapture --test-threads=1
```

The 95 authored configurations retain 117 identical signed/HDR inputs, tiny/near-cancelled luminance, toe crossings, near-identity controls, WB, weighted locals and ordered attenuation/recovery. All authored cases must remain admitted. Complete-raster comparison retains AP1 `2e-5 + 2e-4×abs(reference)`, exact floor/coverage flags and rendered-linear absolute `2e-5`. Final-cell corruption and nonfinite checks verify the reduction itself, including padded workgroups. Identity preserves words and acquired inputs remain unchanged.

The [production scalar/sampling/lifetime regressions](../src/gpu_working_view_oracle_tests.rs) check continuous original-coordinate masks, corrected original-cell footprints, signed/HDR values, floor provenance, abandoned initialization and deferred encoded consumers after owner release. The optional RAW check spans three locations, correction on/off, native/minified/rotated views, Global/soft/hard locals and both targets: 180 comparisons. It creates no gallery.

[Composed shared-colour checks](../src/gpu_shared_colour_tests.rs) additionally compare actual viewer grading and export repair application with independent scalar processing on identical canonical signed AP1. They cover local/global WB and luminance cancellation, later +12 EV recovery, fractional global/local exposure, varying source-coordinate masks and preserved floor flags. Repair values pass the ordinary repaired-domain preflight; fixture/input checks distinguish lost signals from vacuous zero output.

These isolate arithmetic agreement, not filtering-before-grading or footprint-centre-mask approximation error. Stable colour uses black-relative/linear-toe formulas, analytic divided differences, exponent scaling, compensated luminance and guarded WB/post residuals. Source geometry/acquisition and compensated original-reference controls remain separate. Later attenuation can expose an error hidden by clipping; do not substitute a clamp or weaken gates. Broader photographic/domain/device acceptance remains separate.

## View sampling quality oracles

The [scalar sampling oracles](../src/view_sampling_tests.rs) compare representations with exact original-cell correction/inversion/grades, pinned ACES and linear-target averaging. They are offline numerical references, not an interactive CPU path.

```sh
cargo test --locked --offline --features ocio-reference --lib \
  view_sampling_tests -- --nocapture --test-threads=1
cargo test --locked --offline --release --features ocio-reference --lib \
  synthetic_view_sampling_quality -- --ignored --nocapture --test-threads=1
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features raw,gpu,ocio-reference --lib raw_view_sampling_quality \
  -- --ignored --nocapture --test-threads=1
```

Reported RMS/max errors are approximation diagnostics, not implementation tolerances or photographic thresholds. Exact native permutations and complete original support preserve coverage; averaging before nonlinear grades/ACES does not generally commute. Filtered precreative input and footprint-centre masks are explicit preview approximations. Finite sampling candidates deliberately demonstrate aliasing/non-equivalence; their sample counts are not production caps. Small supplied patches do not establish photographic rejection or acceptance.

## RAW and complete GPU chain

For supplied-source metadata/development:

```sh
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- --metadata /absolute/photo.ARW
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- /absolute/photo.ARW
```

Metadata mode reports Sony embedded shading/distortion/CA availability, known camera settings, validated radial scales, shading amplification and automatic output dimensions without unpacking or correcting pixels. It can inspect malformed data even though normal development rejects it. Off is distinct from unavailable coefficients. Discovery reads the selected RAW IFD, not rendered previews, and ignores inherited Sony coefficients in DNG.

Full mode develops the automatically corrected camera raster, checks repeated development and reports reconstruction time/storage separately from LibRaw time. The LibRaw sRGB matrix comparison deliberately uses a separate native raster: channel-dependent CA cannot be applied to already mixed/clipped sRGB. Optional PPMs include corrected/native pairs when applicable. `ANTITONE_RAW_PPM_EV` selects their shared diagnostic exposure (default +3 EV; 0 EV is useful for ordinary photographs). They are clipped unmanaged diagnostics, not colour or photographic acceptance. [Sony processing](processing.md#embedded-sony-lens-correction) owns coordinates, reconstruction and capture authority; [remaining acceptance](plans/raw-lens-correction.md) includes vendor/reference agreement and broader photographs/hardware.

Per-component distortion/CA maps and independent shading checks run offline in the pure model tests; authored native cases separately exercise automatic development and its diagnostic bypass:

```sh
cargo test --locked --offline --no-default-features \
  --test sony_lens --test sony_geometry --test sony_vignetting
cargo test --locked --offline --features raw,project --test raw_sony_lens
cargo test --locked --offline --features gpu --test gpu_continuous
# Add the normal OCIO reference setup for reopened Sony TIFF/AP0 EXR correspondence.
cargo test --locked --offline --features raw,gpu,export,project,ocio-reference --test raw_sony_lens
```

Bounded release-build checks on an Apple M5 Pro / 48 GiB cover ILCE-1M2 / 24–70mm GM II photographs at 24/70 mm and f/2.8/f/8. A distortion/CA-only two-source 24/70-mm timing run took 0.22–0.27 s per roughly 47–50-MP source for reconstruction plus eligibility-mask construction. Corrected RGB storage was 567–597 MB, capture masks 11.8–12.4 MB, and redundant native RGB was released. The serial CLI run, including repeat development and native sRGB references, peaked near 1.23 GB physical footprint. Sampled repeatability was exact, matrix gates passed and corrected/native previews were inspected for gross discontinuities. An adapted-lens scan kept the native path. These are spot-checks, not vendor calibration, fine-detail/CA approval, editor-switching latency or aggregate memory-pressure acceptance.

With embedded shading enabled, a two-source 24/70-mm release probe on the same M5 Pro took 0.42–0.49 s per reconstruction, including eligibility construction, with about 1.23 GB peak footprint for the serial probe and unchanged retained-raster sizes. All 112 supplied profiles validated, with maximum shading gains from 1.108 to 1.907. The 24-mm example retained 3001 red samples above native white; sampled repeats were exact and native matrix comparisons passed. These remain bounded CLI observations, not editor or vendor-reference acceptance.

Full mode checks repeatability, flooring and eligible in-sRGB LibRaw matrix comparisons. Inspect `compared_grid`: zero eligible comparisons cannot establish agreement. Optional PPMs are clipped source diagnostics, not photographic calibration or ACES/ICC output.

[Authored RAW checks](development.md#feature-specific-checks) preserve Scan immutability, measured anchors (`2e-12`), flooring (at most 1%) and sampled LibRaw matrix agreement (`2/65535`). The authored matrix case requires a nonzero eligible grid.

For independent upstream OCIO → Metal:

```sh
./scripts/probe-ocio.sh
./scripts/probe-ocio.sh --camera
cargo test --locked --offline --release --features raw,gpu \
  --test camera_raw -- --ignored --nocapture
```

The camera extension uses the committed patch DNG and explicit code-floor cases. Upstream OCIO supplies the rendering reference; missing/stale requested camera inputs fail. Existing finite/neutral/basis and numerical gates remain unchanged. This is not full-image photographic acceptance.

For corrected affine reconstruction against scalar coverage:

```sh
cargo test --locked --offline --features editor,ocio-reference --lib \
  --test scopes --test viewer --test affine_resample --test geometry_export
cargo test --locked --offline --release --features editor,ocio-reference --lib \
  corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

### Sony source storage precision

The opt-in [storage study](../tests/lens_precision.rs) holds development and capture eligibility fixed while comparing retained f32 camera codes with whole-code rounding and an eight-fractional-bit diagnostic alternative. Within the native range these correspond to RGB16 and Q16.8. For above-white shaded datasets, reports label the rounding models `whole-code`/`code-q8` and omit packed-u16 GPU timing: those values are not representable by plain RGB16. It does not change production storage.

```sh
ANTITONE_RAW_SAMPLES=/absolute/path/to/samples \
  cargo test --locked --offline --release \
  --features raw,gpu,export,project,ocio-reference --test lens_precision \
  -- --ignored --nocapture --test-threads=1
```

Use the normal OCIO/native setup. Outputs stay under `.cache/lens-precision/`: CSV metrics, paired PPMs and explicitly amplified differences. Synthetic cases blend declared integer neighbours; supplied negatives use diagnostic calibration and borrowed Sony warps because their adapted lens has no usable profile. Those warps test resampling/rounding, not physical lens calibration. Real profile-bearing photographs use actual production correction and source inspection. Global samples and 384-square native-detail patches cover default adjustable response with exposure 0/+2/+4 EV and +8 EV stress. Constant-gain/anchor scaling is a scale-invariance and timing control; separate spatial ±20% illumination gains use fixed centre-unity anchors and independent CPU gain evaluation.

Storage errors use the f64 scalar pipeline followed by independent pinned OCIO rendering to sRGB. The GPU timing arms are separately checked against their corresponding scalar inputs under the existing numerical gate. On one M5 Pro / 48-GiB distortion/CA-only run, before shading was enabled, maximum differences expressed in encoded 8-bit-level equivalents were:

| Case | Largest RMS across tested settings | Largest channel difference |
| --- | ---: | ---: |
| Ordinary negative, global/detail, including spatial gains | 0.014 | 0.108 |
| Dense reference negative, including spatial gains | 0.167 | 1.818 |
| Sony photographs, source inspection | 0.030 | 0.272 |

Within its representable range, RGB16 adds at most half a native code. Post-shading headroom loss is a different error and is not included in those historical rounding numbers. The dense reference's rare roughly two-level output changes show that this is not always negligible after inversion; they do not establish subjective visibility or recovered sensor detail. Q16.8 reduced those differences substantially (dense-case maximum below 0.008 encoded 8-bit levels), but needs custom packing and still requires continuous inversion.

With the then-retained exact-code response cache, one historical 147,456-pixel negative-detail buffer had warmed encode/submit/completion medians of about 6.65 ms for fractional f32 versus 0.62 ms for packed RGB16. Rounded integer values in a float upload also took about 0.56 ms, isolating exact-code lookup reuse from transport width. With spatial flat-field gains, all three took roughly 7.4–7.8 ms because they all used continuous inversion. These small-buffer timings exclude upload, readback and rendering; they are not full-resolution export or interaction timings, and do not describe current direct processing after removal of the integer-only cache.

Within-range packed corrected RGB16 could halve retained RGB storage (6 rather than 12 bytes/pixel), but preserving shading headroom would also require a range/scale design; native uncorrected LibRaw storage remains four u16 slots. Current f32 preserves fractions using a standard representation, at real memory/processing cost. The study supports a tradeoff, not a universal source-format requirement. It does not cover all grades, anchor remeasurement, devices or actual profile-bearing film scans; no production format change follows automatically from the experiment.

## Rendering, gamut and ICC

CPU rendering/ICC regenerates its OCIO references without preceding Metal checks or cached input:

```sh
./scripts/probe-rendering.sh
./scripts/probe-rendering.sh --camera
cargo test --locked --offline --features ocio-reference --test rendering
./scripts/probe-gamut.sh
cargo run --locked --offline --features ocio-reference --bin antitone-rendering-probe -- --gamut-check
```

Rendering scripts share `scripts/generate-ocio-reference.sh`, which regenerates upstream CPU/GLSL/LUT artifacts. Existing camera artifacts are ignored unless requested; failures do not fall back to cached files. CPU/GPU agreement, transfer/profile checks and independent ColorSync/Core Image checks address different boundaries. [OCIO artifacts](../assets/ocio/README.md) and [profiles](../assets/profiles/README.md) retain provenance.

## Export

[Export checks](development.md#export-checks) distinguish production GPU/worker workflows, exact encoder serialization and independent scalar/OCIO photographic references. [GPU exports](../tests/gpu_export.rs) reopen original-sample TIFF/AP0 EXR with ordered locals, correction, orientation and straightening; [reference export tests](../tests/export.rs) retain scalar file-code/profile/compression and failure checks behind `ocio-reference`. No reference is a production fallback.

During an agreed GPU window, the opt-in full-size resource exercise retains production physical detail/underlay while exporting and then regrading that retained source:

```sh
ANTITONE_RAW_SAMPLES=/absolute/samples \
  cargo test --locked --offline --release --features editor --lib \
  full_size_export_resources_with_retained_working_view \
  -- --ignored --nocapture --test-threads=1
```

It uses the existing three train/calibration RAW inputs read-only, checks native geometry before development, and keeps the existing monitor's pressure, swap-counter, footprint-growth and unavailable-telemetry stops. Exports stop at bounded-unit checkpoints if those guards trip or the diagnostic duration limit expires. The default cases are uncorrected native composition and synthetic 2×2 correction with a straightened, quarter-turned crop, each as sRGB TIFF and full AP0 EXR. `ANTITONE_EXPORT_RESOURCE_CASE` selects `native`, `rotated`, `corrected-native` or `corrected-rotated`; `ANTITONE_EXPORT_RESOURCE_FORMAT=tiff` narrows a run to TIFF for cost isolation. The default backend is production GPU export. `ANTITONE_EXPORT_RESOURCE_LOCALS=2` adds ordered linear exposure/WB and inverted radial exposure/contrast/saturation stages; the post-export regrade preserves these locals.

There is one production GPU engine for both formats, including exact crops and repaired edges. `ANTITONE_EXPORT_RESOURCE_BACKEND=gpu` selects it. For matched independent scalar/OCIO-reference measurements, add `ocio-reference` to the Cargo features and select `reference` or `compare` (reference followed by production GPU). Source, grade, composition and shared encoders stay identical. The reference is a diagnostic oracle, not a production fallback; its initialization is reported separately, with bounded stop checks during source-sample evaluation. Cache/pair/forecast backend selectors are not available.

Record power source and configured mode with timing observations (`pmset -g batt` and `pmset -g custom` on macOS). Battery operation or a different power mode can alter CPU/GPU clock budgets and the critical-path balance. Comparisons that straddle a power transition are not matched latency evidence; numerical/file equality remains valid. Do not change system power settings, purge caches or interrupt other applications to improve a result without agreement.

### Shared export precision and pixel-endpoint checks

The [shared engine](../src/gpu_export_area.rs) processes conservative real-source bounds plus a real-cell halo per destination block and reads back destination pixels only. Padding and repeated originals count as evaluated work. Exact crops, rotated crops and repaired edges all use that engine. The [worker entry](../src/gpu_export_job.rs) validates the captured source basis and resolves cold repairs independently of viewer warmth. Exact whole-frame source-cell masks keep that repair relevance valid under native mapping roundoff; no edge/EXR fallback engine is retained.

[Independent stage checks](../src/gpu_export_stage_oracle_tests.rs) combine canonical repairs, source-point grades, a scene look, scalar f64 footprint weights and CPU/OCIO target rendering. [EXR checks](../src/gpu_export_common_exr_tests.rs) exercise the public engine in both storage precisions. The current common working allowance is `2e-5 + 2e-4 × |reference|`, with storage quantization separate. Exact signed-HDR cancellation, constant/range conservation, invalid-input masks and flags supplement this comparison; it is not a visibility threshold or a universal-domain proof. Selected TIFF cases retain tighter one-code comparisons, and serialization of supplied samples is tested independently.

[Continuous-source worker checks](../src/gpu_export_corrected_source_tests.rs) exercise authored Sony fractions/headroom plus residual correction against independent scalar/OCIO TIFF, with a clipped-source control. [Lifetime checks](../src/gpu_export_look_lifetime_tests.rs) count cold/warm LUT allocations through changing scratch dimensions and owner release. The guarded response is common to TIFF and EXR: it preserves near-base subtraction and retains compensated inversion under sensitive downstream conditions or large log anchors. [Margin](../src/gpu_export_precision_margin_tests.rs) and [low-gamma](../src/gpu_export_density_condition_tests.rs) tests retain independent working-space and exact-flag checks.

For TIFF pixel-endpoint measurements, the test-only [ready-to-encode sink](../src/export_pixel_receipt.rs) receives complete RGB16 frame codes through production block callbacks without opening a destination or invoking a codec. File-writing/reopening comparisons must use the identical captured recipe and match those codes. Include required cold repair preparation; report source acquisition, renderer/kernel preparation and codec/I/O separately. Prewarm is startup work, not free computation. Historical component/prototype controls are not alternate production backends.

Unit-payload reports include f32 camera input/upload, working/rendered support, destination/readback/encoding temporaries, exact-frame masks and host/GPU sparse repair uploads. Retained logical repairs and job LUT are separate; small constants, allocator/driver overhead and full-frame ownership remain outside that unit bound. The sparse upload reserves its selected count before materialization. These payloads are not aggregate process-memory admission.

Normal-pressure/2-GiB non-speculative-free admission, swap, 2-GiB growth, telemetry and three-minute export stops are unchanged. Preserve refused, noisy and adverse observations. When GPU timestamp features are explicitly requested, deferred processing queries use native clock calibration and owning-submission brackets; final collection is separate from the pixel endpoint. Pass elapsed time includes scheduling/stalls, not exclusive ALU or inversion cost. Unavailable/invalid rendering counters do not become zero-cost measurements.

### TIFF encoding cost

TIFF encoding is separate from producing ready RGB16 pixels. The current encoder uses lossless Deflate at level 1, with horizontal prediction and eight-row strips. It retains the same full-frame assembly, metadata/ICC, flush/sync and checked atomic publication. The compression tag is 8 rather than LZW's 5; this changes file representation, not the photograph or its samples. The production path has no experimental codec selector.

A bounded frozen-frame investigation attributed about 2.2 s to the preceding LZW codec, versus about 0.005 s prediction, 0.12–0.14 s observed file-write calls and 0.006 s sync. The write timings are below buffering but still include OS/cache behavior, not isolated uncached physical-disk bandwidth. Codec-to-counting-sink replay is a separate observation, not an interval to add to the normal encoding profile. On the two grainy RGB16 frames, LZW slightly expanded 256 MB of samples into 260–261 MB; fast Deflate produced approximately 205 MB and reduced frozen-frame encode-and-save time from roughly 2.4 s to 0.84 s.

One balanced normal-worker window measured total Save means of 6.870→5.153 s sRGB and 6.877→5.406 s P3/look/cold repair, approximately 21–25% shorter with fast Deflate. There were two unprofiled samples per codec/recipe, opposite codec orders and separate profiles. Source acquisition and renderer/look infrastructure were separately charged; the endpoint includes encoding, flush/sync, final stop check and publication. Complete reopened samples were exact. The slower 7.331 s LZW sample remains in the sRGB mean; these are one-source/device observations, not universal percentages, batch throughput or combined gains from earlier optimizations.

Uncompressed TIFF was faster still but forfeits substantial size savings on smooth content. Authored flat/ramp/noise checks and native ImageIO RGB16/dimension/ICC reopening support the lossless Deflate choice. ImageIO storage/profile acceptance is not independent pixel-rendering truth or broad compatibility with Lightroom/Resolve, older readers or other platforms. Whole-file bytes legitimately differ from LZW; supplied-sample serialization, metadata and safe publication retain their separate checks. Existing native admission/pressure/swap/growth/telemetry/duration stops and AC/mode 0 were unchanged, with no stops or application/cache/power manipulation. These observations do not establish aggregate/in-flight resource acceptance.

### Earlier bounded-reuse observations

These measurements describe the preceding coverage/cache/pair exporter, not the shared original-sample engine or its current performance. They retain adverse evidence and explain the earlier cost investigation; the corresponding backend selectors are no longer maintained.

On one already-developed 49.77-MP Sony source, synthetic 2×2 correction and a 42.66-MP quarter-turned/straightened LZW TIFF, a balanced M5 Pro comparison measured about 5% lower full-export latency from two-original batching, with identical complete files and unchanged resource stops. Bounded authored checks measured about 13–14% lower corrected tile-path time. Two-slot GPU/batched-submit probes did not establish a useful gain; larger fixed per-plan packs offered little additional launch reduction on that demand trace. These are different mechanisms from concatenating two originals into one dispatch/render/map or pairing across plans.

At the same 4-MiB tile payload, admitted 1×5 traversal/future-use retention reduced 4,062 to 3,233 original evaluations for a 10,559-request/2,710-unique-origin case; dispatches changed from 2,652 to 2,642. A comparison with two ordered locals, both run orders, logged AC power and unchanged guards averaged 12.109 s versus 10.623 s (about 12% lower), with complete-file identity. Global-only comparisons showed about 6% benefit but did not log power state. One local comparison regressed; battery operation was reported afterward, with unknown transition timing. Preserve that unfavorable evidence rather than attributing every difference to the algorithm. These are already-developed single-image observations, not isolated hardware or batch-throughput claims.

### Reading export resource reports

Every output pixel is reopened through bounded TIFF strips or a nonparallel EXR counting/finite-value sink; decoder limits are not disabled and another full-frame decoder cache is not introduced. Temporary photographic outputs are removed automatically. GPU phase traces include source preparation, bounded area planning, GPU/readback waits and output packing; reference totals include scalar photographic processing, reconstruction and encoding. GPU reports count evaluated source pixels, dispatches, destination readback bytes, stop checks and peak declared unit payload, including repeated support. Both report actual source-sample evaluation counts, file bytes and sampled process footprint, resident memory and Metal allocation, with sample age. Those quantities overlap and must not be added. The regrade exercises the GPU service after export, not simultaneous editing or physical UI latency. If other GPU consumers are not known to be idle, timings are indicative rather than isolated cost evidence; do not interrupt other work without agreement. This is a bounded observation on the supplied camera/device, not forced pressure, an aggregate admission guarantee or broad photographic acceptance.

The GPU exercise also emits opt-in test-only `export_stage` aggregates: setup, coverage planning/grouping, tile access, sample filling, weighted reduction, target encoding, frame allocation, format packing, file encoding/flush and destination finalization. Coverage planning includes exact coordinate mapping, validation and clipping; tile access still combines CPU preparation, GPU work and readback waits. Format packing includes EXR AP0 conversion and precision checks; file encoding combines compression with writes, not isolated filesystem time. Allocation timings do not separately account for later lazy page faults. Accounted/residual/overlap fields expose the timing boundary; do not add nested or overlapping costs. Normal builds expand the markers to the original operations without recording or clock reads.

Host `export_detail` rows distinguish source preparation/upload, command encoding, command finalization/submission, map arming, completion wait, mapped-memory copying/unmapping, working-vector conversion and target-vector layout. Some explain residual work; others subdivide parent intervals: source upload is inside command encoding, map/wait/copy/unmap inside tile access, and target layout inside target encoding. For GPU RGB16 output, `WorkingPack` measures packed-status validation and unpacking; `TargetEncode`/`TargetLayout` are zero host costs, not zero GPU encoding work. RGB16 runs inside the existing submission/completion interval. `Setup` includes job-owned RGB16 kernel/boundary preparation. The optional test-only linear CPU reference uses its ordinary transfer/quantization markers. EXR's `WorkingPack` still includes validity scanning and promotion for AP0 serialization. Actual readback reports eight bytes per RGB16 pixel versus sixteen per linear scene pixel; `job_rgb16_payload_bytes` reports the separate fixed boundary buffer, and the unit ceiling includes the extra packed GPU scratch. The readback buffer's allocation capacity is not the copied byte count. Do not add all detail rows to the stage total. Queue-write and mapped-copy times measure host operations, not isolated device transfer bandwidth; completion wait includes queued GPU processing/rendering/reconstruction and host/driver scheduling. Use calibrated pass counters for narrower GPU attribution, and compare instrumented/uninstrumented endpoints before interpreting small differences.

For read-only preflight diagnosis without RAW loading or export, run `cargo test --locked --offline --release --features editor --lib resource_preflight_only -- --ignored --nocapture`. The unchanged guard requires Normal pressure and 2 GiB non-speculative free memory. Telemetry distinguishes that counter from speculative pages, which macOS already includes in its total free-page count; low non-speculative free memory is not by itself a pressure alarm. Do not purge caches or relax the guard to force a measurement.

ImageIO provides independent TIFF reopening:

```sh
cargo test --locked --offline --features raw,export,ocio-reference --test raw_dng \
  authored_dng_tiffs_reopen_with_independent_imageio_storage_and_profiles \
  -- --ignored --nocapture
```

This compiles [the native checker](../native/tiff_reference.mm) and creates all products in an isolated temporary directory. It checks dimensions, RGB16 storage and pinned ICC bytes, including malformed/wrong/multi-image/RGB8 rejection. Storage/profile agreement is not pixel-rendering or photographic acceptance; large-file resources and Lightroom/Resolve remain separate.

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --submission-stress
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --interactive
```

Submission stress brings its own window forward and checks repeated native resizing, background completion and presented frames through the shared gate. Native application automation/window exercises require explicit authorization. Zero submitted frames fails. Layer/encoding/readback agreement does not establish physical monitor matching, transitions, macOS 14 or broader devices; keep those limits distinct from the tested [output boundary](colour-and-output.md).
