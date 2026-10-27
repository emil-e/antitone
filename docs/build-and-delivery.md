# Local build and delivery

The [build and run](build-and-run.md) guide owns prerequisites and commands. Local packaging produces a macOS `.app` and ZIP on Apple Silicon and a portable ZIP on Windows x64; Linux packaging is not implemented. [Continuous integration](#continuous-integration) verifies both platforms, and tagging publishes both packages.

## Build ownership

Cargo builds Rust and selects feature-specific native dependencies. `build.rs` compiles small bridges with `cc`, builds the vendored Adobe SDK through `native/dng/CMakeLists.txt` and publishes link settings to Cargo. CMake/Ninja owns that native build; it is not an outer build system that runs Cargo. Core-only Cargo commands still need no native libraries or desktop stack.

LibRaw/OpenMP and the independent OCIO test dependency use the maintained native setup scripts. Their archives are pinned and SHA-256 checked before extraction/building. The SDK is already source in the checkout, with provenance and per-file checksums; its native library is assembled from source libraries rather than an Adobe validator executable's object cache.

`cargo xtask` is a small host-side orchestration crate, independent of application/native dependencies. It sequences the existing setup/format/package tools and Cargo commands. The Python standard-library packager remains responsible for macOS bundle metadata, dependency loading, notices and archive verification. Neither layer implements an incremental build engine. `build.rs` never downloads SDK source, installs tools, packages or signs the app.

## Checks and environment

The `cargo xtask` setup/check/package sequence below is the macOS workflow. Windows uses the PowerShell setup and packaging scripts in [build and run](build-and-run.md#windows), with ordinary Cargo commands for verification.

`setup` checks tools, runs native setup, installs pinned Ruff/clang-format into a local Python environment and installs the revision-pinned WGSL formatter locally through Cargo. `setup-native` stops after native setup; jobs that do not format sources use it. Rustup selects the checked-in Rust toolchain. All generated tools/dependencies stay in ignored storage or Cargo's normal cache; no system installation is performed.

`check` runs authored-source formatting, packaging/format-tool tests, core tests, production editor tests/lints, all-feature reference tests/lints and release-profile composed UI tests. Production is checked separately so feature unification cannot conceal an OCIO runtime dependency. No private samples are needed. Ignored photographic/resource diagnostics and explicitly launched native-window exercises remain separate, as described in [development](development.md).

Metal tests require a Metal device. Missing hardware is a verification failure/limit, not a reason to silently skip tests. The automated pipeline does not establish photographic, physical-input, native-window or actual macOS-14 acceptance. The relocated package smoke check runs `--version`, not an editor interaction scenario.

Use `DEVELOPER_DIR` to select Xcode and the normal Cargo target-directory override if desired. Record tool versions when evaluating a build. The scripts do not alter your selected global Xcode or Rust default. Exact binary reproducibility is not the objective; a defined environment and pinned inputs should produce an equivalent working package.

## Package boundary

On Windows, `scripts/package-windows.ps1` builds the editor and stages `antitone.exe`, the app-local Visual C++/OpenMP runtime DLLs and component notices into a portable ZIP. It checks the staged executable with `--version`, extracts the ZIP into a fresh path containing spaces, checks executable/runtime hashes and repeats the smoke test there. A `SHA256SUMS` file accompanies the ZIP; keep the DLLs beside the executable when distributing or moving the folder.

On macOS, `package` first builds the production editor, then creates the bundle in staging. The packager accepts system libraries plus the explicitly bundled OpenMP runtime; unexpected non-system dependencies fail, including OCIO. It removes checkout-local search paths, installs a bundle-relative runtime path, copies notices and signs the runtime/app ad hoc. It never modifies the Cargo-built executable in place.

The bundle includes version/toolchain/source-state metadata and a production Cargo dependency-license inventory. Archive SHA-256 is written beside the ZIP. `verify-package` extracts the ZIP under a new path outside the checkout, checks libraries/search paths and the code signature, then executes it with an isolated home/current directory and no inherited loader overrides. Loader diagnostics must identify the bundled OpenMP library. Packaging tests also exercise real compiled/signed fixture bundles, path names containing spaces and rejection of an external search path or missing runtime.

Ad-hoc signing permits local execution; it provides neither Developer ID identity nor notarization/Gatekeeper distribution approval. The package identifier is still development-local and file associations are not configured.

## Fresh-checkout exercise

Run the platform's [setup, checks and packaging](build-and-run.md) in a checkout with no `target/`, `.cache/` or native-prefix overrides. Preinstalled developer tools and normal Cargo/rustup download caches are allowed prerequisites; project build artifacts are not. To validate cold acquisition as well, use empty download caches and network access. Do not describe archive-seeded or reused-prefix checks as a successful cold-download exercise.

Re-run after preparation to check that setup/building is repeatable. A separate relocated-package check establishes the bundle's loading independence, not clean-machine or minimum-OS compatibility. Retain the relevant command results and limits when reporting verification.

## Continuous integration

Development is pull-request based: change work happens on task branches, opens a PR into `main`, and two workflows gate it — [macos.yml](../.github/workflows/macos.yml) on `macos-15` (Apple Silicon) and [windows.yml](../.github/workflows/windows.yml) on `windows-latest` x64. Both also run on pushes to `main` and manual dispatch, so merges and the trunk tip stay continuously verified.

CI covers the hardware-independent subset only. The split of responsibility:

| Verification | Where it runs |
| --- | --- |
| Authored-source formatting, documentation links, packaging/format-tool unit tests | macOS `checks` job, hosted (`cargo xtask tools` first) |
| Numerical-core tests | macOS `checks` job, hosted |
| Native setup, vendored-SDK adapter tests, production and all-features clippy | macOS `editor-checks` job, hosted (Metal-free stages) |
| Native setup, package build, relocated ZIP verification and ZIP artifact | macOS `package` job, hosted |
| Windows formatting, clippy, numerical-core and direct-SDK tests | Windows `lint-and-tests` job, hosted |
| Windows editor build and portable ZIP artifact | Windows `package` job, hosted |
| Editor library tests, kittest `ui-tests`, GPU suites | Agent/local machine before integration — these need a Metal device on macOS or a Vulkan device on Windows, outside the hardware-independent hosted jobs |
| Photographic, physical-input, native-window acceptance | Unchanged: manual/local exercise, never CI |

The all-features clippy compiles the reference and `ui-tests` configurations without running them, so that test-harness code stays buildable in CI even though only a local run can execute it.

A green PR therefore does not authorize merging on its own: the device-dependent suites in [development checks](development.md) still need to run in an agent session or on the developer's machine before integration, until a labeled self-hosted runner joins CI. Do not respond to missing hosted coverage by making those tests skip or silently pass in the workflow.

GitHub's runner images change over time (Xcode and tool versions drift); the pinned native-dependency cache is keyed on the setup scripts, so force a fresh cache when a runner-image change needs re-verification. CI uses SHA-pinned actions with `contents: read` permissions, per-run concurrency cancellation, and selects Xcode 26.3 via `DEVELOPER_DIR` because the image's default Xcode (16.4) ships a `swift-format` too old for the checked-in `.swift-format` schema; a runner-image update may require re-pinning that path. Cached paths are the pinned native dependencies (keyed on the setup scripts), the pinned formatting tools (keyed on the xtask crate) and Cargo downloads (keyed on the lockfile); `target/` is not cached. Formatting tools are installed and cached only in the job that formats sources, and their cache is restored only on an exact key match so a changed pin cannot keep an older formatter. Within a workflow one job writes each cache, and the jobs that only need its contents restore it without saving. The jobs each run the stages they need concurrently, so a pull request finishes when the slowest job does. The workflow mirrors the documented local commands rather than defining a second build system.

## Releasing a tag

Pushing a `v*` tag publishes both platforms: [release.yml](../.github/workflows/release.yml) first validates that the tag matches the crate version in `Cargo.toml`, then builds macOS ARM64 and Windows x64 in separate jobs. macOS runs native setup, direct-SDK adapter tests, `cargo xtask package` and relocated-package verification; Windows reuses the [Windows workflow](../.github/workflows/windows.yml), including its tests and package smoke checks.

Only after both jobs succeed does publication download the artifacts, verify each platform's SHA-256 manifest and ZIP integrity with [release assembly](../scripts/prepare-release.py), and create one GitHub Release containing both ZIPs and a combined `SHA256SUMS`. Publishing is direct rather than draft because pushing a tag is a deliberate act. Device-dependent suites remain the local pre-merge responsibility. Before tagging, complete the license review and clean-machine acceptance under [before public distribution](#before-public-distribution).

## Before public distribution

Packaged ZIPs and CI artifacts are verification outputs; public distribution requires the review below. [NOTICE.md](../NOTICE.md) describes license/asset and release boundaries. Native dependencies need review alongside Cargo dependencies; the generated inventory is not a legal-compatibility checker. Public release work still includes a fresh dependency/asset license review, outstanding asset clearance, Developer ID signing/notarization if chosen, clean-machine execution and platform-specific acceptance. Do not infer release clearance from a green build or a valid ad-hoc signature.
