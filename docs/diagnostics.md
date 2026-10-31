# Specialist diagnostics

Use [development checks](development.md) for the ordinary contributor loop. These recipes investigate native, numerical, image or presentation boundaries; they are not prerequisites for every change.

Run from the repository root after [setup](build-and-run.md#build-and-run). Examples use `--offline` with populated caches. Private RAWs are read-only local inputs, not downloadable fixtures. Keep generated products under ignored `.cache/` and use fresh export/report directories.

Choose a diagnostic for a concrete question. A command's presence here does not make it a routine gate: numerical agreement, offscreen input, a visible native window and physical photographic acceptance are different evidence, and timings and captures are observations, not acceptance thresholds.

## Collecting a session log

The desktop app writes readable stderr and local JSON-lines session files automatically, including launches from Finder. Console lines show UTC time, level, target, thread and `key=value` fields; session files preserve typed fields. To report a problem:

1. Reproduce it, then quit Antitone to drain queued session-file events.
2. In Finder, choose **Go → Go to Folder** and enter `~/Library/Logs/Antitone`.
3. Copy the newest `session-…` folder and share its `.jsonl` files together. `current.jsonl` is newest; `1.jsonl` and `2.jsonl` hold earlier events after rotation.

Logs stay on the machine until shared. Review them first: propagated errors and panic details can contain file paths and usernames, and startup records identify the GPU and native runtime. Abrupt termination can lose queued events; copy the folder before repeated relaunches age it out.

Each session retains up to three 8 MiB files. Startup keeps five sessions where possible, protecting sessions still open in other processes. A bounded background queue keeps normal log IO out of paint callbacks; overload is reported as `records_dropped`. File failures leave console logging active; unavailable stderr does not stop file capture. Use a writable `ANTITONE_LOG_DIR` and relaunch if capture fails.

During normal capture, stderr has its own bounded queue and is best effort under backpressure: a full console queue drops console-only lines, and each console flush waits at most 250 ms. A stalled reader therefore cannot hold session-file capture or native Quit.

Both outputs use the same filter: the default app level is `info`, and dependencies are capped at `warn`. For detailed source, thumbnail, native and preview timings, launch with `debug` and optionally a separate capture directory:

```sh
ANTITONE_LOG=debug ANTITONE_LOG_DIR="$PWD/.cache/logs" cargo app
```

`ANTITONE_LOG` accepts `off`, `error`, `warn`, `info`, `debug` or `trace`. Records carry a timestamp, level, target, thread, event name and typed fields. Rust instrumentation uses `log` key/value fields, for example `log::warn!(request, error = reason.as_str(); "source_failed")`; native bridges use [the shared scope logger](../native/logging.h). Standalone probe output and test reports remain on stdout.

## Editor and layout checks

Build and open a saved project for real-use inspection:

```sh
cargo build-app --offline
target/release/antitone /path/to/grade.antitone
```

Calibration, grade/source controls, physical-pixel viewing, picker-source return and normal/narrow layouts use authored [framework UI tests](development.md#framework-ui-regressions). Native dialogs and photographic Save/Open inspection remain real-use checks.

### Viewer-tool evidence

Framework tool tests render Crop/Dmin/WB and flat-field support at normal/narrow sizes. Optional `ANTITONE_UI_CAPTURE` products are offscreen evidence, not native-window appearance approval; [native-host checks](development.md#explicit-native-host-checks) separately exercise focus, size forwarding, surface paint and completion/close failures.

## Flat-field correction

For authored reference/area transitions, corrected picks, both targets/thumbnails, Enabled/Clear and Save/Open:

```sh
cargo test --locked --offline --features ui-tests --lib \
  editor::kittest_tests::correction
cargo test --locked --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

The second exercise exports/reopens full-size images and is more expensive. These check implemented [correction](flat-field.md), not physical reference suitability or native-dialog gestures.

## Analyze roll

Use [authored analysis checks](development.md#roll-analysis) for transaction and estimator regressions; they cover review and exception edits, layouts, cancellation and failure, atomic support/grade installation, Save/Open and both targets. They are offscreen — [native-host completion checks](development.md#explicit-native-host-checks) are separate.

An optional regression checks opening recovery and holder-tail exclusion on one twelve-frame feedback scan set:

```sh
ANTITONE_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib real_ambiguous_first_frame_uses_roll_opening_instead_of_holder \
  -- --ignored --nocapture
```

Its named files and area/Dmax assertions apply only to that case; neither route certifies photographic success or a general opening detector.

A unity-WB project can compare the current Negative analysis against the former first-scan as-shot basis on one complete read-only roll:

```sh
ANTITONE_UNITY_WB_PROJECT=/absolute/roll.antitone cargo test --locked --offline --release \
  --features editor --test unity_wb_roll_compare \
  -- --ignored --nocapture
```

It uses the project's accepted analysis areas, redevelops the roll in both bases, and checks that the analyzed frame means and chroma remain within diagnostic general-look bounds. Equal eligible sample populations are required; differing populations make the comparison inconclusive rather than evidence about WB.

### Retained area evidence and complete native review

The retained-evidence comparison pins native line extraction, the frozen **local** rectangle baseline and canonical original-centre membership — the current shared-holder provider intentionally differs from that local baseline:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence ANTITONE_REGION_NATIVE=1 \
  cargo test --locked --offline --release --features raw,project --lib \
  retained_rectangle_baseline_parity -- --ignored --nocapture
```

Compare the current provider with that frozen baseline across the retained corpus into a fresh `.cache/` output directory:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence \
ANTITONE_REGION_OUTPUT="$PWD/.cache/holder-comparison" \
ANTITONE_REGION_IMAGES=DSC04331,DSC01635 \
  cargo test --locked --offline --release --features editor --lib \
  compare_shared_holder_with_frozen_local_baseline \
  -- --ignored --nocapture
```

The output directory must be new and outside the frozen evidence tree. The comparison checks expected shared-support/abstention counts and spatial population adequacy; optional PNGs (old local opening in red, conservative opening in gold, picture statistics in cyan) are evidence for software behavior, not photographic appearance.

For one explicitly named complete retained roll, the Editor worker regression performs real native preparation, browsing, Analyze, return to the working photo, Save/Open and accepted-support reuse, preserving RAW size/mtime snapshots and writing only temporary project data under `.cache/`:

```sh
ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence \
ANTITONE_RAW_ROOT=/absolute/read-only-samples ANTITONE_REGION_ROLL=roll-name \
  cargo test --locked --offline --features editor --lib \
  complete_private_roll_prepares_reviews_analyzes_and_round_trips \
  -- --ignored --nocapture
```

### Bounded area-preparation profiling

The ignored `profile_complete_roll_area_preparation` diagnostic calls the production preparation provider directly — no Editor, thumbnails, areas or window. Each invocation is bounded to one complete 2–40-source roll and 240 seconds at cooperative checkpoints; native calls cannot be interrupted mid-call, so do not truncate a larger roll or relax stops to obtain a result. The [monitor](../native/cache_observation.rs) requires Normal pressure and 2 GiB non-speculative free memory.

Run the two arms as separate processes with fresh output directories. The **cold** arm means an empty application source store, not an uncached filesystem; the **warm** arm seeds only the last ordered source under a 600 MiB optional budget, so it is a partially warm document, not a fully resident roll:

```sh
mkdir -p .cache/region-scratch
export TMPDIR="$PWD/.cache/region-scratch"
export ANTITONE_REGION_EVIDENCE=/absolute/retained-evidence
export ANTITONE_RAW_ROOT=/absolute/read-only-samples
export ANTITONE_REGION_ROLL=roll-name
ANTITONE_REGION_PERF_ARM=cold ANTITONE_REGION_OUTPUT="$PWD/.cache/region-cold" \
  cargo test --locked --offline --release --features editor --lib \
  profile_complete_roll_area_preparation -- --ignored --nocapture
ANTITONE_REGION_PERF_ARM=warm ANTITONE_REGION_OUTPUT="$PWD/.cache/region-warm" \
  ANTITONE_REGION_PERF_COMPARE="$PWD/.cache/region-cold/report.json" \
  cargo test --locked --offline --release --features editor --lib \
  profile_complete_roll_area_preparation -- --ignored --nocapture
```

Reports contain per-stage samples (p50/p95/max), actual per-source native development counts and nested open/unpack/develop timings. **Sum exclusive stage times and inspect residual wall time** — the inclusive totals overlap. Each acquisition carries its source identity, so expected development counts can be checked exactly per source. A successful run measures preparation only, not import, numerical Analyze, UI latency or photographic border accuracy.

For scale on one complete twelve-source Sony roll, a cold baseline measured ~18 s of preparation, of which native development dominated (95%); retaining one source dropped it to ~16.6 s with identical areas. The job-local inference-scratch candidate measured ~9.5 s cold and ~8.4 s partially warm, with exact area/evidence/revision agreement — roughly half the acquisition cost — and a 36-source cold roll exercised the capacity fallback (46.6 s versus 56.8 s baseline) with unchanged peak footprint. These single-run observations motivated the scratch design; they are not latency gates. Re-measure after relevant changes.

### Marked crop quality and original-pixel inspection

Crop quality is a consumer question: report wanted content lost, unwanted border admitted and abstention separately. The read-only [crop diagnostic](../native/crop_quality/mod.rs) uses normal `.antitone` projects and the production proposal/margin functions, without running Analyze or accepting edits. Supply a private manifest with project paths relative to the manifest and marks/windows in **original developed-source coordinates** (including any active-area origin — not oriented, cropped or preview coordinates):

```json
{"margins_percent":[0,0.5],"cases":[{
  "project":"roll.antitone","source":1,"observation":"visible_frame",
  "marks":{"wanted":[[[100,100],[120,100],[120,120],[100,120]]],
           "unwanted":[[[80,100],[90,100],[90,120],[80,120]]]},
  "windows":[[80,80,128,128]]
}]}
```

`observation` is `visible_frame`, `censored` or `ambiguous`; optional `dense_wanted` marks have their own loss score. Marks are convex quadrilaterals wholly inside the active source. Scores use exact polygon/crop intersection; error areas sum supplied regions, so overlapping marks count repeatedly. Empty categories have a null error, not a successful zero; sparse marked areas must not be presented as whole-picture error percentages. Freeze independent marks before inspecting predictions.

```sh
mkdir -p .cache
ANTITONE_CROP_MANIFEST=/absolute/private/marks.json \
ANTITONE_CROP_OUTPUT="$PWD/.cache/crop-quality-FRESH" \
  cargo test --locked --offline --release --features project --lib \
  report_marked_crop_quality -- --ignored --nocapture
```

For prediction-free native windows, use the same manifest and fresh output, add `ANTITONE_RAW_ROOT=/absolute/read-only-samples`, select `--features editor` and replace the test name with `capture_original_crop_windows`. Each case needs independently chosen windows; capture is bounded to three explicit sources and twelve windows per source, each at most 256×256, under the same pressure/memory preflight and stops. The lossless `.rgb16` files contain little-endian original camera RGB16 in source orientation; PNGs are 4× nearest-neighbour viewing aids with only the sRGB transfer applied — no colour matrix, inversion, correction, repairs or grade.

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

Group scans deliberately; frame counts do not establish physical roll identity. Supply a private JSON array (`sources` are ordered photos; optional `reference` supplies correction; `area` is `[x,y,width,height]` statistics geometry):

```json
[{"name":"roll-name","sources":["/absolute/photo.ARW"],"reference":null,"area":null}]
```

```sh
ANTITONE_ROLL_CASES=/absolute/cases.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_OUTPUT" \
cargo test --locked --offline --release --features editor --lib diagnose_sample_rolls \
  -- --ignored --nocapture
.cache/roll-check/image-contacts sheet .cache/roll-check/positive-contact.png \
  .cache/roll-check/FRESH_OUTPUT/roll-name/*.tif
```

This drives real import/correction with a shared sampling area, bypassing per-photo review preparation; its approximate contacts do not establish acceptance of the interactive review. [Implementation](../src/editor/analysis/diagnostics.rs) owns report fields.

## Responsive image scopes

```sh
cargo test --locked --offline --features ui-tests --lib \
  scope_plot_scale_and_target_widgets_produce_valid_completed_reductions \

cargo test --locked --offline --features ui-tests --lib \
  compact_scope_header_fits_and_disclosure_preserves_state_at_both_densities \

```

The compact-header check covers normal/narrow 1×/2× layouts, disclosure and menu keyboard access, collapsed scheduling and unchanged project state; with `ANTITONE_UI_CAPTURE` it writes inspection captures. The authored test exercises [scope modes/amplification](scopes.md), real widgets and completed GPU reductions in both targets. Viewer and scopes must consume the same accepted working result.

## Investigating responsiveness

Capture a [debug session](#collecting-a-session-log) to inspect import/development, selected-preview/switch, thumbnail, acquisition and original-view events for a reported workload. Match phases and inputs; filesystem caching is uncontrolled. Backend Ready or paint-command recording is not GPU completion or physical presentation, and no minimum hardware target or general latency threshold is established. The `authored_` [worker checks](development.md#feature-specific-checks) use repository fixtures; their deadlines detect stalled tests, not unacceptable photographic latency.

### Separating input, queue work and presentation

Enable hosted [latency tracing](../src/platform/latency.rs) when kernel timings cannot explain an interaction:

```sh
ANTITONE_LATENCY_TRACE=1 ANTITONE_LOG_DIR="$PWD/.cache/logs" cargo app
# Requires an explicitly authorized, unoccluded desktop window:
ANTITONE_LATENCY_TRACE=1 ANTITONE_LOG_DIR="$PWD/.cache/logs" \
  cargo test --locked --offline --features editor --test native_host -- --run editor
```

Match `latency_frame`/`latency_completion` session events by their `id` field. Timestamps separate drawable acquisition, UI handling, encoding, submission and the present call; input starts at delivery to winit, not the physical gesture. Completion callbacks attach under the shared submission gate; callback observation includes polling/thread scheduling and is not a hardware GPU timestamp. `gpu_resource_bytes` reports Metal's native allocation size on macOS and wgpu's live Vulkan buffer/texture allocations on Windows, including deferred destruction but excluding unused allocator reservation. Neither measures all driver/process memory or resident VRAM; sampled peaks are not exhaustive. Cache defaults use physical local-heap capacity on discrete Windows GPUs, not these live counters or a dynamic driver budget. Measure same-PID RSS separately; on Apple Silicon these views overlap and must not be added.

For native scheduling and shared native builds, see [native development](native-development.md#editor-scheduling).

## Adjustment-layer processing and reuse

Adjustment layers use complete-stack editor admission. These diagnostics isolate original-sample processing and immutable snapshots; the [composed workflow checks](development.md#adjustment-layers-and-shared-consumers) separately exercise controls and normal workers:

```sh
ANTITONE_ADJUSTMENT_LAYER_PERF_QUICK=1 cargo test --locked --offline --release \
  --features editor --lib gpu::tests::adjustment_layer_performance -- --ignored --nocapture
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib real_raw_adjustment_layer_preview_tiles_and_refinement \
  -- --ignored --nocapture
```

For a bounded layer-stack ownership observation on the production working view:

```sh
cargo test --locked --offline --release --features editor --lib \
  physical_view_adjustment_layer_resources -- --ignored --nocapture
```

The [diagnostic](../src/gpu/view/tests/adjustment_layer_resources.rs) uses an authored RAW, a 1080p detail grid and the production whole-composition underlay with correction off/on, numeric/geometry edits and both targets. It samples same-process footprint/resident/Metal allocation and checkpoints current/building, deferred encoded consumers and retirement. Set `ANTITONE_SCREEN_RAW=1 ANTITONE_RAW_SAMPLES=/absolute/samples` for the supplied native-size source (headers must pass the existing 600-MiB diagnostic source bound); both variants use the existing pressure/free-memory/swap/growth stops, and timings without a coordinated window are indicative, not isolated interaction latency.

Original-sample processing shares the editor's f32 inversion for integer and continuous inputs. [Continuous-source checks](../src/gpu/tests/gpu_continuous.rs), [film-response checks](../src/gpu/tests/gpu_film_response.rs) and [original export checks](../src/gpu_export/tests.rs) exercise the model and exact floor flags.

## Equivalent ACES execution

The [execution comparison](../src/gpu_aces_execution_tests.rs) uses an independent fragment shell with the original constant array as the oracle for production's read-only-buffer layout, keeping pinned ACES functions, decimal constants and tables unchanged; the reference does not use the [production adapter](../src/gpu/aces.rs):

```sh
cargo test --locked --offline --features editor --lib \
  equivalent_aces_execution -- --nocapture
ANTITONE_SCREEN_PERF_SIZE=4k cargo test --locked --offline --release \
  --features editor --lib aces_execution_cost -- --ignored --nocapture
```

Require complete-raster bit agreement with the original shader plus the existing CPU gate for dense signed/HDR hue/exposure cases in both targets. Compute shells are independent execution references, not a second production renderer; timings exclude compilation, upload, readback, scopes and presentation.

## Physical-view editor prototype

[Editor ownership](editor.md#a-stable-working-view-while-grading) explains the normal view and purpose-bound original comparison/tools. Composed tests exercise controls, source/positive switching, navigation with acquisition blocked, idle stability, shared viewer/scopes and zero optional retention:

```sh
cargo test --locked --offline --features ui-tests --lib physical_view_ \
  -- --nocapture
ANTITONE_SCREEN_RAW=1 ANTITONE_RAW_SAMPLES=/absolute/samples \
  ANTITONE_SCREEN_PERF_SIZE=4k ANTITONE_SCREEN_CORRECTED=1 \
  cargo test --locked --offline --release --features editor --lib \
  physical_view_underlay_gpu_cost_and_retirement -- --ignored --nocapture
```

The production diagnostic covers initial detail/whole-composition acquisition, immutable edits/target changes, navigation composition and retirement. It is not input-to-display timing: compilation, scopes, host/presentation, concurrent UI and readback are excluded. `ANTITONE_SCREEN_PERF_SIZE` selects a workload, not a resolution cap; RAW correction is a synthetic 2×2 field, not measured photographic correction.

The authored acquisition/grade/regional [resource exercise](development.md#specialist-diagnostics) accepts `ANTITONE_GENERATED_CACHE_BYTES` to set optional payload retention at context creation; use `0` to measure regeneration. `ANTITONE_RESOURCE_OUTPUT` saves f32 physical/regional captures for pixel comparisons. Compare resource snapshots as well as repeated phase timings; the cache budget excludes required working storage and output images.

Run `cargo test --locked --offline --release --features editor --lib neutral_image_retention_resources -- --ignored --nocapture` alone to compare completed neutral-image retention against regeneration under the same controls. It retires caller-owned source wrappers between requests, verifies exact pixels, and reports lookup/reacquisition time, retained payload, actual GPU allocation and buffer creation. Clearing the cache must return optional ownership while independently held pixels remain valid; lookup timing excludes downstream grading and presentation.

For a matched 1536² resident-camera comparison, run `cargo test --locked --offline --release --features editor --lib resident_camera_execution_resources -- --ignored --nocapture` alone. It alternates separately materialized photographic/effect boundaries and whole-composition recording, checks exact AP1 output and reports per-iteration host/completion time, encoded-boundary GPU allocation, evaluation-buffer constructors and retirement. First evaluations include exercised-driver-cache compilation/setup; they are not genuine cold-compilation measurements.

For scale, the tested Sony/M5 Pro native-width workload (8640-wide originals) measures acquisition around 0.65 s uncorrected / 3.9 s corrected per view-sized replacement, fixed-view Global edits around 2–11 ms depending on display size, and composition checkpoints peaking around 280–950 MiB native resources that retire to ~2 MiB after completion. Re-measure after relevant changes.

[Coverage tests](../src/gpu/view/tests/underlay.rs) compare signed/HDR ramps, textures, valid black and missing support against an independent target-linear bilinear oracle. [Inspection tests](../src/gpu/view/tests/inspection.rs) cover camera-to-AP1/correction, saved-WB normalization and inspection EV in both targets; ordinary inspection must not schedule idle original refinement or acquire a photographic grade.

## Stable float32 view grading

The [working-view tests](../src/gpu/view/tests/colour.rs) exercise the actual `RenderedView` lifecycle and shared colour operators:

```sh
cargo test --locked --offline --features editor --lib gpu::view::tests::colour \
  -- --nocapture
```

[Model and geometry checks](../src/gpu/view/tests/colour/model.rs) cover Digital coverage, exactly-once exposure, asymmetric crops and rounded proxy dimensions. [Scalar/sampling/lifetime checks](../src/gpu/view/tests/colour/oracles.rs) cover continuous original-coordinate masks, corrected footprints, floor provenance and deferred encoded consumers after owner release. These isolate stage and sampling behavior from the working view's filtering-before-grading approximation.

## Digital starting brightness

Use supplied read-only RAWs to inspect [camera-reference initialization](processing.md#digital-starting-brightness) and shared-preview cost:

```sh
ANTITONE_EXPOSURE_SOURCES="/absolute/photo.ARW:/absolute/photo.dng" \
ANTITONE_EXPOSURE_OUTPUT="$PWD/.cache/digital-start-FRESH" \
  cargo test --locked --offline --release --features raw,dng-sdk,project,ocio-reference \
  --lib photo_preparation::photographic:: -- --ignored --nocapture
```

The source list uses the platform path separator (`:` on macOS, `;` on Windows). Reports separate header, preview, development, preparation and repeated-preparation costs. Optional small PNG panels show camera reference, normalized zero EV and initialized output through the CPU ACES reference. Inspect framing, brightness and retained colour differences together; the estimator matches a scalar, not the camera's rendering style. Unusable references report an explicit unavailable reason, so a successful diagnostic exit alone does not establish a brightness match.

## Photographic arithmetic

Use the [paired-output diagnostic](../tests/photographic_samples.rs) to inspect ordinary f32 photographic processing against the scalar/OCIO model:

```sh
ANTITONE_RAW_SAMPLES=/absolute/samples \
ANTITONE_PHOTO_OUTPUT="$PWD/.cache/photographic-FRESH" \
  cargo test --locked --offline --release --features raw,gpu,export,ocio-reference \
  --test photographic_samples -- --ignored --nocapture
```

It develops the train calibration references and three photographs (`DSC04520`, `DSC04522`, `DSC04524`) in Negative's unity-WB basis, writes paired 512×384 original-pixel TIFF crops in both targets, and varies correction, ordered adjustment layers and their blend endpoints/interiors. Reports include RGB16 difference distributions, counts above one 8-bit-code equivalent, nonclipped reference coverage and export time. Inspect the paired files before attributing a difference to precision; the outputs also include GPU-versus-OCIO rendering differences. Crop timings include export setup and encoding, and do not predict full-frame throughput. The [full-size exercise](#export) measures that workload separately.

## View sampling quality oracles

The [scalar sampling oracles](../src/view_sampling_tests.rs) compare representations with exact original-cell correction/inversion/grades, pinned ACES and linear-target averaging — offline numerical references, not an interactive CPU path:

```sh
cargo test --locked --offline --features ocio-reference --lib \
  view_sampling_tests
cargo test --locked --offline --release --features ocio-reference --lib \
  synthetic_view_sampling_quality -- --ignored --nocapture
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features raw,gpu,ocio-reference --lib raw_view_sampling_quality \
  -- --ignored --nocapture
```

Reported RMS/max errors are approximation diagnostics, not implementation tolerances or photographic thresholds. Averaging before nonlinear grades/ACES does not generally commute; filtered precreative input and footprint-centre masks are explicit preview approximations, and finite sampling candidates deliberately demonstrate aliasing/non-equivalence.

## RAW and complete GPU chain

For supplied-source metadata/development:

```sh
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- --metadata /absolute/photo.ARW
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- /absolute/photo.ARW
```

Metadata mode reports Sony embedded shading/distortion/CA availability, validated radial scales and automatic output dimensions without unpacking or correcting pixels, and can inspect malformed data that normal development rejects. Off is distinct from unavailable coefficients; discovery reads the selected RAW IFD, not rendered previews. Full mode develops the automatically corrected camera raster, checks repeated development and reports reconstruction time/storage separately from LibRaw time. Optional PPMs are clipped unmanaged diagnostics (`ANTITONE_RAW_PPM_EV` selects their exposure); [Sony processing](processing.md#embedded-sony-lens-correction) owns coordinates and reconstruction.

On macOS, the private X-Trans exercise checks import, unity/as-shot WB, Save/Open, both display targets and reopened 192×128 TIFF/EXR exports. Supply a colon-separated list of X-Trans RAFs; Negative uses synthetic anchors to exercise inversion, not assess film calibration:

```sh
ANTITONE_XTRANS_SAMPLES="/absolute/first.RAF:/absolute/second.RAF" \
  cargo test --locked --offline --features editor --lib \
  raf_import_reopen_view_and_bounded_exports -- --ignored --nocapture
```

Per-component distortion/CA maps and independent shading checks run offline in the pure model tests; authored native cases separately exercise automatic development:

```sh
cargo test --locked --offline --no-default-features \
  --test sony_lens --test sony_geometry --test sony_vignetting
cargo test --locked --offline --features raw,project --test raw_sony_lens
cargo test --locked --offline --features gpu --lib gpu::tests::gpu_continuous
# Add the normal OCIO reference setup for reopened Sony TIFF/AP0 EXR correspondence.
cargo test --locked --offline --features raw,gpu,export,project,ocio-reference --test raw_sony_lens
```

For scale, bounded release-build runs on an Apple M5 Pro / 48 GiB with ILCE-1M2 / 24–70mm GM II photographs measured roughly 0.22–0.27 s per ~50-MP source for distortion/CA reconstruction (0.42–0.49 s with embedded shading), ~600 MB corrected RGB storage per source and ~1.2 GB peak footprint for the serial CLI probe, with exact sampled repeatability and passing matrix gates. These are spot-checks, not vendor calibration or editor-switching latency acceptance.

Full mode reports flooring rates and checks repeatability and eligible in-sRGB LibRaw matrix comparisons — inspect `compared_grid`; zero eligible comparisons cannot establish agreement. [Authored RAW checks](development.md#feature-specific-checks) preserve Scan immutability, measured anchors (`2e-12`), flooring (at most 1%) and sampled LibRaw matrix agreement (`2/65535`).

For independent upstream OCIO → Metal:

```sh
./scripts/probe-ocio.sh
cargo test --locked --offline --release --features raw,gpu \
  --lib gpu::tests::camera_raw -- --ignored --nocapture
```

The OCIO probe exercises upstream-generated AP1 rendering inputs and LUTs. The separate authored RAW diagnostic exercises camera processing through canonical GPU evaluation.

For corrected affine reconstruction against scalar coverage:

```sh
cargo test --locked --offline --features editor,ocio-reference --lib \
  --test scopes --test viewer --test affine_resample --test geometry_export
cargo test --locked --offline --release --features editor,ocio-reference --lib \
  corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

### Sony source storage precision

The opt-in [storage study](../src/gpu/tests/lens_precision.rs) holds development and numerical sampling support fixed while comparing retained f32 camera codes with whole-code rounding and an eight-fractional-bit diagnostic alternative (within the native range these correspond to RGB16 and Q16.8). Synthetic cases blend declared integer neighbours; supplied negatives use diagnostic calibration and borrowed Sony warps, because their adapted lens has no usable profile — those warps test resampling/rounding, not physical lens calibration. Storage errors use the f64 scalar pipeline followed by independent pinned OCIO rendering to sRGB; GPU timing arms are checked against their scalar inputs under the existing numerical gate.

On one M5 Pro / 48-GiB distortion/CA-only run, maximum differences expressed in encoded 8-bit-level equivalents were:

| Case | Largest RMS across tested settings | Largest channel difference |
| --- | ---: | ---: |
| Ordinary negative, global/detail, including spatial gains | 0.014 | 0.108 |
| Dense reference negative, including spatial gains | 0.167 | 1.818 |
| Sony photographs, source inspection | 0.030 | 0.272 |

Within its representable range, RGB16 adds at most half a native code. The dense reference's rare roughly-two-level output changes show rounding is not always negligible after inversion. Q16.8 reduced those differences substantially (dense-case maximum below 0.008 encoded 8-bit levels) but needs custom packing and still requires continuous inversion. The study supports a tradeoff — within-range packed corrected RGB16 could halve retained storage but would need a range/scale design to preserve shading headroom — not a universal source-format requirement, and no production format change follows automatically from it.

### Direct DNG SDK probe

The `dng-sdk` feature exposes a read-only probe of the vendored SDK backend without the editor or LibRaw. The native library keeps its validation instrumentation; the Adobe validator executable is not built.

```sh
# Explicit main-sensor pixel bound; inputs remain read-only.
cargo run --locked --features dng-sdk --bin antitone-dng-probe -- 1000000 /path/to/authored.dng
# Compare bounded execution budgets; reports split timings and a camera-pixel digest.
cargo run --locked --release --features dng-sdk --bin antitone-dng-probe -- \
  --threads 8 60000000 /path/to/photo.dng
```

## Rendering, gamut and ICC

CPU rendering/ICC regenerates its OCIO references without preceding Metal checks or cached input:

```sh
./scripts/probe-rendering.sh
cargo test --locked --offline --features ocio-reference --test rendering
./scripts/probe-gamut.sh
cargo run --locked --offline --features ocio-reference --bin antitone-rendering-probe -- --gamut-check
```

Rendering scripts share `scripts/generate-ocio-reference.sh`, which regenerates upstream CPU/GLSL/LUT artifacts; failures do not fall back to cached files. CPU/GPU agreement, transfer/profile checks and independent ColorSync/Core Image checks address different boundaries. [OCIO artifacts](../assets/ocio/README.md) and [profiles](../assets/profiles/README.md) retain provenance.

## Export

[Export checks](development.md#export-checks) distinguish production GPU/worker workflows, exact encoder serialization and independent scalar/OCIO photographic references. [GPU exports](../tests/gpu_export.rs) reopen original-sample TIFF/AP0 EXR with ordered layers, correction, orientation and straightening; [reference export tests](../tests/export.rs) retain scalar file-code/profile/compression and failure checks behind `ocio-reference`. No reference is a production fallback.

During an agreed GPU window, the opt-in full-size resource exercise retains production physical detail/underlay while exporting and then regrading that retained source:

```sh
ANTITONE_RAW_SAMPLES=/absolute/samples \
  cargo test --locked --offline --release --features editor --lib \
  full_size_export_resources_with_retained_working_view \
  -- --ignored --nocapture
```

It reads the three train/calibration RAW inputs read-only, checks native geometry before development, and keeps the existing pressure, swap-counter, footprint-growth and telemetry stops. `ANTITONE_EXPORT_RESOURCE_CASE` selects `native`, `rotated`, `corrected-native` or `corrected-rotated`; `ANTITONE_EXPORT_RESOURCE_FORMAT=tiff` narrows a run to TIFF; `ANTITONE_EXPORT_RESOURCE_LOCALS=2` adds ordered linear and inverted radial locals (the post-export regrade preserves them); `ANTITONE_EXPORT_RESOURCE_BACKEND=gpu` selects the production engine (the default), while adding `ocio-reference` to the features enables `reference` or `compare` (reference followed by production GPU) for matched independent scalar/OCIO-reference measurements with identical source, grade, composition and encoders.

Record power source and configured mode with timing observations (`pmset -g batt` and `pmset -g custom` on macOS). Battery operation or a different power mode can alter CPU/GPU clock budgets, so comparisons that straddle a power transition are not matched latency evidence; numerical/file equality remains valid. Do not change system power settings, purge caches or interrupt other applications to improve a result without agreement.

### Shared export precision and pixel-endpoint checks

The [shared engine](../src/gpu_export/area.rs) processes conservative real-source bounds plus a real-cell halo per destination block and reads back destination pixels only; exact crops, rotated crops and repaired edges all use it, with exact whole-frame source-cell masks keeping repair relevance valid under native mapping roundoff. The [worker entry](../src/gpu_export/job.rs) validates the captured source basis and resolves cold repairs independently of viewer warmth.

[Independent stage checks](../src/gpu_export/area/stage_oracle_tests.rs) combine canonical repairs, source-point grades, a scene look, scalar f64 footprint weights and CPU/OCIO target rendering. [EXR checks](../src/gpu_export/common_exr_tests.rs) exercise the public engine in both storage precisions under the common working allowance `2e-5 + 2e-4 × |reference|` (storage quantization separate), with signed/HDR inputs, invalid-input masks and selected tighter TIFF cases. [Continuous-source worker checks](../src/gpu_export/job/tests/corrected_source.rs) exercise authored Sony fractions/headroom plus residual correction; [lifetime checks](../src/gpu_export/area/look_lifetime_tests.rs) count cold/warm LUT allocations through changing scratch dimensions and owner release.

For TIFF pixel-endpoint measurements, the test-only [ready-to-encode sink](../src/export_timing/pixels.rs) receives complete RGB16 frame codes through production block callbacks without opening a destination or invoking a codec. File-writing/reopening comparisons must use the identical captured recipe and match those codes; report source acquisition, renderer/kernel preparation and codec/I/O separately (prewarm is startup work, not free computation).

### TIFF encoding cost

TIFF encoding is separate from producing ready RGB16 pixels. The current encoder uses lossless Deflate at level 1, with horizontal prediction and eight-row strips; the compression tag is 8 rather than LZW's 5, which changes file representation, not the photograph or its samples.

A bounded frozen-frame investigation attributed about 2.2 s to the preceding LZW codec versus about 0.005 s prediction and ~0.13 s file-write on the same frames. LZW slightly expanded 256 MB of grainy RGB16 samples into 260–261 MB; fast Deflate produced approximately 205 MB and reduced frozen-frame encode-and-save from roughly 2.4 s to 0.84 s. One balanced normal-worker window measured total Save means drop from ~6.9 s to ~5.2–5.4 s (approximately 21–25% shorter) with exact reopened samples. Uncompressed TIFF was faster still but forfeits substantial size savings on smooth content; authored flat/ramp/noise checks and native ImageIO RGB16/dimension/ICC reopening support the lossless Deflate choice.

These are one-source/one-device observations, not universal percentages. ImageIO reopening is an independent decoder oracle for the encoded file, not for the rendering that produced the samples.

### Earlier bounded-reuse observations

These measurements describe the *preceding* coverage/cache/pair exporter, not the shared original-sample engine; the corresponding backend selectors are no longer maintained. On one already-developed 49.77-MP source, batching two originals measured ~5% lower full-export latency with identical files, and admitted 1×5 traversal/future-use retention cut original evaluations from 4,062 to 3,333 for a 10,559-request case, averaging ~12% lower total export latency with complete-file identity. One local comparison regressed and battery operation was reported afterward — that adverse evidence is retained rather than attributing every difference to the algorithm.

### Reading export resource reports

Every output pixel is reopened through bounded TIFF strips or a nonparallel EXR counting/finite-value sink; decoder limits are not disabled. GPU phase traces include source preparation, bounded area planning, GPU/readback waits and output packing; reference totals include scalar photographic processing, reconstruction and encoding. Reports count evaluated source pixels, dispatches, destination readback bytes, stop checks, sampled process footprint/resident memory/Metal allocation and sample age — **those quantities overlap and must not be added**.

Opt-in `export_stage` aggregates and host `export_detail` rows subdivide the same work (setup, coverage planning, tile access, weighted reduction, encoding, packing, finalization; upload inside command encoding, map/wait/copy/unmap inside tile access). Some rows explain residual work; others subdivide parent intervals — **do not add nested or overlapping costs to the stage total**, and use calibrated pass counters for narrower GPU attribution. Queue-write and mapped-copy times measure host operations, not isolated device transfer bandwidth; completion wait includes queued GPU processing and host/driver scheduling. Field definitions live with the report generators in [export resource tests](../src/gpu_export/resource_tests.rs); read them there rather than duplicating the full schema here.

If other GPU consumers are not known to be idle, timings are indicative rather than isolated cost evidence. This is a bounded observation on the supplied camera/device, not forced pressure, an aggregate admission guarantee or broad photographic acceptance.

### Preflight and independent reopening

For read-only preflight diagnosis without RAW loading or export, run `cargo test --locked --offline --release --features editor --lib resource_preflight_only -- --ignored --nocapture`. The unchanged guard requires Normal pressure and 2 GiB non-speculative free memory; low non-speculative free memory alone is not a pressure alarm (macOS already counts speculative pages in its total). Do not purge caches or relax the guard to force a measurement.

ImageIO provides independent TIFF reopening:

```sh
cargo test --locked --offline --features raw,export,ocio-reference --test raw_dng \
  authored_dng_tiffs_reopen_with_independent_imageio_storage_and_profiles \
  -- --ignored --nocapture
```

This compiles [the native checker](../native/tiff_reference.mm), creates all products in an isolated temporary directory, and checks dimensions, RGB16 storage and pinned ICC bytes including malformed/wrong/multi-image/RGB8 rejection.

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --submission-stress
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --interactive
```

Submission stress brings its own window forward and checks repeated native resizing, background completion and presented frames through the shared gate; zero submitted frames fails. Native application automation/window exercises require explicit authorization. Layer/encoding/readback agreement does not establish physical monitor matching, transitions, macOS 14 or broader devices; keep those limits distinct from the tested [output boundary](colour-and-output.md).
