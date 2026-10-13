# Making and checking a change

Use [the README](../README.md) to build and run. If you are new to an area, read its [topic guide](README.md#understand-the-system), then follow the code links. The guides explain the model; types, implementation comments and tests define its mechanics.

## Everyday checks

The default build is the numerical core, without native libraries or the desktop stack:

```sh
cargo build --locked
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

Cargo uses the committed lockfile. Add `--offline` when caches are populated and downloads are undesirable; omit it for a first build.

## Feature-specific checks

Choose checks for the boundary you changed rather than running every native/sample-dependent exercise.

```sh
# Pixel processing and photographic grading; no native libraries or Metal.
cargo test --locked --no-default-features \
  --test camera_inversion --test processing --test film_response --test photographic

# Persistence; no native libraries or Metal.
cargo test --locked --features project \
  --test camera_project --test project --test film_project --test flat_field_project

# Production GPU math; requires Metal, not native RAW/rendering libraries.
cargo test --locked --features gpu --test gpu_processing --test gpu_film_response

# Composed editor, scopes and viewer; native setup and Metal required.
cargo test --locked --features editor --lib --test scopes --test viewer
```

The photographic tests use an independent scalar oracle in [test support](../tests/support/photographic_reference.rs). Numerical gates live in the tests; do not loosen them merely to make a new implementation agree. A missing Metal device is a verification limit, not a numerical pass.

After native setup, use broader integration/lints when the change warrants them:

```sh
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

## Roll analysis

[Analysis](roll-analysis.md) spans sampling, numerical estimation and editor installation. Check the layer you changed:

```sh
cargo test --locked --no-default-features --test roll_analysis --test roll_sampling
cargo test --locked --lib roll_analysis::
cargo test --locked --features editor --lib editor::analysis::tests -- --test-threads=1
```

The first two exercise synthetic populations and curve fitting. Editor checks additionally cover snapshots, confirmation, cancellation and atomic full-grade reset. [Real-RAW diagnostics](diagnostics.md#analyze-roll) are separate, explicit exercises.

## Shared editor UI checks

Read [shared presentation](editor.md#build-ui-from-shared-presentation) before adding a widget. Check the component **and** its composed consumer: paint equivalence alone does not verify state or input ownership.

```sh
cargo test --locked --features editor --lib editor::presentation
cargo test --locked --features editor --lib editor::controls::tests
cargo test --locked --features editor --lib editor::tests::viewer
cargo test --locked --features editor --lib editor::tests::workspace
cargo test --locked --features editor --lib editor::thumbnails
cargo clippy --locked --features editor --lib --tests -- -D warnings
```

For feature-specific transactions, use the `editor::analysis::tests`, `editor::flat::tests`, `editor::files::tests`, `editor::navigation::tests`, `editor::preferences::tests` or `editor::scopes::tests` filter. Use [native layout and GPU fixtures](diagnostics.md#editor-and-layout-smoke) when painting/layout changes.

Known [selection/accessibility gaps](plans/inspection-and-interaction.md#focused-fixes) still need implementation work. Physical pointer/trackpad behavior, VoiceOver and preference relaunch also need real-use checks; headless events cannot establish them.

## Specialist diagnostics

Most ordinary tests need no private RAWs. Tests marked ignored are opt-in because they need samples, hardware, visible windows or substantial resources. [Diagnostics](diagnostics.md) explains how to run them safely and interpret their results.

Keep local photographs and generated images/logs in ignored storage. Automated agreement establishes a tested software boundary, not photographic quality, physical monitor matching or broad device support. [The roadmap](../PLAN.md) tracks remaining acceptance.
