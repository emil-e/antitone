# Making and checking a change

Use [the README](../README.md) to build and run. If you are new to an area, read its [topic guide](README.md#understand-the-system), then follow the code links.

## Engineering standards

### Code and design

Follow the neighboring code. Separate declarations and logical phases with blank lines, name values by role outside conventional mathematics, and expand dense control flow even when the formatter accepts it.

Understand the current processing, output and project-compatibility behavior before changing it. Code, tests and topic guides describe present choices; their existence does not make each choice a product requirement. Explain a design change in terms of its benefits, drawbacks and affected callers or saved data, and judge storage and caching choices by measured benefit, memory cost and lifetime.

### Comments and documentation

Explain current behavior, rationale and non-obvious constraints — ownership, lifecycle, numerical assumptions, platform quirks, compatibility. Distinguish an implementation dependency from product intent, and give requirements their source rather than inferring them from code or tests. Public documentation describes supported behavior at the reader's level, not private call paths.

Remove restatements, patch narration and completed investigation notes; replace "previously X, now Y" with the reason Y matters. A negative statement earns its place only if you can name what a reader would otherwise *do* wrong — "Edits are not persisted until Save" prevents lost work; a sentence like "the old toolbar is no longer rendered" records only a completed change, and "this check does not establish X" defends a claim no reader acted on. If no wrong action exists, cut the sentence.

Run `python3 scripts/check-docs.py` after documentation changes. It fails on broken links and reports word growth per changed guide, long paragraphs and hedge density — findings to judge, not verdicts.

Write for prospective contributors: the problem, the mental model, the guarantees. Keep one explanation per topic in its owning guide, with links rather than parallel inventories. Plans describe unfinished scope and next actions, not execution history; transfer useful knowledge into the owning guide before removing finished plans.

### Tooling retention

Keep tooling that has current use: the build chain, regenerators with a documented recipe for committed artifacts, diagnostics listed in this guide, and sources compiled into the crate. Retire a study's scaffolding once its outcome lands — conclusions belong in code, tests and the owning topic guide, and the derivation stays recoverable in git history. Before handing off an investigation, ask which of its scripts a fresh contributor would still run today; anything without a live consumer or a documented workflow is history, not product.

### Meaningful tests

For each test, ask: **What plausible defect would this catch, and why would that defect matter?** Derive expectations from requirements, justified invariants or independent references — agreement between two paths can establish consistency without establishing that either is correct.

Apply this scrutiny at both whole-case and individual-assertion level, including fixture setup and control discovery. For UX regressions, distinguish user-visible protection from locking down the current arrangement: fixed dimensions, wording, colours or widget placement need a reason beyond matching today's UI, and finding a control through its painted colour can impose the same lock indirectly.

Watch for self-comparisons, expectations that mirror implementation calculations, mocks that confirm their own arrangement, and broad snapshots that obscure the meaningful expectation — review signals, not automatic deletion rules. Test names and assertions should describe the scenario and invariant actually exercised. Establish why a new expected value or tolerance is correct before updating it; a green test is not that justification. Before deleting a test, identify the protection being removed and whether it is still needed or covered elsewhere.

## Everyday checks

The default build is the numerical core, without native libraries or the desktop stack:

```sh
cargo build --locked
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

Cargo uses the committed lockfile. Add `--offline` when caches are populated and downloads are undesirable; omit it for a first build.

## Source formatting

Run `python3 scripts/check-format.py` before handing off a change. It checks tracked and new authored Rust, native, Python, WGSL and Swift sources, plus shell syntax; ignored caches, samples and generated OCIO GLSL are excluded. `--write` applies the configured formatters without changing lint rules. Neither mode builds the editor or needs native dependencies.

Rust uses the checked-in toolchain. Native formatting requires **clang-format 22.1.8** on `PATH`; Swift uses `swift-format` from the selected Xcode toolchain. Install the other formatters in ignored checkout-local storage:

```sh
python3 -m venv .cache/format-tools/venv
.cache/format-tools/venv/bin/python -m pip install 'ruff==0.17.1'
cargo install --git https://github.com/wgsl-analyzer/wgsl-analyzer \
  --rev 58a9aba79a0e0f9d3fef12a5ef0e577b3c4a2ae4 --locked \
  --root .cache/format-tools wgslfmt

python3 scripts/check-format.py
python3 scripts/check-format.py --write
```

`RUFF`, `WGSLFMT` and `CLANG_FORMAT` can point to existing compatible executables. Ruff's version and conservative syntax/undefined-name/unused-code checks live in `ruff.toml`. The WGSL formatter is pinned by revision because its package version does not distinguish development snapshots.

## Feature-specific checks

Production and reference dependencies are separate:

| Feature | Boundary |
| --- | --- |
| no default features | Numerical core; no native, GPU or UI dependency |
| `raw` | CPU RAW development and measurements; LibRaw/OpenMP |
| `dng-sdk` | Direct-SDK adapter and authored tests; combined with `raw` routes DNG shared sources; explicitly supplied Adobe SDK |
| `gpu` | Precise photographic GPU processing; no OCIO or desktop |
| `render` | Output targets, transfer and profiles; no OCIO |
| `export` | Encoders, metadata and atomic file safety; no OCIO |
| `raw,gpu,export` | Bounded original-sample GPU export; no desktop or OCIO |
| `editor` | Application, including direct DNG SDK development and GPU export; no OCIO |
| `ocio-reference` | Independent OCIO renderer and scalar composed export oracles |
| `ui-tests` | Editor interaction checks, including independent OCIO assertions |

Tests that explicitly use the CPU/OCIO oracle require `ocio-reference`; this is not a production fallback. Editor and `--all-features` builds need the [explicit SDK setup](../README.md#direct-dng-sdk-setup). Verify the production configuration separately so reference-feature unification cannot hide a runtime dependency.

Choose checks for the boundary you changed rather than running every native/sample-dependent exercise:

```sh
# Pixel processing and photographic grading; no native libraries or Metal.
cargo test --locked --no-default-features \
  --test camera_inversion --test processing --test film_response --test photographic

# Persistence; no native libraries or Metal.
cargo test --locked --features project \
  --test camera_project --test project --test film_project --test flat_field_project \
  --test dust_project

# Independent direct-SDK adapter; explicit SDK setup, no LibRaw/Metal/private photographs.
cargo test --locked --features dng-sdk --lib dng::
# Bounded native worker ownership, failure joins and tile-scratch policy; no SDK needed.
mkdir -p .cache/checks
/usr/bin/clang++ -std=c++17 -O3 -Wall -Wextra -Werror \
  native/dng_execution_tests.cpp -o .cache/checks/dng-execution
.cache/checks/dng-execution
# Shared source, original precision/capture, saved SDK identity and flat-reference matching.
cargo test --locked --features raw,dng-sdk,project --test dng_integration
# Cold Save/Open and reopened TIFF/AP0 EXR against analytic camera/scalar expectations.
cargo test --locked --features editor,ocio-reference --test dng_integration

# Authored DNG plumbing; native libraries, no private photographs or desktop.
cargo test --locked --features raw,export,project,ocio-reference --test raw_dng --test raw_flat_field --test raw_roll

# Document-owned look selection, cancellation acknowledgement and physical bake/cache retirement.
cargo test --locked --features editor --lib editor::look_workflow:: -- --test-threads=1

# Reference-session and lazy-support completion, retirement and correction transactions without painting.
cargo test --locked --features editor --lib -- \
  editor::reference_workflow:: editor::support_workflow:: \
  --test-threads=1

# Effective grade ownership, physical support and purpose-bound reconstruction without painting.
cargo test --locked --features editor --lib -- \
  editor::grade_scope:: editor::original_workflow:: editor::working_view::workflow_tests:: \
  --test-threads=1

# Candidate preparation/promotion, history and physical retirement without painting.
cargo test --locked --features editor --lib editor::document_workflow:: -- --test-threads=1

# Captured writes, baseline/history/visit preservation and guarded continuation retirement.
cargo test --locked --features editor --lib editor::save:: -- --test-threads=1

# Visit-owned source workflow and native physical admission, including pending candidates.
cargo test --locked --features editor --lib editor::source_workflow:: -- --test-threads=1

# Authored import/correction/Analyze/JPEG/cache workers; native setup and Metal, no desktop window.
cargo test --locked --features editor --lib authored_ -- --test-threads=1

# Production GPU math; requires Metal, not native RAW/rendering libraries.
cargo test --locked --features gpu --test gpu_processing --test gpu_film_response

# Composed editor, scopes and viewer; native setup and Metal required.
cargo test --locked --features editor,ocio-reference --lib --test scopes --test viewer
```

The [authored RAW fixtures](../tests/fixtures/raw/README.md) supply declared sensor data for API, worker and UI checks. The photographic tests use an independent scalar oracle in [test support](../tests/support/photographic_reference.rs); a missing Metal device is a verification limit, not a numerical pass.

After native setup, use broader integration/lints when the change warrants them:

```sh
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

## Export checks

```sh
# Production services/workflows, immutable snapshots/operation identity, encoders and destination safety; no OCIO.
cargo test --locked --features editor --lib export -- --test-threads=1
# Independent original-sample processing/reconstruction and reopened files, including correction/locals.
cargo test --locked --features raw,gpu,export,project,ocio-reference \
  --lib gpu_export:: -- --test-threads=1
cargo test --locked --features raw,gpu,export,project,ocio-reference \
  --test gpu_export --test export --test geometry_export --test local_adjustment_export
cargo build --locked --release --features editor --bin antitone
```

Check captured service output separately from UI decisions/activity, and distinguish numerical correspondence from exact serialization: GPU/scalar comparisons allow justified rounding differences, while encoder tests require exact supplied-sample serialization. Check reconstruction and reopened files as well as intermediate arithmetic. The [output guide](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns processing and resource boundaries. For batches, check source-specific pixels/grades, cold acquisition after cache closure, per-photo failure continuation, prior-output protection and publication races. Composed checks cover scope/target isolation, numeric commits, modal cancellation and reachable narrow-layout actions:

```sh
ANTITONE_UI_CAPTURE="$PWD/.cache/ui-captures" \
  cargo test --locked --features ui-tests --lib -- \
  editor::kittest_tests::export:: editor::kittest_tests::progress:: --test-threads=1
```

The opt-in [ImageIO export check](diagnostics.md#export) independently reopens TIFF storage/profiles; it needs no private files.

## Cancellation checks

Exercise the [shared cancellation pattern](architecture.md#cooperative-cancellation) before the first poll, with parked waiters, queued admission and admitted work. Observe actual wake/submission boundaries, retain cancelling feedback until acknowledgement, and verify physical resources survive owner retirement through cleanup. A stop after publication must preserve success; export can finish its current photo before stopping.

```sh
cargo test --locked --features editor --lib -- \
  cancellation:: editor::analysis_workflow::acknowledgement_tests:: \
  editor::export_workflow:: editor::export_batch:: --test-threads=1
```

## Roll analysis

[Analysis](roll-analysis.md) spans sampling, numerical estimation and editor installation. Check the layer you changed:

```sh
cargo test --locked --no-default-features --test roll_analysis --test roll_sampling
cargo test --locked --lib roll_analysis::
cargo test --locked --features editor --lib editor::analysis:: -- --test-threads=1
cargo test --locked --features editor --lib editor::analysis_review:: -- --test-threads=1
cargo test --locked --features editor --lib editor::analysis_workflow:: -- --test-threads=1
```

Core checks use synthetic populations. Editor checks exercise preparation and transactional installation; changes to inference scratch must preserve exact outputs and source freshness across reuse and reacquisition.

For the actual sampling-area review and Analyze controls with authored RAWs, use the [framework prerequisites](#framework-ui-regressions) and run:

```sh
cargo test --locked --features ui-tests --lib editor::kittest_tests::analyze -- --test-threads=1
```

Optional captures use `analyze-` filenames. [Real-RAW diagnostics](diagnostics.md#analyze-roll) evaluate supplied photographs separately.

## Auto-crop

```sh
cargo test --locked --features editor --lib auto_crop -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::auto_crop -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests::auto_crop -- --test-threads=1
```

Check both proposal geometry and the composed Apply/Cancel workflow described in [the editor guide](editor.md#auto-crop-is-an-explicit-accepted-edit). Batch proposals must survive browsing without retargeting. `ANTITONE_UI_CAPTURE` enables `auto-crop-` inspection images.

## Creative-look checks

The [catalog guide](../assets/looks/README.md) owns photo-free authoring and pinned-reference regeneration. Application checks distinguish strict asset/storage admission, source-model agreement, fixed-reference numerical gates and composed consumer behavior:

```sh
cargo test --locked --features project --lib looks::
cargo test --locked --features project --test look_project
cargo test --locked --features project --lib project::json
cargo test --locked --release --features gpu,render,project --test gpu_look_sampling --test gpu_looks
# Add the independent OCIO CPU all-node forward-reference oracle and original TIFF/EXR checks.
cargo test --locked --release --features editor,ocio-reference --test gpu_looks --test gpu_look_export
cargo test --locked --features editor --lib editor::look_ui::tests -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::looks -- --test-threads=1
cargo test --locked --features ui-tests --lib viewer::look_tests
```

Verify rendered pixels and scope contents against independent expectations, not only agreement between production signatures. Nonlinear interpolation fixtures must distinguish tetrahedral from trilinear sampling. Lifecycle checks preserve accepted selection through failure, cancellation and stale replies, and reopen embedded assets without their original files; host-idle completion matters because continuously painting the UI can conceal stranded map callbacks. `ANTITONE_UI_CAPTURE` optionally writes `look*-offscreen.png` captures.

## Shared GPU queue checks

```sh
cargo test --locked --features gpu --lib gpu::submission
cargo test --locked --features editor --lib refinement_worker_uses_the_submission_gate_supplied_to_the_host
```

These headless Metal checks exercise submission exclusion and the host-supplied gate. Use the [native presentation stress route](diagnostics.md#macos-presentation) for real surface reconfiguration alongside background GPU work.

## Local grading and shared consumers

Use the [processing model](processing.md#local-grades-change-controls-not-rendered-pixels) and [editor workflow](editor.md#immutable-local-grade-routing) to identify the boundary being changed:

```sh
cargo test --locked --no-default-features --test local_adjustments
cargo test --locked --features project --test local_adjustment_project
cargo test --locked --features export,project,ocio-reference --test local_adjustment_export
cargo test --locked --features gpu --lib gpu::
cargo test --locked --features gpu --test gpu_local_adjustments
cargo test --locked --features editor,ocio-reference --test local_adjustment_consumers --test viewer --test scopes
cargo test --locked --features editor --lib editor::processing::tests -- --test-threads=1
cargo test --locked --features editor --lib editor::adjustments::tests -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::adjustments -- --test-threads=1
cargo test --locked --features editor --lib editor::mask_editor:: -- --test-threads=1
cargo test --locked --features editor --lib editor::document::tests::admission -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::masks -- --test-threads=1
```

Processing admission, source-owned transactions and downstream consumers need separate checks; one supported fixture does not establish admission for every response or control combination. Use [performance/RAW diagnostics](diagnostics.md#local-processing-and-reuse) for opt-in resource observations.

## Undo/redo

```sh
cargo test --locked --features editor --lib editor::history:: -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::history:: -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests::history:: -- --test-threads=1
```

Exercise accepted-state hooks separately from snapshot storage: zero idle history snapshots on repair-bearing projects, gesture grouping, drafts/text ownership, exact values outside slider spans, redo branching, failed restoration and stale worker replies.

## Dust persistence

```sh
cargo test --locked --features project --test dust_project --test image_recipe
cargo test --locked --features editor --lib repair_tests -- --test-threads=1
cargo test --locked --features editor --lib recipe_tests -- --test-threads=1
# Proper thumbnail pixels against scalar filtering/grading and independent OCIO output.
cargo test --locked --features editor,ocio-reference \
  --lib editor::thumbnails::recipe_tests::pixel_tests -- --test-threads=1
cargo test --locked --features ui-tests --lib dust -- --test-threads=1
# Captured evaluation, physical retirement, source/grade lifetime and shared GPU cache.
cargo test --locked --features editor --lib editor::dust_workflow:: -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::document::tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests::dust -- --test-threads=1
```

Test [saved repair intent](projects.md#source-owned-dust-repairs) separately from evaluation. Project checks need malformed-record rejection, destination safety and lossless masks/IDs at uneven roll scale. Composed checks need accepted-only dirty state, failure-safe Save/Open/Save As, reopened donor metadata and repaired f32 bits, and proper-thumbnail pixels with repairs composed before filtering. A nonconstant look must not hide missing repairs, and disabling repairs must restore unrepaired pixels. Optional captures use `dust-persistence-{normal,narrow}-offscreen.png`.

## Shared editor UI checks

Read [shared presentation](editor.md#build-ui-from-shared-presentation) before adding a widget. Check the component **and** its composed consumer: paint alone does not verify state or input ownership.

App-level fixtures must run production startup before injecting events and bring controls into view before clicking. Workflow commands need a live application context: admitted commands register their owned task immediately, with the factory deferred until the mutable UI borrow ends; rendering-only fixtures reject execution instead of emulating a queued operation. Keep immutable-input/service checks separate from live command checks.

```sh
cargo test --locked --features editor --lib editor::presentation
cargo test --locked --features editor --lib editor::activity::tests
cargo test --locked --features editor --lib editor::controls::tests
cargo test --locked --features editor --lib editor::tests::viewer
cargo test --locked --features editor --lib editor::tests::workspace
cargo test --locked --features editor --lib editor::thumbnails
cargo clippy --locked --features editor --lib --tests -- -D warnings
```

For scoped execution, admission, source demand and measurement:

```sh
for module in application:: blocking_jobs:: editor::development:: editor::measurement:: editor::photo_scope::; do
  cargo test --locked --features editor --lib "$module" -- --test-threads=1 || break
done
```

The [live application fixture](../src/application_test_support.rs) exposes short state transactions, explicit workflow entry and bounded wake pumping without painting. For feature transactions, filter the owning module: for example `editor::analysis::tests`, `editor::flat::tests`, `editor::files::tests`, `editor::navigation::tests`, `editor::preferences::tests`, `editor::feedback::tests` or `editor::scopes::tests`. Use [layout/GPU diagnostics](diagnostics.md#editor-and-layout-checks) when painting changes. For wordmark/icon changes, also [regenerate the branding assets](../assets/branding/README.md#regenerate) and inspect the generated artwork.

Physical pointer/trackpad behavior, VoiceOver, preference relaunch and wheel semantics need real-use checks.

### Framework UI regressions

The optional `ui-tests` feature pins egui_kittest to the application's egui version. It exercises the production UI offscreen, with authored RAWs and no desktop window. Native setup and a **Metal device** are required; missing Metal fails rather than silently skipping. Run both profiles:

```sh
cargo test --locked --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo clippy --locked --features ui-tests --lib --tests -- -D warnings
```

The [harness](../src/editor_kittest.rs) uses the Editor's device, queue and submission gate to paint production callbacks into RGBA16Float. CPU source-rendering agreement checks are not an independent ACES oracle; rendering changes also need the relevant [OCIO/Apple references](diagnostics.md#rendering-gamut-and-icc). Worker/readback waits are bounded; an unfinished operation must fail at its deadline.

Known Save destinations and chooser results are fixture setup; menu actions, guards and workers remain production code. Native file dialogs are not automated. For optional inspection artifacts:

```sh
ANTITONE_UI_CAPTURE="$PWD/.cache/ui-captures" \
  cargo test --locked --features ui-tests --lib dmin_picker_cancels_or_applies \
  -- --test-threads=1
```

Captures are clipped/quantized from float output for inspection, not approval of appearance or proof of native surface presentation. Use [native-host checks](#explicit-native-host-checks) for the native-surface boundary.

### Explicit native-host checks

The [native-host executable](../tests/native_host.rs) uses the production host. Lifecycle cases need macOS/Metal and an available desktop, but no RAWs/native RAW libraries. The `editor` case additionally needs native setup and the patch DNG; it opens a temporary authored project with isolated HOME/cache storage. Ordinary invocation runs software result-contract checks without opening a window, including under `--all-features`. Only explicit `--run` launches it:

```sh
cargo test --locked --features ui --test native_host -- --run lifecycle
cargo test --locked --release --features ui --test native_host -- --run lifecycle
# Idle local continuation, nested native-loop deferral, retirement and late wake.
cargo test --locked --features ui --test native_executor -- --run
cargo test --locked --release --features ui --test native_executor -- --run
# Current-Space full-display entry/paint/return/paint; requires an unlocked desktop.
cargo test --locked --features ui --test native_host -- --run fullscreen
# Native menu installation/state/accelerators/event wakeup and file-panel Edit restoration.
cargo test --locked --features ui --test native_menus -- --run
# Isolated child apps: native termination negotiation and cleanup; no real logout/shutdown.
cargo test --locked --features ui --test native_termination -- --run
cargo test --locked --release --features ui --test native_termination -- --run
# Actual Editor composition, native menus and selected-image surface passes; authored inputs only.
cargo test --locked --features editor --test native_host -- --run editor
cargo test --locked --release --features editor --test native_host -- --run editor
# Negative cases: all must exit nonzero with the corresponding failure.
cargo test --locked --features ui --test native_host -- --run early-close
cargo test --locked --features ui --test native_host -- --run timeout
cargo test --locked --features ui --test native_host -- --run process-timeout
```

Lifecycle success requires actual surface paints and explicit completion; premature closure and watchdog expiry fail. The authored Editor case additionally requires selected-image callbacks and paint acknowledgments at each requested size.

The native-menu executable also requires explicit `--run`: it activates real AppKit menu items/accelerators, verifies dynamic Undo/Redo labels, and checks event-loop wakeup, minimized-window restoration and temporary native-responder Edit routing. Editor controller/composed checks separately cover unsaved-change protection, numeric commits and Settings:

```sh
cargo test --locked --features ui-tests --lib editor::menu::tests -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::kittest_tests::menus -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests::menus -- --test-threads=1
```

The native-termination executable launches children that send real AppKit termination requests and check deferred painting, repeated-request coalescing, rejection/retry and cleanup-before-exit; negative cases cover an interrupted host, a panic while a reply is pending and an incompatible pre-existing delegate method. No child initiates logout/shutdown. Editor controller tests separately exercise dirty/clean requests and blocking tools:

```sh
cargo test --locked --features ui-tests --lib editor::menu::quit_tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::menu::quit_tests -- --test-threads=1
cargo test --locked --features ui-tests --lib system_quit_preserves_leading_paste -- --test-threads=1
```

These checks require an unlocked desktop for focus and native paints; headless or locked-session timeouts are not passes. Fullscreen checks current-Space borderless display coverage and restoration of window bounds/presentation options — do not substitute kittest viewport metadata for that round trip. [Presentation references/stress checks](diagnostics.md#macos-presentation) address a separate boundary.

## Specialist diagnostics

Tests marked ignored are opt-in because they need samples, hardware, visible windows or substantial resources. Choose them for the boundary being investigated; [diagnostics](diagnostics.md) explains safe workloads and interpretation. Keep local photographs and generated images/logs in ignored storage.