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

Use [authored analysis checks](development.md#roll-analysis) for transactions and estimator regressions: picture-area controls, confirmation/cancellation/failure, atomic initialization, Save/Open and both output targets, including correction. They are offscreen, not photographic certification.

An optional regression checks opening recovery and holder-tail exclusion on one twelve-frame feedback scan set:

```sh
ANTITONE_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features editor --lib real_ambiguous_first_frame_uses_roll_opening_instead_of_holder \
  -- --ignored --nocapture --test-threads=1
```

Its named files and area/Dmax assertions apply only to that case. Other supplied rolls use an explicit manifest or the editor; neither route certifies [photographic success](plans/roll-assistance.md) or a general opening detector.

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

The diagnostic drives actual import/correction/Analyze and approximate contacts. Inspect failures and every image. [Implementation](../src/editor_analysis_diagnostics.rs) owns report fields.

## Responsive image scopes

```sh
cargo test --locked --offline --features ui-tests --lib \
  scope_plot_scale_and_target_widgets_produce_valid_completed_reductions \
  -- --test-threads=1
```

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
cargo test --locked --offline --features render --lib \
  view_sampling_tests -- --nocapture --test-threads=1
cargo test --locked --offline --release --features render --lib \
  synthetic_view_sampling_quality -- --ignored --nocapture --test-threads=1
ANTITONE_RAW_SAMPLES=/absolute/samples cargo test --locked --offline --release \
  --features raw,gpu,render --lib raw_view_sampling_quality \
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
cargo test --locked --offline --features editor --lib \
  --test scopes --test viewer --test affine_resample --test geometry_export
cargo test --locked --offline --release --features editor --lib \
  corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

## Rendering, gamut and ICC

CPU rendering/ICC regenerates its OCIO references without preceding Metal checks or cached input:

```sh
./scripts/probe-rendering.sh
./scripts/probe-rendering.sh --camera
cargo test --locked --offline --features render --test rendering
./scripts/probe-gamut.sh
cargo run --locked --offline --features render --bin antitone-rendering-probe -- --gamut-check
```

Rendering scripts share `scripts/generate-ocio-reference.sh`, which regenerates upstream CPU/GLSL/LUT artifacts. Existing camera artifacts are ignored unless requested; failures do not fall back to cached files. CPU/GPU agreement, transfer/profile checks and independent ColorSync/Core Image checks address different boundaries. [OCIO artifacts](../assets/ocio/README.md) and [profiles](../assets/profiles/README.md) retain provenance.

## Export

[Export tests](../tests/export.rs) check TIFF codes/profile/compression, signed/HDR AP0 EXR and destination safety. [Authored RAW tests](development.md#feature-specific-checks) additionally decode, calibrate and process model-4 input before export.

ImageIO provides independent TIFF reopening:

```sh
cargo test --locked --offline --features raw,export --test raw_dng \
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
