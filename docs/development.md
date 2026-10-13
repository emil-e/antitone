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

## Specialist verification

[Build and probes](build-and-probes.md) covers numerical/reference probes, ignored RAW tests, export reopening and scripted presentation checks. These are optional specialist checks, not prerequisites for an ordinary build.

Automated agreement does not establish photographic appearance, physical gestures, monitor matching, actual macOS-14 execution or broader-device acceptance. Current behavior/tolerances are defined in [the contracts](README.md#contracts); [the roadmap](../PLAN.md) tracks remaining acceptance.
