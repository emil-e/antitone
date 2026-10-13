# Specialist build and verification probes

Use [the README](../README.md) for setup/build/run and [development checks](development.md) for everyday and feature-specific tests. This guide covers numerical/reference comparisons, explicit RAW tests, export reopening and scripted presentation. Remaining acceptance is tracked in [the roadmap](../PLAN.md).

## Prerequisites and interpreting results

Run from the repository root on macOS/Apple Silicon. Native features use the pinned local libraries and Apple Clang configured by the shared setup/Cargo path. OpenMP's source-built arm64/macOS-14 runtime and local licenses are described in [the native guide](native-development.md); deployment targets and checkout-local rpaths do not establish distributable packaging or actual macOS-14 acceptance. Little CMS remains statically built through Cargo; `LCMS2_LIB_DIR` is not a supported override.

Commands below use `--offline` with populated caches. Omit it for a first build, retaining `--locked`. RAW probes need explicitly supplied read-only local samples; setup never downloads photographs. Photographic outputs remain ignored/private.

Numerical gates, scripted lifecycle/visible-frame checks and physical/device acceptance address different boundaries. A missing device, locked/occluded window or zero-visible-frame smoke is a verification limit or failure, not a passed acceptance check.

## Editor and layout smoke

```sh
cargo build-app --offline
target/release/antitone samples/DSC04416.ARW
# Explicit diagnostic calibration, not the user's saved grade:
target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
# Six normal/narrow Roll/Photo/Rendering layout holds:
ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

Launch a saved document with `target/release/antitone /path/to/grade.antitone.json`. Smoke exercises actual GPU/worker/layout paths with diagnostic calibration and model-4 settings, not photographic calibration or broader-device performance.

For Save/Open, append one additional unvisited source:

```sh
target/release/antitone --smoke-project \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW samples/DSC04393.ARW
```

This uses temporary project files, leaves RAWs untouched, and verifies exact model-4 response state, custom saved WB after New/different-WB import, unvisited records, working/sRGB/P3 pixel reproduction, live refinement and failed Open preservation. A zero-onscreen-frame result remains failure; a locked macOS session can pass persistence assertions but fail this presentation gate.

The flat-field editor/worker integration has an explicit read-only local-RAW/Metal test (temporary projects/32×32 corrected TIFF/AP0 EXR crops):

```sh
cargo test --locked --offline --release --all-features --lib \
  editor::flat::tests::real_reference_editor_save_open_and_corrected_exports \
  -- --ignored --nocapture
```

It uses `DSC01739`/`DSC01737`, tests cancellation/failure, automatic support acceptance, corrected thumbnail completion, Save/Open and Enabled/view preservation. It is headless; it does not establish actual reference-picker/rectangle onscreen or native-dialog acceptance.

Scripted corrected onscreen acceptance (keeps the nonzero-visible-frame gate and rejects early window closure):

```sh
target/release/antitone --smoke-flat-field samples/DSC01737.ARW samples/DSC01739.ARW
target/release/antitone --smoke-flat-field samples/DSC01811.ARW samples/DSC01813.ARW
cargo test --locked --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

The host smoke renders Fit/reference support, exercises scripted geometry move/resize/cancel and support edits, verifies actual corrected Dmin/Dmax/Gamma/WB backend results against independent CPU calls, both-target settled previews/thumbnails, +2 EV native-pixel inspection, Enabled/Clear and exact Save/Open. It does not automate the native file dialog or physical gestures. The explicit full-size test writes temporary 8640×5760 sRGB/P3 TIFF and half/full AP0 EXR, decodes every pixel, checks metadata and 247 independent scalar-derived positions, and verifies signed/HDR EXR range. Both commands leave RAWs unchanged.

### Analyze roll

Focused scalar/persistence checks and explicit read-only 120 RAW/Metal worker regressions:

```sh
cargo test --locked --offline --no-default-features --test roll_analysis --test roll_sampling --test camera_inversion --test processing --test film_response --test photographic
cargo test --locked --offline --release --lib roll_analysis::
cargo test --locked --offline --features project --test camera_project
cargo test --locked --offline --release --features editor --lib editor::analysis::tests:: -- --test-threads=1
cargo test --locked --offline --release --features gpu --test gpu_processing --test gpu_film_response
ANTITONE_SAMPLES="$PWD/samples" cargo test --locked --offline --release --features editor --lib \
  editor::analysis::tests::real_ -- --ignored --nocapture --test-threads=1
```

