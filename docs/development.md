# Development checks

Use [the README](../README.md) for prerequisites, native setup and editor build/run commands. [Architecture](architecture.md) describes the feature boundaries.

## Everyday checks

The default build is the numerical core, without native RAW/rendering libraries or the desktop stack:

```sh
cargo build --locked
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

Cargo uses the committed lockfile. Add `--offline` when the dependency cache is populated and downloads are undesirable; omit it for first builds.

## Feature-specific checks

Choose the feature and tests relevant to the change:

```sh
# Signed/HDR Photo grading and its independent f64 reference; no native libraries or Metal.
cargo test --locked --no-default-features --test photographic

# Project persistence without native libraries or Metal.
cargo test --locked --features project --test camera_project --test project --test film_project

# GPU response; requires Metal, but not native RAW/rendering libraries.
cargo test --locked --features gpu --test gpu_film_response

# Native dependencies plus Metal; editor/library and composed viewer/scope checks.
cargo test --locked --features editor --lib --test scopes --test viewer
```

After native setup, broader integration/lint checks are available when relevant:

```sh
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

`tests/support/photographic_reference.rs` supplies the independent endpoint/blend oracle and its scalar invariant tests to `tests/photographic.rs`. Run the latter with `--features project` to include Photo settings serialization checks.

These tests do not require private RAWs by default; sample/hardware acceptance checks marked ignored are exercised explicitly. A missing Metal device is a verification limit, not evidence that a numerical gate passed.

## Shared editor UI checks

Use the [shared visual-language contract](contracts/editor-controls.md#shared-visual-language) and [agent entry points](../AGENTS.md#ui-engineering) to choose the component and its composed consumers. Presentation tests run without document/worker fixtures; photographic adapters and editor tests retain domain/transaction coverage.

```sh
cargo test --locked --features editor --lib editor::presentation
cargo test --locked --features editor --lib editor::controls::tests
cargo test --locked --features editor --lib editor::files::tests
cargo test --locked --features editor --lib editor::tests::viewer
cargo test --locked --features editor --lib editor::tests::workspace
cargo test --locked --features editor --lib editor::thumbnails
cargo fmt --all -- --check
cargo clippy --locked --features editor --lib --tests -- -D warnings
```

The presentation suite checks direct-widget paint/geometry/ID equivalence, allocation-free width/sizing, focus/activation, inherited/local disable, section expansion and caption elision at normal/narrow widths and 1×/2×. Thumbnail checks include actual GPU scroll-edge clipping and camera-preview precedence. Use `editor::analysis::tests`, `editor::flat::tests`, `editor::navigation::tests`, `editor::preferences::tests` or `editor::scopes::tests` for changes involving those owners. Run the [native layout and GPU fixtures](build-and-probes.md#editor-and-layout-smoke) when painting/layout changes, not as a substitute for event/state checks.

Equivalence coverage preserves existing behavior, not complete accessibility. Known [selection/semantic gaps](plans/inspection-and-interaction.md#focused-fixes), physical input/VoiceOver, normal-app preference relaunch and broader-device/photographic acceptance remain separate. Real-RAW/resource probes stay opt-in; an ordinary focused run does not perform ignored acceptance.

## Specialist verification

[Build and probes](build-and-probes.md) covers numerical/reference probes, ignored RAW tests, export reopening and scripted presentation checks. These are optional specialist checks, not prerequisites for an ordinary build.

Automated agreement does not establish photographic appearance, physical gestures, monitor matching, actual macOS-14 execution or broader-device acceptance. Current behavior/tolerances are defined in [the contracts](README.md#contracts); [the roadmap](../PLAN.md) tracks remaining acceptance.
