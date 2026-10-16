# Specialist diagnostics

Use [development checks](development.md) for the ordinary contributor loop. These recipes investigate native, numerical, image or presentation boundaries; they are not prerequisites for every change.

Run from the repository root after [setup](../README.md#build-and-run). Examples use `--offline` with populated caches. Private RAWs are read-only local inputs, not downloadable fixtures. Keep generated products under ignored `.cache/` and use fresh export/report directories.

Choose a diagnostic for a concrete question. A command's presence here does not make it a routine gate. Numerical agreement, offscreen input, a visible native window and physical photographic acceptance are different evidence. Timings and captures are observations, not acceptance thresholds. Sample-dependent checks can be expensive; do not run this entire page reflexively.

## Editor and layout checks

Build and open a saved project for real-use inspection:

```sh
cargo build-app --offline
target/release/antitone /path/to/grade.antitone.json
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

Capture stdout under `.cache/` to retain sampled process-memory summaries as well as each private JSON report. Reports contain individual stage samples, aggregate nearest-rank p50/p95/max, actual per-source native development counts and nested open/unpack/develop timings. These distributions are across sources/stage invocations in one run, not repeated whole-roll latency percentiles. Native times are included in acquisition; nested material/holder timings are included in their parents. Sum **exclusive** stage times and inspect residual wall time, not the overlapping inclusive totals. Normal builds compile markers to their original expressions with no clocks/storage; tests take clocks only during an explicit recording.

A successful run preserves source file revisions and accepted document state. It measures preparation, not import, the seed development, numerical Analyze, physical UI latency or photographic border accuracy. Do not infer a cache policy or broad hardware/pressure guarantee from one roll.

A bounded baseline on one complete twelve-source Sony roll at `42f8aa4` measured 18.05 s with an empty source store: 24 native developments consumed 17.16 s of acquisition (95% of preparation), including 11.22 s unpacking and 5.93 s development/validation. Native line observation took 0.49 s total; fine preview extraction 0.16 s; fine area inference 0.15 s. With one 511 MiB source retained, preparation measured 16.57 s and 22 developments, with identical ordered areas and frame evidence. Sampled peak process footprint was approximately 666 MiB cold and 1179 MiB partially warm; pressure remained Normal and reported swap counters did not change. These are single-run observations, not latency gates or a fully warm-roll comparison. They prioritize avoiding repeat development over line-search micro-optimization for this workload. Re-measure after relevant changes.

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

This older diagnostic drives real import/correction and the legacy shared-area analysis control, bypassing the new per-photo review provider. It renders approximate contacts for comparison, not acceptance of the new workflow. Inspect failures and every image. [Implementation](../src/editor_analysis_diagnostics.rs) owns report fields.

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

The [local feature](plans/local-adjustments.md) remains guarded in the editor. These checks exercise original-sample processing and immutable consumer snapshots, not interactive local-tool admission:

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

[Response-table tests](../src/gpu_response_table_tests.rs) check exact RGB16-code reuse, complete keys, signed/floor behavior and initialization/publication through the hosted submission gate. Continuous corrected input and inspection bypass the table. Independent original-processing operator/stage references remain numerical diagnostics; they are not alternate editor caches or scheduling policies. Existing numerical gates must not be relaxed for a faster candidate.

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

The [production colour tests](../src/gpu_working_view_colour_tests.rs) use the actual `RenderedView` lifecycle and [view-colour shader](../src/shaders/view_colour.wgsl). Their independent precise oracle applies original photographic arithmetic and mapped source-coordinate masks to identical acquired AP1; it is not another cache, renderer or admission policy.

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

These isolate arithmetic agreement, not filtering-before-grading or footprint-centre-mask approximation error. Stable colour uses black-relative/linear-toe formulas, analytic divided differences, exponent scaling and targeted compensated luminance. Source geometry/acquisition and original-sample processing remain precise. Later attenuation can expose an error hidden by clipping; do not substitute a clamp or weaken gates. Broader photographic/domain/device acceptance remains separate.

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

It uses the existing three train/calibration RAW inputs read-only, checks native geometry before development, and keeps the existing monitor's pressure, swap-counter, footprint-growth and unavailable-telemetry stops. Exports stop at bounded-unit checkpoints if those guards trip or the diagnostic duration limit expires. The default cases are uncorrected native composition and synthetic 2×2 correction with a straightened, quarter-turned crop, each as sRGB TIFF and full AP0 EXR. `ANTITONE_EXPORT_RESOURCE_CASE` selects `native`, `rotated`, `corrected-native` or `corrected-rotated`; `ANTITONE_EXPORT_RESOURCE_FORMAT=tiff` narrows a run to TIFF for cost isolation. The default backend is production GPU export.

For batching attribution, select `ANTITONE_EXPORT_RESOURCE_BACKEND=serial` for the diagnostic single-original GPU loader with production's cache policy, or `compare-batch` / `compare-batch-reverse` for the two orders under one leased source/view setup. With `ANTITONE_EXPORT_RESOURCE_FORMAT=tiff`, these modes require complete-file hash equality after bounded reopening; the hash pass is outside export timing and inside the resource monitor. They do not change normal builds or select a CPU fallback. Test-only `export_batch` distinguishes original evaluations from dispatch/render/map count: batching reduces submissions, not the number of missed originals. Compare full wall time and geometry-consumer wait rather than extrapolating isolated tile timings. Render-pass end queries have produced zero/stale values on the measured Metal path; do not saturate negative intervals into apparent zero-cost ACES, or treat pass elapsed time as ALU utilization. ACES/copy attribution remains open.

`ANTITONE_EXPORT_RESOURCE_BACKEND=compare-cache` compares uncached GPU execution with production's cache policy under one leased source/view setup and monitor. `uncached` selects only the baseline; `cached` forces reuse even for exact compositions, where production avoids unnecessary retention. For matched independent scalar/OCIO-reference measurements, add `ocio-reference` to the Cargo features and select `reference` or `compare` (reference followed by production GPU). Source, grade, composition and shared encoders stay identical. This is a reference benchmark, not a production fallback or a historical old-binary benchmark. Its processor initialization is reported separately, with bounded stop checks during source-sample evaluation.

Every output pixel is reopened through bounded TIFF strips or a nonparallel EXR counting/finite-value sink; decoder limits are not disabled and another full-frame decoder cache is not introduced. Temporary photographic outputs are removed automatically. GPU phase traces separate tile access (miss sampling/GPU/readback or hit lookup) from other job work; reference totals include scalar photographic processing, reconstruction and encoding. Tile requests, actual GPU reads, cache hits and peak retained tile payload distinguish reuse from repeated work. Both report actual source-sample evaluation counts, file bytes and sampled process footprint, resident memory and Metal allocation, with sample age. Those quantities overlap and must not be added. The regrade exercises the GPU service after export, not simultaneous editing or physical UI latency. If other GPU consumers are not known to be idle, timings are indicative rather than isolated cost evidence; do not interrupt other work without agreement. This is a bounded observation on the supplied camera/device, not forced pressure, an aggregate admission guarantee or broad photographic acceptance.

The GPU exercise also emits opt-in test-only `export_stage` aggregates: setup, coverage planning/grouping, tile access, sample filling, weighted reduction, target encoding, frame allocation, format packing, file encoding/flush and destination finalization. Coverage planning includes exact coordinate mapping, validation and clipping; tile access still combines CPU preparation, GPU work and readback waits. Format packing includes EXR AP0 conversion and precision checks; file encoding combines compression with writes, not isolated filesystem time. Allocation timings do not separately account for later lazy page faults. Accounted/residual/overlap fields expose the timing boundary; do not add nested or overlapping costs. Normal builds expand the markers to the original operations without recording or clock reads.

TIFF planning runs on its scoped CPU producer, outside the consumer's thread-local stage aggregate. Test-only `export_geometry` reports producer planning time, consumer wait, completed-plan payload peak/bound and virtual thread stack separately. Producer and consumer times overlap and must not be added to export wall time. Completed payload excludes construction/allocator transients and is not process admission; the same sampled resource stops still apply. Normal builds take no planning clocks or diagnostic atomics.

For a low-memory CPU-only geometry A/B, run `cargo test --locked --offline --release --features raw,gpu,export --lib reconstruction_stages_without_native_source_or_gpu -- --ignored --nocapture --test-threads=1`. It compares per-pixel mapping with exact shared-edge reuse on synthetic already-processed values, alternating order across four turns. It loads no RAW, creates no GPU and retains no full output frame; it is not a photographic or end-to-end GPU benchmark.

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
