# Local build and delivery

The [README](../README.md#end-to-end-local-delivery) owns prerequisites and commands. The first delivery workflow targets macOS on Apple Silicon and produces a local `.app` and ZIP. Windows/Linux packaging and GitHub release automation are not implemented yet.

## Build ownership

Cargo builds Rust and selects feature-specific native dependencies. `build.rs` compiles small bridges with `cc`, builds the vendored Adobe SDK through `native/dng/CMakeLists.txt` and publishes link settings to Cargo. CMake/Ninja owns that native build; it is not an outer build system that runs Cargo. Core-only Cargo commands still need no native libraries or desktop stack.

LibRaw/OpenMP and the independent OCIO test dependency use the maintained native setup scripts. Their archives are pinned and SHA-256 checked before extraction/building. The SDK is already source in the checkout, with provenance and per-file checksums; its native library is assembled from source libraries rather than an Adobe validator executable's object cache.

`cargo xtask` is a small host-side orchestration crate, independent of application/native dependencies. It sequences the existing setup/format/package tools and Cargo commands. The Python standard-library packager remains responsible for macOS bundle metadata, dependency loading, notices and archive verification. Neither layer implements an incremental build engine. `build.rs` never downloads SDK source, installs tools, packages or signs the app.

## Checks and environment

`setup` checks tools, runs native setup, installs pinned Ruff/clang-format into a local Python environment and installs the revision-pinned WGSL formatter locally through Cargo. Rustup selects the checked-in Rust toolchain. All generated tools/dependencies stay in ignored storage or Cargo's normal cache; no system installation is performed.

`check` runs authored-source formatting, packaging/format-tool tests, core tests, production editor tests/lints, all-feature reference tests/lints and release-profile composed UI tests. Production is checked separately so feature unification cannot conceal an OCIO runtime dependency. No private samples are needed. Ignored photographic/resource diagnostics and explicitly launched native-window exercises remain separate, as described in [development](development.md).

Metal tests require a Metal device. Missing hardware is a verification failure/limit, not a reason to silently skip tests. The automated pipeline does not establish photographic, physical-input, native-window or actual macOS-14 acceptance. The relocated package smoke check runs `--version`, not an editor interaction scenario.

Use `DEVELOPER_DIR` to select Xcode and the normal Cargo target-directory override if desired. Record tool versions when evaluating a build. The scripts do not alter your selected global Xcode or Rust default. Exact binary reproducibility is not the objective; a defined environment and pinned inputs should produce an equivalent working package.

## Package boundary

`package` first builds the production editor, then creates the bundle in staging. The packager accepts system libraries plus the explicitly bundled OpenMP runtime; unexpected non-system dependencies fail, including OCIO. It removes checkout-local search paths, installs a bundle-relative runtime path, copies notices and signs the runtime/app ad hoc. It never modifies the Cargo-built executable in place.

The bundle includes version/toolchain/source-state metadata and a production Cargo dependency-license inventory. Archive SHA-256 is written beside the ZIP. `verify-package` extracts the ZIP under a new path outside the checkout, checks libraries/search paths and the code signature, then executes it with an isolated home/current directory and no inherited loader overrides. Loader diagnostics must identify the bundled OpenMP library. Packaging tests also exercise real compiled/signed fixture bundles, path names containing spaces and rejection of an external search path or missing runtime.

Ad-hoc signing permits local execution; it provides neither Developer ID identity nor notarization/Gatekeeper distribution approval. The package identifier is still development-local and file associations are not configured.

## Fresh-checkout exercise

Run `cargo xtask all` in a checkout with no `target/`, `.cache/` or native-prefix overrides. Preinstalled developer tools and normal Cargo/rustup download caches are allowed prerequisites; project build artifacts are not. To validate cold acquisition as well, use empty download caches and network access. Do not describe archive-seeded or reused-prefix checks as a successful cold-download exercise.

Re-run after preparation to check that setup/building is repeatable. A separate relocated-package check establishes the bundle's loading independence, not clean-machine or minimum-OS compatibility. Retain the relevant command results and limits when reporting verification.

## Continuous integration

[ci.yml](../.github/workflows/ci.yml) runs the hosted-runnable subset on `macos-15` (Apple Silicon) for pull requests, pushes to `main` and manual dispatch. The `checks` job runs `cargo xtask tools`, the formatting gate, the documentation link gate, the packaging/format-tool unit tests and the numerical-core tests. The `editor` job runs `cargo xtask setup`, the Metal-free direct-SDK adapter tests, production and all-features clippy, then `cargo xtask package`/`verify-package` and uploads the ZIP as a workflow artifact.

Hosted macOS runners provide no Metal device, so CI deliberately does not run the editor library tests, kittest `ui-tests` or GPU suites; a green CI run is not equivalent to a full local `cargo xtask check`. Missing Metal is a coverage limit to run locally or on a labeled self-hosted runner, not something to silence in the workflow. GitHub's runner images also change over time (Xcode and tool versions drift); the pinned native-dependency cache is keyed on the setup scripts, so force a fresh cache when a runner-image change needs re-verification.

CI uses SHA-pinned actions with `contents: read` permissions, per-run concurrency cancellation, and selects Xcode 26.3 via `DEVELOPER_DIR` because the image's default Xcode (16.4) ships a `swift-format` too old for the checked-in `.swift-format` schema. A runner-image update may require re-pinning that path. Cached paths are the pinned native dependencies and format tools (keyed on the setup scripts) and Cargo downloads (keyed on the lockfile); `target/` is not cached. The workflow mirrors the documented local commands rather than defining a second build system.

## Before public distribution

The current ZIP is a local test artifact. [NOTICE.md](../NOTICE.md) describes license/asset and release boundaries. Native dependencies need review alongside Cargo dependencies; the generated inventory is not a legal-compatibility checker. Public release work still includes a fresh dependency/asset license review, outstanding asset clearance, Developer ID signing/notarization if chosen, clean-machine execution and platform-specific acceptance. Do not infer release clearance from a green build or a valid ad-hoc signature.
