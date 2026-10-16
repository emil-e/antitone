# Making and checking a change

Use [the README](../README.md) to build and run. If you are new to an area, read its [topic guide](README.md#understand-the-system), then follow the code links.

## Engineering standards

### Code and design

Follow the neighboring code. Separate declarations and logical phases with blank lines, name values by role outside conventional mathematics, and expand dense control flow and macros even when the formatter accepts them. Inspect complete touched functions; formatting success alone does not establish readability.

Preserve the owning guides' processing, output and project-compatibility contracts. Code and tests own exact defaults, formats and numerical gates. Distinguish established requirements from implementation choices and experimental hypotheses. Keep source storage compact; judge additional caching by measured benefit, memory cost and lifetime.

### Comments and documentation

Explain current contracts, rationale and non-obvious constraints, including ownership, lifecycle, numerical assumptions, platform quirks, compatibility and security. Public documentation should describe supported behavior at the reader's level rather than enumerate private helpers or call paths.

Remove obvious restatements, patch narration, completed TODOs and temporary investigation/review notes. Replace “previously X, now Y” with the reason Y matters, if still useful. Keep history where it serves the reader: decision rationale, migration guidance and changelogs. Negative statements earn their place when they prevent a plausible, consequential misunderstanding: “Edits are not persisted until Save” explains a boundary; “the old toolbar is no longer rendered” usually records only a completed change.

Write for prospective contributors: explain the problem, mental model and guarantees. Keep one explanation per topic in its owning guide, with links rather than parallel implementation inventories. Keep build/run prerequisites in the top-level README and contributor workflow in this guide. Update affected guides and links when behavior changes; update the roadmap when priorities, status or blockers change.

Plans describe unfinished scope, acceptance and next actions, not execution history. Transfer useful knowledge into the owning guide before removing completed steps or finished plans. Delete obsolete tools and superseded material rather than creating archives. Rewrite awkward prose when it contains useful knowledge instead of deleting that knowledge with it.

### Meaningful tests

For each test, ask: **What plausible defect would this catch, and why would that defect matter?** Derive expectations from requirements, justified invariants or independent references. Agreement between two paths can establish consistency without establishing that either is correct.

Protect the relevant contract while allowing behavior-preserving changes. Exact wording, snapshots, private calls, ordering, representations and operation counts need justification; accessibility, serialization, resource lifetime and performance can make them essential. Structural guarantees and source inspection can also deserve architectural tests. Distinguish those from facts guaranteed only by the test's own setup.

Watch for self-comparisons, expected results that mirror implementation calculations, checks for permanently removed UI text, mocks that merely confirm their arrangement, duplicate tests without distinct failure coverage and broad snapshots that obscure the meaningful expectation. These are review signals, not automatic deletion rules.

Test names and assertions should describe the scenario and invariant actually exercised. Preserve unusual fixture rationale and non-obvious preconditions beside the relevant setup.

Establish why a new expected value or tolerance is correct before updating it; a green test or newly accepted snapshot is not that justification. Do not weaken checks or alter development behavior to improve an experiment's appearance.

Before deleting a test, identify the protection being removed and whether it is still needed or covered elsewhere. Inspect cleanup diffs for lost meaning; documentation and comments can also contain executable examples or affect generated output.

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

Rust uses the checked-in toolchain. Native formatting requires **clang-format 22.1.8** on `PATH`; Swift uses `swift-format` from the selected Xcode toolchain with `.swift-format`. Install the other formatters in ignored checkout-local storage:

```sh
python3 -m venv .cache/format-tools/venv
.cache/format-tools/venv/bin/python -m pip install 'ruff==0.17.1'
cargo install --git https://github.com/wgsl-analyzer/wgsl-analyzer \
  --rev 58a9aba79a0e0f9d3fef12a5ef0e577b3c4a2ae4 --locked \
  --root .cache/format-tools wgslfmt

python3 scripts/check-format.py
python3 scripts/check-format.py --write
```

`RUFF`, `WGSLFMT` and `CLANG_FORMAT` can point to existing compatible executables. Ruff's version and conservative syntax/undefined-name/unused-code checks live in `ruff.toml`. The WGSL formatter is pinned by revision because its package version does not distinguish development snapshots. Upgrade tools deliberately and inspect their output rather than mixing versions in one cleanup.

## Feature-specific checks

Production and reference dependencies are separate:

| Feature | Boundary |
| --- | --- |
| no default features | Numerical core; no native, GPU or UI dependency |
| `raw` | CPU RAW development and measurements; LibRaw/OpenMP |
| `gpu` | Precise photographic GPU processing; no OCIO or desktop |
| `render` | Output targets, transfer and profiles; no OCIO |
| `export` | Encoders, metadata and atomic file safety; no OCIO |
| `raw,gpu,export` | Bounded original-sample GPU export; no desktop or OCIO |
| `editor` | Production application, including GPU export; no OCIO |
| `ocio-reference` | Independent OCIO renderer and scalar composed export oracles |
| `ui-tests` | Editor interaction checks, including independent OCIO assertions |

Tests that explicitly use the CPU/OCIO oracle require `ocio-reference`; this is not a production fallback. `--all-features` includes those references. Verify the production configuration separately so reference-feature unification cannot hide a runtime dependency.

Choose checks for the boundary you changed rather than running every native/sample-dependent exercise:

```sh
# Pixel processing and photographic grading; no native libraries or Metal.
cargo test --locked --no-default-features \
  --test camera_inversion --test processing --test film_response --test photographic

# Persistence; no native libraries or Metal.
cargo test --locked --features project \
  --test camera_project --test project --test film_project --test flat_field_project \
  --test dust_project

# Authored DNG plumbing; native libraries, no private photographs or desktop.
cargo test --locked --features raw,export,project,ocio-reference --test raw_dng --test raw_flat_field --test raw_roll

# Authored import/correction/Analyze/JPEG/cache workers; native setup and Metal, no desktop window.
cargo test --locked --features editor --lib authored_ -- --test-threads=1

# Production GPU math; requires Metal, not native RAW/rendering libraries.
cargo test --locked --features gpu --test gpu_processing --test gpu_film_response

# Composed editor, scopes and viewer; native setup and Metal required.
cargo test --locked --features editor,ocio-reference --lib --test scopes --test viewer
```

The [authored RAW fixtures](../tests/fixtures/raw/README.md) supply declared sensor data for API, worker and UI checks. Tests read the committed files directly and write exports/projects to temporary directories. The fixture guide explains their construction and maintenance; private photographs are optional diagnostic inputs, not ordinary test prerequisites.

The photographic tests use an independent scalar oracle in [test support](../tests/support/photographic_reference.rs). A missing Metal device is a verification limit, not a numerical pass.

After native setup, use broader integration/lints when the change warrants them:

```sh
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

## Export checks

```sh
# Production worker, immutable snapshots/operation identity, encoders and destination safety; no OCIO.
cargo test --locked --features editor --lib export -- --test-threads=1
# Independent original-sample processing/reconstruction and reopened files, including correction/locals.
cargo test --locked --features raw,gpu,export,project,ocio-reference \
  --lib gpu_export:: -- --test-threads=1
cargo test --locked --features raw,gpu,export,project,ocio-reference \
  --test gpu_export --test export --test geometry_export --test local_adjustment_export
cargo build --locked --release --features editor --bin antitone
```

Distinguish numerical correspondence from exact serialization: GPU/scalar processing comparisons allow justified rounding differences, while encoder tests require exact supplied-sample serialization. Check reconstruction and reopened files as well as intermediate arithmetic. The [output guide](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns processing and resource boundaries; per-job bounds are not an aggregate memory-pressure guarantee.

The opt-in [ImageIO export check](diagnostics.md#export) independently reopens TIFF storage/profiles using a native checker compiled into temporary storage. It needs no private files or prebuilt helper.

## Roll analysis

[Analysis](roll-analysis.md) spans sampling, numerical estimation and editor installation. Check the layer you changed:

```sh
cargo test --locked --no-default-features --test roll_analysis --test roll_sampling
cargo test --locked --lib roll_analysis::
cargo test --locked --features editor --lib editor::analysis::tests -- --test-threads=1
cargo test --locked --features editor --lib editor::analysis_review:: -- --test-threads=1
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

Check both proposal geometry and the composed Apply/Cancel workflow described in [the editor guide](editor.md#auto-crop-is-an-explicit-accepted-edit). `ANTITONE_UI_CAPTURE` enables `auto-crop-` inspection images. Software checks do not establish photographic border accuracy or physical interaction; [remaining acceptance](plans/region-detection.md) covers those boundaries.

## Shared GPU queue checks

```sh
cargo test --locked --features gpu --lib gpu::submission
cargo test --locked --features editor --lib refinement_worker_uses_the_submission_gate_supplied_to_the_host
```

These headless Metal checks exercise submission exclusion and the host-supplied gate. Changes to initialization also need `gpu::response::tests`. Use the [native presentation stress route](diagnostics.md#macos-presentation) for real surface reconfiguration alongside background GPU work.

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

Processing admission, source-owned transactions and downstream consumers need separate checks. Exercise correction, ordered local grades and source/target changes where relevant; one supported fixture does not establish admission for every response or control combination. Keep numerical gates intact and use [performance/RAW diagnostics](diagnostics.md#local-processing-and-reuse) for opt-in resource observations.

## Dust persistence

```sh
cargo test --locked --features project --test dust_project
cargo test --locked --features ui-tests --lib dust -- --test-threads=1
cargo test --locked --features ui-tests --lib editor::document::tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests::dust -- --test-threads=1
```

Test [saved repair intent](projects.md#source-owned-dust-repairs) separately from evaluation. Project checks need malformed-contract/record rejection, destination safety and lossless masks/IDs at uneven roll scale without a document-size cap. Controller and composed checks need accepted-only dirty state, pending/failed evaluation saves, source removal/reimport and failure-safe Save/Open/Save As. Authored RAW reopening must reproduce donor metadata and repaired f32 bits across supported CPU/GPU routes, with viewer/reference/scopes sharing the accepted repair content. Inspect the output-omission warning at normal/narrow sizes; optional captures use `dust-persistence-{normal,narrow}-offscreen.png`. These checks do not establish photographic or physical-input acceptance.

## Shared editor UI checks

Read [shared presentation](editor.md#build-ui-from-shared-presentation) before adding a widget. Check the component **and** its composed consumer: paint alone does not verify state or input ownership.

App-level fixtures must run production startup before injecting events and bring controls into view before clicking; painted text bounds alone do not establish an interactive target. Resource-policy tests supply explicit budgets/pressure rather than depending on the host's current pressure.

```sh
cargo test --locked --features editor --lib editor::presentation
cargo test --locked --features editor --lib editor::activity::tests
cargo test --locked --features editor --lib editor::controls::tests
cargo test --locked --features editor --lib editor::tests::viewer
cargo test --locked --features editor --lib editor::tests::workspace
cargo test --locked --features editor --lib editor::thumbnails
cargo clippy --locked --features editor --lib --tests -- -D warnings
```

For feature transactions, filter the owning module: for example `editor::analysis::tests`, `editor::flat::tests`, `editor::files::tests`, `editor::navigation::tests`, `editor::preferences::tests`, `editor::feedback::tests` or `editor::scopes::tests`. Use [layout/GPU diagnostics](diagnostics.md#editor-and-layout-checks) when painting changes. For wordmark/icon changes, also [regenerate the branding assets](../assets/branding/README.md#regenerate) and inspect the generated artwork and local bundle.

Physical pointer/trackpad behavior, VoiceOver and preference relaunch need real-use checks. [Interaction follow-up](plans/inspection-and-interaction.md) tracks that acceptance, including wheel semantics.

### Framework UI regressions

The optional `ui-tests` feature pins egui_kittest to the application's egui version. It exercises the production UI offscreen, with authored RAWs and no desktop window. Native setup and a **Metal device** are required; missing Metal fails rather than silently skipping. Run both profiles:

```sh
cargo test --locked --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo test --locked --release --features ui-tests --lib editor::kittest_tests -- --test-threads=1
cargo clippy --locked --features ui-tests --lib --tests -- -D warnings
```

The [harness](../src/editor_kittest.rs) uses the Editor's device, queue and submission gate to paint production callbacks into RGBA16Float. CPU source-rendering agreement checks are not an independent ACES oracle; rendering changes also need the relevant [OCIO/Apple references](diagnostics.md#rendering-gamut-and-icc). Worker/readback waits are bounded; an unfinished operation must fail at its deadline.

Personal preferences are not restored. Most scenarios disable thumbnail scheduling; project/correction scenarios run the production thumbnail worker with disk storage disabled. Known Save destinations and chooser results are fixture setup; menu actions, guards and workers remain production code. Native file dialogs are not automated.

Use Cargo's test filter or `-- --list` to select cases. For optional inspection artifacts:

```sh
ANTITONE_UI_CAPTURE="$PWD/.cache/ui-captures" \
  cargo test --locked --features ui-tests --lib dmin_picker_cancels_or_applies \
  -- --test-threads=1
```

Tests own capture names and available scenarios. Captures are clipped/quantized from float output for inspection, not approval of appearance or proof of native surface/ColorSync presentation. Small fixture viewports exercise native-pixel regions without claiming native-shell layout acceptance. Working-feedback galleries supply presentation states and counts; they do not measure backend duration or cancellation. Use [native-host checks](#explicit-native-host-checks) and [physical/photographic acceptance](plans/inspection-and-interaction.md) for their respective boundaries.

### Explicit native-host checks

The [native-host executable](../tests/native_host.rs) uses the production host. Lifecycle cases need macOS/Metal and an available desktop, but no RAWs/native RAW libraries. The `editor` case additionally needs native setup and the patch DNG; it opens a temporary authored project with isolated HOME/cache storage. Ordinary invocation runs software result-contract checks without opening a window, including under `--all-features`. Only explicit `--run` launches it:

```sh
cargo test --locked --features ui --test native_host -- --run lifecycle
cargo test --locked --release --features ui --test native_host -- --run lifecycle
# Current-Space full-display entry/paint/return/paint; requires an unlocked desktop.
cargo test --locked --features ui --test native_host -- --run fullscreen
# Actual Editor composition and selected-image surface passes; authored inputs only.
cargo test --locked --features editor --test native_host -- --run editor
cargo test --locked --release --features editor --test native_host -- --run editor
# Negative cases: all must exit nonzero with the corresponding failure.
cargo test --locked --features ui --test native_host -- --run early-close
cargo test --locked --features ui --test native_host -- --run timeout
cargo test --locked --features ui --test native_host -- --run process-timeout
```

Lifecycle success requires actual surface paints and explicit completion; premature closure and UI/process watchdog expiry fail. Hardware errors are failures after a requested launch. The authored Editor case additionally requires selected-image callbacks and paint acknowledgments at each requested size, establishing basic Editor/host composition rather than per-tool or physical-input acceptance.

Fullscreen checks current-Space borderless display coverage, system menu/Dock access, actual painting and restoration of window bounds/presentation options. This is distinct from native fullscreen Spaces and requires an unlocked desktop. Do not substitute kittest viewport metadata for that round trip. [Presentation references/stress checks](diagnostics.md#macos-presentation) and [remaining native lifecycle/input/display acceptance](plans/inspection-and-interaction.md#layout-and-controls) address separate boundaries.

## Specialist diagnostics

Tests marked ignored are opt-in because they need samples, hardware, visible windows or substantial resources. Choose them for the boundary being investigated; [diagnostics](diagnostics.md) explains safe workloads and interpretation.

Keep local photographs and generated images/logs in ignored storage. Automated agreement establishes a tested software boundary, not photographic quality, physical monitor matching or broad device support. [The roadmap](../PLAN.md) tracks remaining acceptance.