The RAW tests use `DSC04299`–`DSC04310` and `DSC04312`–`DSC04323`. They check cancellation/failure, adequacy, atomic full Photo-grade reinitialization with composition preservation, Save/Open and shared opening recovery when the first frame has no local suggestion. Samples remain unchanged. Numerical bounds detect holder-tail contamination, not calibrated colour accuracy.

With an unlocked desktop and the release editor built, the scripted visible-window workflow exercises area acceptance, atomic Analyze and both settled targets:

```sh
target/release/antitone --smoke-analysis - \
  samples/DSC043{21,22,23,12,13,14,15,16,17,18,19,20}.ARW
# Replace '-' with a matching bare-light reference RAW to exercise correction.
```

The nonzero-visible-frame gate remains required. These checks do not establish photographic quality or physical area-gesture/monitor acceptance. [Current behavior](contracts/editor-controls.md#analyze-roll), [remaining acceptance](plans/roll-assistance.md).

### All-roll visual diagnostics

`native/image_contacts.m` creates private ImageIO source previews/contact sheets for visually identifying picture/gray/white/leader boundaries. These source thumbnails are not LibRaw/processing acceptance evidence. Build and keep outputs under an ignored directory:

```sh
mkdir -p .cache/roll-check
/usr/bin/clang -fobjc-arc -Wall -Wextra -Werror native/image_contacts.m \
  -framework Foundation -framework ImageIO -framework CoreGraphics -framework CoreText \
  -o .cache/roll-check/image-contacts
.cache/roll-check/image-contacts previews .cache/roll-check/source /absolute/samples/*.ARW
.cache/roll-check/image-contacts metadata .cache/roll-check/source-metadata.json /absolute/samples/*.ARW
.cache/roll-check/image-contacts sheet .cache/roll-check/source-contact.png .cache/roll-check/source/*.png
```

After visual grouping, supply a private JSON array of cases: `[{"name":"roll-name","sources":["/absolute/photo.ARW"],"reference":null,"area":null}]`. Sources are ordered pictures; an optional matching bare-light reference uses the existing correction workflow, and optional `area` is explicit `[x,y,width,height]` statistics geometry. Do not infer physical roll identity from frame count alone or silently mix a rescan with another capture setup.

```sh
ANTITONE_ROLL_CASES=/absolute/cases.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_OUTPUT" \
cargo test --locked --offline --release --features editor --lib diagnose_sample_rolls \
  -- --ignored --nocapture --test-threads=1
.cache/roll-check/image-contacts sheet .cache/roll-check/positive-contact.png \
  .cache/roll-check/FRESH_OUTPUT/roll-name/*.tif
```

The diagnostic drives the actual Editor import/correction/Analyze transaction on Metal. Each case reports failure unchanged or installed state, full proposal, local support and picture rejection counts. Per-frame channel tails and counts at/below proposed Dmax audit original eligible populations before equal-frame endpoint resampling. Successful cases render every source through the actual scalar Pipeline and CPU ACES into 384px nearest-centre sRGB TIFF previews. These are deliberately approximate contacts, not area-averaged settled/export acceptance. The test succeeds when the diagnostic completes, **not** when every roll is photographically acceptable; inspect statuses, render errors and all images. Generated reports, projects and photographic images remain private/ignored. Use a fresh output directory; samples and saved grades are untouched.

#### Appearance counterfactuals

To separate exposure, near-base response and Dmin effects for an installed diagnostic proposal, use the private baseline report (with an automatic support matching its picture area) and a fresh output directory:

```sh
ANTITONE_ROLL_REPORT=/absolute/baseline/roll-name/report.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_APPEARANCE" \
ANTITONE_ROLL_FRAMES=DSC04495,DSC04496,DSC04515 \
ANTITONE_ROLL_VARIANTS=baseline,EV-minus-1,toe-1-matched-first-frame,first-frame-only-Dmin,mean-key,median-frame-mean-key \
cargo test --locked --offline --release --features editor --lib diagnose_analyzed_appearance \
  -- --ignored --nocapture --test-threads=1
```

The probe audits original film/picture populations across the complete roll, including per-frame positive pre-Photo arithmetic mean luminance and endpoint contributors. Named frames render at 1024px nearest-centres with the unchanged Pipeline/CPU ACES. Omit `ANTITONE_ROLL_VARIANTS` for all variants, including −0.5/−1.5 EV and a re-analyzed toe-1 response. The matched toe-1 variant aligns the first ordered frame's median brightness; first-frame-only Dmin uses the same band estimator on that frame's film pool, not a validated manual film-base patch. Mean keys change exposure only and are experiments, not application controls or accepted replacement behavior. Candidate margin patch coordinates in the report are unvalidated Sony diagnostic regions, not material detection. No grade is installed or saved.

For a complete cross-roll exposure-only comparison, repeat against each installed baseline report with `ANTITONE_ROLL_VARIANTS=baseline,median-frame-mean-key` and all of its ordered source stems in `ANTITONE_ROLL_FRAMES`. A private batch can obtain those stems with:

```sh
python3 -c 'import json,pathlib,sys; p=json.load(open(sys.argv[1]))["project"]; s={v["id"]:v for v in p["sources"]}; print(",".join(pathlib.Path(s[i]["path"]).stem for i in p["order"]))' /absolute/report.json
```

Use a fresh directory per case; contacts sorted by filename pair baseline with candidate. Inspect every pair, including already-dark pictures. For post-fit outlier sensitivity, take `median(log2(pre_photo_positive_picture_mean_y))` from the audit, replace one frame at a time with a log-key 32 stops beyond either observed extreme, and compare the medians. The EV movement is the negated key movement. Holding endpoints/balance fixed isolates the exposure statistic; it does not test the full estimator's resistance to damaged endpoints.

#### Per-photo exposure counterfactuals

Use an appearance report containing `baseline_project` and the `median-frame-mean-key` variant, not an Editor baseline report directly:

```sh
ANTITONE_ROLL_REPORT=/absolute/appearance/roll-name/report.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_PHOTO_EXPOSURE" \
ANTITONE_ROLL_FRAMES=DSC04495,DSC04510,DSC04515 \
ANTITONE_PHOTO_VARIANTS=roll-only,photo-blend99,photo-robust-blend99 \
cargo test --locked --offline --release --features editor --lib diagnose_photo_exposures \
  -- --ignored --nocapture --test-threads=1
```

The candidate roll, references and all non-exposure Photo settings stay fixed. Proposals are additive EV changes to diagnostic Photo copies, never installed/saved. Original eligible picture pixels provide positive Y populations (at least 512); original signed/HDR pixels render through the actual Pipeline/CPU ACES at 1024px nearest-centres. Reports assert original project state is unchanged and include each proposed EV, mean/p99 placements and a synthetic one-hot-positive sensitivity check (first positive replaced by `65536*p99`, not a simulated RAW defect).

Omit `ANTITONE_PHOTO_VARIANTS` to compare all controls: `roll-only`, `photo-mean` (mean→0.18), `photo-highlight99` (p99→1), `photo-blend95`, `photo-blend99` (average of mean/highlight EV corrections), and `photo-robust-blend99` (same blend, but the mean's values above p99 contribute as p99). The upper-tail limit bounds only the statistic; no source or working-pixel clamp. Quantiles use linear interpolation over positive Y. Targets are uncalibrated AP1 scene-linear priors, not display white or clipping limits. These per-photo estimators cancel a common roll exposure shift; the roll/Photo EV split is not an additional normalization benefit. The production initializer selects the robust p99 blend after gamma/shared roll WB, with median total EV in Roll and residuals in Photo; explicit overwrite/reset semantics belong to the [Analyze contract](contracts/editor-controls.md#analyze-roll), not this fixed-colour counterfactual.

For a complete scan-set comparison, list all ordered stems from the report's `baseline_project` (the earlier extraction command uses `project`), select `roll-only,photo-robust-blend99`, and use a fresh output per case. Construct contacts in report `frames`/`variants` order, roll-only first: lexical `*.tif` sorting puts `photo-...` before `roll-only` and reverses that visual convention.

#### Gamma-balance influence counterfactuals

The gamma influence/tonal/drift counterfactuals consume a **mean-neutral gamma control** per-photo exposure report with zero photographic roll WB, all ordered sources and `photo-robust-blend99` proposals. Their arithmetic-mean reconstruction guard deliberately does not accept an arbitrary grade from the newer tonal-drift initializer. The probes retain fixed colour/exposure controls rather than installing or reinitializing a project. The influence probe reconstructs global gamma 0.65 with zero starting gamma-balance coordinates:

```sh
ANTITONE_ROLL_REPORT=/absolute/photo-exposure/roll-name/report.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_GAMMA_BALANCE" \
ANTITONE_ROLL_FRAMES=DSC04495,DSC04509,DSC04510,DSC04515 \
ANTITONE_BALANCE_LOO=1 \
cargo test --locked --offline --release --features editor --lib diagnose_gamma_balance \
  -- --ignored --nocapture --test-threads=1
```

All sources are sampled with the production eligibility/positive-selection rules, equal original counts and 2,048 balance-sample cap. Named frames render all six variants: `current`, `tail99`, `tail95`, `frame-mean`, `native99`, `native95`. Tail weights are `min(1,cutoff/metric)` with each frame's weight sum restored to its original count. The metric is unbalanced pre-exposure AP1 Y, or `max(native_positive_RGB / camera_neutral_target)` for native variants. Weights are frozen before fitting, joint across RGB, never colour clustering or source/working-pixel modification. The separate frame-mean control uses inverse baseline mean Y per frame, equalizing initial luminance contribution rather than sample weight. All fits reuse the actual-response weighted neutral solver/target and unchanged numerical gates.

Reports audit native channel contributions per frame, brightness-ranked 1/5/10% tail shares, pre-fit/fitted AP1 means, and every single-frame omission fit with **endpoints fixed**. Omission failures are explicitly reported, never replaced by another fit; actual product sources remain included. Set `ANTITONE_BALANCE_LOO=0` to omit that diagnostic. Roll/Photo EV, global gamma/response, endpoints, basis and Photo WB stay fixed during 1024px nearest-centre Pipeline/CPU ACES rendering; no exposure key is recalculated. Changing channel gammas can nevertheless change luminance. Every rendered variant and the original project snapshot are checked for non-balance changes. Assemble contacts in report variant order, not wildcard filename order. These probes separate balance-mean leverage from endpoint contamination; they do not establish calibrated scene colour or a production robust estimator.

#### Gamma tonal-objective counterfactuals

Use the same per-photo exposure report and a fresh output directory:

```sh
ANTITONE_ROLL_REPORT=/absolute/photo-exposure/roll-name/report.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_GAMMA_TONES" \
ANTITONE_ROLL_FRAMES=DSC04495,DSC04510,DSC04515,DSC04526 \
cargo test --locked --offline --release --features editor --lib diagnose_gamma_tones \
  -- --ignored --nocapture --test-threads=1
cargo test --locked --offline --features editor --lib known_neutral_ramp_recovers_one_gamma_fit_across_tones
```

The shared original-sample loader reconstructs the current fit before testing frozen unbalanced-Y ranks: shadow/middle/light 1–33/33–67/67–99% bands, both per-frame and pooled. Relative bands keep equal frame counts; pooled counts are reported and may differ. Tonal-fit failures are reported, not substituted. No band is an identified gray/neutral material. `pixel-root-Y` uses frozen inverse-square-root Y weights with each frame's total normalized back to its count.

The `span-axis` parameter control sets channel gamma ratios proportional to `ln(Dmin/Dmax)` while retaining the selected global gamma and response. It preserves the chromaticity of a **model-defined straight normalized-density axis**, not a measured film gray reference. `span-axis-WB` additionally fits the existing Bradford WB to the same whole-roll positive mean. `constant-WB-control` uses zero gamma-balance coordinates plus Bradford correction of that unbalanced mean. These are diagnostic copies using existing processing operators, not selected model/initializer changes. Operator controls expect zero original Photo WB; their nonzero WB is explicit. Current/global settings, all exposure values, source data and ACES are unchanged; no exposure recalculation or installation/saving occurs.

Reports contain all fits and frame counts, per-picture tonal AP1 means and RGB/Y, synthetic mean-colour and normalized-density model-axis responses, and the rendered settings. Known-neutral synthetic tests use unequal camera-neutral components and verify one true gamma fit across dark/middle/light populations; they do not validate the actual roll endpoints or scene-grey assumptions. Named pictures render all successful variants at 1024px nearest-centres through the actual Pipeline/CPU ACES. Assemble contacts in report `rendered`/`variants` order, not wildcard order. Changed gamma/WB can change luminance even with exposure fixed; RGB/Y measures separate that from chromatic changes. No photographic, corrected-roll, GPU or calibrated-monitor acceptance follows from a successful run.

#### WB-invariant gamma optimization and WB comparison

The diagnostic-only drift optimizer uses the same complete per-photo exposure report, original bounded sample loader and frozen per-frame tonal ranks:

```sh
ANTITONE_ROLL_REPORT=/absolute/photo-exposure/roll-name/report.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_GAMMA_DRIFT" \
ANTITONE_ROLL_FRAMES=DSC04495,DSC04510,DSC04515,DSC04525 \
cargo test --locked --offline --release --features editor --lib diagnose_gamma_drift \
  -- --ignored --nocapture --test-threads=1
cargo test --locked --offline --release --features editor --lib gamma_drift_recovers_known_curves_and_ignores_wb_and_exposure
```

For each frame/band, invert original pixels, average, map to AP1 and take log-opponent Bradford cone coordinates. The scalar score is the sum of squared **equal-frame mean** shadow-minus-mid and highlight-minus-mid differences. Average across frames before squaring; do not require each picture's differently coloured materials to be gray. Fixed populations and differencing make the score invariant to constant per-frame WB/exposure. Nonpositive band-mean cone responses make a candidate undefined; candidates are counted/rejected without deleting samples. Other errors propagate. A deterministic eight-direction pattern search uses current/zero/density-span starts, only the existing balance range, and coordinate resolution at most `1e-7`. Reports retain all starts/scores/residuals and rejection counts. This is a scene-statistical proxy, not physical stock characterization or an accepted production optimizer.

After curve optimization, fit existing mean-neutral Bradford WB separately to either equal-count whole-roll means or individual-frame means. Render in this order: `current`, `optimized-pre-WB`, `optimized-roll-WB`, `optimized-photo-WB`. Source data, endpoints, selected global gamma/response, geometry and all roll/Photo exposure values stay fixed; WB may nevertheless change luminance. This diagnostic represents common WB in **Photo copies** to preserve its input project. Production instead uses the independent persistent roll WB control; do not apply the common correction twice. Invalid/out-of-range WB is explicitly reported without clipping or substituting identity; all frames still contribute to gamma fitting. Reports assert unchanged original project state and verify each available WB leaves tonal drift unchanged.

The native contact tool retains six columns by default; `--columns 4` makes a four-control row. Supply files in report order, not lexical wildcard order:

```sh
.cache/roll-check/image-contacts sheet .cache/roll-check/comparison.png --columns 4 \
  .cache/roll-check/FRESH_GAMMA_DRIFT/DSC04495-current.tif \
  .cache/roll-check/FRESH_GAMMA_DRIFT/DSC04495-optimized-pre-WB.tif \
  .cache/roll-check/FRESH_GAMMA_DRIFT/DSC04495-optimized-roll-WB.tif \
  .cache/roll-check/FRESH_GAMMA_DRIFT/DSC04495-optimized-photo-WB.tif
```

These 1024px nearest-centre Pipeline/CPU ACES comparisons do not establish settled-preview, corrected-roll, export, stock/process or calibrated-colour acceptance. Per-photo gray-world WB can remove intended/material colour. Production selects shared roll WB and zero initial Photo WB, leaving local/creative WB manual; these fixed-exposure counterfactuals do not establish photographic acceptance.

`image-contacts raster OUTPUT.ppm IMAGE` decodes a full image into colour-managed 8-bit sRGB PPM for private screenshot/result pixel comparisons. It converts the input profile rather than treating Display P3 bytes as sRGB. Raster statistics and nearest-centre contacts do not replace settled-preview or calibrated-monitor acceptance.

### Window capture

The maintained capture helper records normal/narrow Roll/Photo/Rendering layout holds. After building the editor, with the named local RAWs and an unlocked desktop:

```sh
mkdir -p .cache/probes/controls
/usr/bin/clang -fobjc-arc native/controls_window_id.m \
  -framework Foundation -framework CoreGraphics \
  -o .cache/probes/controls-window-id
python3 scripts/capture-controls.py
```

It captures only the application window. Logs/screenshots remain private under `.cache/probes/controls`; normal screen-capture permissions are required. Do not bypass occlusion or automate Lightroom/Resolve.

The shared viewer-bar tools and real bare-light support have separate layout routes, using the same window helper and existing deadlines/visible-frame gates:

```sh
python3 scripts/capture-viewer-tools.py --samples samples
python3 scripts/capture-viewer-tools.py --flat-only --samples samples \
  --output .cache/probes/flat-layout
```

The six-tool route holds crop/Dmin/WB at normal/narrow sizes with diagnostic regions; crop holds show Photo/hide scopes, picker holds show Rendering/scopes. Flat-only holds real reference support at normal/narrow sizes and cancels without accepting a map, saving or exporting. These are diagnostic setups, not automatic tab/scope changes. Partial captures or timeout are not route acceptance; keep photographic outputs private and record unperformed physical/device checks separately.

### Camera-roll review fixtures

The GPU-painted strip-only fixture shows landscape, portrait, wide crop, long filename, loading/error cells and selection/hover at 140-, 80- and 52-point strip heights:

```sh
cargo test --locked --offline --features editor --lib \
  capture_camera_roll_review_fixtures -- --ignored --nocapture
```

It writes private TIFFs under `.cache/probes/camera-roll`. Images are synthetic RGB16 data through the normal thumbnail renderer, not photographs or photographic acceptance. The fixture paints actual egui cards/captions; it does not establish native presentation, physical input or VoiceOver. Use the window-capture route separately for composed native layouts.

## Responsive image scopes

The above-controls slot provides positive-grade-only whole-frame approximate waveform/vectorscope, in selected output-native codes. RGB/luma reuse bins; vectorscope scale re-bins cached rendering without rerunning ACES. Source/reference views and crop drafts suspend scopes. Accepted crop/orientation follows canonical whole-frame mapping, independently of viewer navigation. [Contract](contracts/scopes.md).

```sh
cargo test --locked --offline --features editor --lib --test scopes --test viewer
# Bounded 1024×1024 flat/varied reduction probe; Metal, no RAWs/window:
cargo test --locked --offline --features editor --test scopes \
  scope_reduction_latency -- --ignored --nocapture
# Release editor built as above; scripted scope modes/amplification and layout holds:
ANTITONE_SMOKE_SCOPES=1 ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

The latency probe times encode/submit/wait for reductions only, not RAW/ACES processing or end-to-end gesture latency. GPU-bin/pixel readbacks are test-only; the live UI maps just 16 diagnostic bytes asynchronously with one outstanding job. Smoke retains nonzero-visible-frame acceptance and uses diagnostic calibration, not the user's grade. Capture only the application window during layout holds with the existing [helper](#window-capture); photographic screenshots remain ignored/private.

Worktrees share the normal rustup/Cargo download caches and keep separate `target/` products and native build directories by default. To reuse existing native builds, set `LIBRAW_SOURCE` and `OCIO_PREFIX` to their absolute paths; OpenMP must also be available at the fixed checkout-local path above, for example through an ignored shared `.cache/native-deps` link. `LIBRAW_SOURCE` does not relocate the OpenMP runtime. Use absolute RAW paths or read-only sample links for photographic checks; no per-worktree Rust bootstrap is needed.

## Rendering/import performance probes

The ignored editor probes use local Metal. `selected_proxy_switching_latency` exercises actual Editor switches with retained 256 RGB16 memory proxies and a miss. It checks pending-to-authoritative replacement and records admission, first pending/authoritative paint callback and backend Ready independently. Callbacks render offscreen; timing is CPU render-command recording, not GPU completion or OS/physical presentation. The probe warms renderer/layout/fonts, polls subsequent frames at 10 ms, includes real viewer GPU work, and does not control filesystem/page caching. Run:

```sh
cargo test --locked --offline --release --features editor --lib selected_proxy_switching_latency -- --ignored --nocapture
```

`visible_thumbnail_import_latency` measures actual Editor imports at 1100×800 with 4-/36-source rolls, a private empty disk-proxy cache, then reuse in a fresh Editor. The selected source seeds its normal proxy; other cold visible sources need full development. It records foreground preparation, first/all-visible developed-thumbnail UI cache readiness, first camera-preview readiness, all-visible navigability (camera or developed) and whether developed first readiness waited for settled refinement. Frames poll at 2 ms; callbacks are not painted, filesystem caching is uncontrolled, and cache readiness is not GPU completion, host wake latency or physical presentation. Sources are read-only `DSC04378` onward. Run:

```sh
cargo test --locked --offline --release --features editor --lib visible_thumbnail_import_latency -- --ignored --nocapture
```

The read-only embedded-preview diagnostic inspects LibRaw header descriptors and extracts each encoded preview without sensor unpack/development. It reports dimensions, format, JPEG SOI/byte counts and header/extraction time, not JPEG decode/upload or display/colour correctness. The diagnostic has no production preview path. Build against the same pinned native libraries:

```sh
mkdir -p .cache/probes
/usr/bin/clang++ -std=c++17 -O2 -Wall -Wextra -Werror -mmacosx-version-min=14.0 \
  -I.cache/native-deps/LibRaw-0.22.0-openmp native/raw_thumbnail_probe.cpp \
  .cache/native-deps/LibRaw-0.22.0-openmp/lib/libraw_r.a -lz \
  -L.cache/native-deps/openmp-22.1.8-install/lib -lomp \
  -Wl,-rpath,"$PWD/.cache/native-deps/openmp-22.1.8-install/lib" \
  -o .cache/probes/raw-thumbnail-probe
.cache/probes/raw-thumbnail-probe samples/DSC04378.ARW
```

Embedded JPEGs are camera-rendered/display-referred, not linear native-camera RGB16; they cannot feed model-4 inversion, correction, calibration, scopes or export. The [filmstrip camera-preview path](contracts/editor-controls.md#filmstrip-thumbnails) keeps that distinction. The explicit read-only bridge regression checks that extraction does not unpack the sensor or change metadata and compares sampled camera development with/without extraction:

```sh
cargo test --locked --offline --release --features editor --lib embedded_jpeg_does_not_unpack_or_change_camera_development -- --ignored --nocapture
```

This sampled check is not a new whole-image numerical gate. JPEG placeholder readiness does not include GPU upload/completion or physical presentation; true cold filesystem, actual gestures/pressure and broader cameras remain separate acceptance.

Runtime `selected_preview` logs report proxy dimensions, independent GPU pixel bytes and preparation time. `selected_switch` distinguishes admission/cache hit, backend message receipt and first paint callback for each request. Keep generated output in ignored cache storage; compare only runs with matching boundaries and workload.

The other probes cover import, refinement and thumbnail lifecycle: `import_switching_latency` measures backend readiness for 4-/36-source import/Open, switching/revisits and controlled cold/memory/disk/selected-seed thumbnails. Filesystem caching is uncontrolled; backend Ready is not physical presentation. `raw_byte_edits_reopen_without_identity_rejection_and_old_seed_keeps_its_revision` edits only a temporary RAW copy, checks saved state/exact RGB16 and verifies that a selected proxy is not retagged with newer file stats. Other probes leave samples unchanged. `import_and_refinement_latency` compares bounded queue depths 1/2/3 with and without simulated foreground GPU submissions and checks bitwise reconstruction agreement. This is not physical control/presentation latency. `initial_import_with_renderer_prewarm` separates header/unpack/process/resource phases, then verifies whole-frame scope processing/diagnostics remain coherent during seeded-thumbnail creation and disk restart. `project_relative_locator_cold_thumbnail` uses an ignored temporary symlink/project/cache to verify cold development against a resolved locator.

```sh
cargo test --locked --offline --release --features editor --lib import_switching_latency -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib raw_byte_edits_reopen_without_identity_rejection -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib import_and_refinement_latency -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib initial_import_with_renderer_prewarm -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib project_relative_locator_cold_thumbnail -- --ignored --nocapture
```

Combined geometry integration (corrected real RAW, shared affine refinement, tile seams, both targets and independent scalar coverage; no sample edits):

```sh
cargo test --locked --offline --features editor --lib --test scopes --test viewer --test affine_resample --test geometry_export --test camera_inversion --test gpu_film_response
cargo test --locked --offline --release --features editor --lib corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

Additional ignored native follow-up checks:

```sh
cargo test --locked --offline --release --features editor --lib development_and_region_picks -- --ignored --nocapture
# Build .cache/native-deps/probe per the linked guide before the contention probe:
cargo test --locked --offline --release --features editor --lib native_development_foreground_contention -- --ignored --nocapture
```

[Native complete-RGB/metadata/diagnostics comparison and build guide](native-development.md). Native thread stress excludes physical input/layout/surface latency, as with tile stress; scoped CPU policy and lazy decoding do not change source/development identity. `raw_development` logs include priority/requested thread count.

Runtime `source_registration`, `raw_import`, `raw_development`, `selected_source`, `thumbnail` and `settled_preview` logs identify phases. Thumbnail logs include memory/disk hits, cancellation and failure; source checks cover camera/development compatibility, not RAW byte identity. File stats are optional-cache freshness; generated RGB16 checksums and small locator-key hashes do not read/hash RAW contents. Settled `gpu_seconds` includes encoding/upload/submission/wait wall time, not hardware timestamps.

## RAW and complete GPU chain

Native-camera checks use read-only RAWs and diagnostic anchors:

```sh
cargo test --locked --offline --release --features raw,gpu \
  --test camera_raw -- --ignored --nocapture
```

The ignored real flat-field Save/Open/corrected-export check above exercises the same native-camera model with an accepted map. These headless checks do not establish actual onscreen or physical colour acceptance.


```sh
cargo run --locked --offline --features raw,gpu \
  --bin antitone-calibration-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  --gpu-fixture samples/DSC04416.ARW
./scripts/probe-ocio.sh
```

Calibration writes full-resolution camera sample grids, matrix/parameters and CPU AP1 reference fixtures under `.cache/probes`; OCIO adds them to synthetic cases before the GPU complete-chain comparison. Regenerate in that order after changing fixtures. Camera source is `camera-source-model4.rgb16` plus post-inversion column-major `camera-to-ap1-model4.f64`; there is no float-source fallback. These are regenerable RAW-derived fixtures, not hidden authored source or a full-image comparison.

## Rendering, gamut and ICC

After regenerating OCIO fixtures:

```sh
./scripts/probe-rendering.sh
cargo test --locked --offline --features render --test rendering
./scripts/probe-gamut.sh
# Independent current-target check, no RAW fixtures needed:
cargo run --locked --offline --features render \
  --bin antitone-rendering-probe -- --gamut-check
```

CPU renderer/scalar reference, exhaustive profile/transfer and independent ColorSync/Core Image checks address different boundaries. CPU/GPU agreement alone is insufficient for target colour preservation. Pinned production artifacts/checksums: [OCIO](../assets/ocio/README.md), [profiles](../assets/profiles/README.md).

## Export and cached sampling

```sh
cargo run --locked --offline --release --features raw,export \
  --bin antitone-export-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW .cache/probes/NEW_OUTPUT_DIRECTORY
cargo run --locked --offline --release --features raw \
  --bin antitone-scan-probe -- \
  samples/DSC04378.ARW 4000 2600 128 128
```

Export CLI uses explicit diagnostic calibration/default edits, full developed extent and no overwrite. Choose a **fresh** output directory; the UI instead exports the active metadata area. Exports can consume hundreds of MiB and significant time, so run only when relevant. Numerical TIFF/EXR reopen tests are cheaper checks for state/metadata changes. See [output limits/safety](contracts/output.md).

For an independent full TIFF/ImageIO reopening after a diagnostic export:

```sh
/usr/bin/clang++ -std=c++17 -O2 native/tiff_reference.mm \
  -framework Foundation -framework CoreGraphics -framework ImageIO \
  -o .cache/probes/tiff-reference
.cache/probes/tiff-reference .cache/probes/NEW_OUTPUT_DIRECTORY
```

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
# Leave the chart open:
cargo run --locked --offline --features ui \
  --bin antitone-presentation-probe -- --interactive
```

Core Image provides an encoding/reference comparison, not interactive rendering. The host explicitly tags extended-sRGB `RGBA16Float` SDR/EDR=false after configuration. The chart checks f32/half transfer/readback, ordinary UI and two resizes. **Zero submitted frames fails the onscreen check.** Layer/readback evidence does not establish physical-monitor matching, monitor transitions, UI blending, forced surface recovery, macOS-14 or other-device performance.

## Generated outputs

Keep generated fixtures, logs, plots and private photographic exports/screenshots under ignored `.cache/probes`, not in documentation. Dependency inputs/products belong in the setup-managed cache paths. Probe results require source/model, tested domain, tolerances and environment to be interpretable; a finite run is not universal numerical or hardware acceptance.
