<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/wordmark-amber.svg">
    <img src="assets/branding/wordmark-density.svg" alt="Antitone" width="320">
  </picture>
</h1>

An editor for camera-scanned film negatives. Analyze a roll to establish shared colour and individual exposures, then refine the photographs manually and export TIFF or EXR.

Antitone works from linear camera measurements rather than inverting a displayed JPEG. Calibration and response are shared across a roll; Photo adjustments provide individual grading. [How processing works](docs/processing.md) and [how Analyze chooses a starting grade](docs/roll-analysis.md) explain those two separate jobs.

The current photo input is ARW RAW, with Sony ILCE-1M2 lossless-compressed ARW as the supported camera baseline. Saved grades use `.antitone` project files (JSON). Developed JPEG/TIFF/EXR import, catalogs and HDR display output are not implemented; this checkout is not a distribution-ready app. The [provisional Dust workflow](docs/editor.md#provisional-dust-cleanup) saves source-owned manual repairs and applies them to completed positive views, proper thumbnails and TIFF/AP0 EXR exports.

## Prerequisites

The long-term goal is a cross-platform application. The implemented desktop application and development workflow currently target **macOS on Apple Silicon**; other platforms are not yet supported. Broader device testing and macOS 14 acceptance remain open; see [project status](PLAN.md).

Install these tools using your normal developer setup:

- [Rust through rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects Rust 1.99.0, rustfmt, Clippy, rust-analyzer and Rust sources without changing your global default. First use downloads missing components into your usual rustup installation. `Cargo.toml` separately declares an MSRV of 1.95.
- Full Xcode with its developer directory selected for the editor's DNG SDK dependency. Core/LibRaw-only work can use Apple's command-line tools.
- CMake and Ninja. If you use Homebrew: `brew install cmake ninja`.

Cargo and native builds select Apple Clang; no GCC installation is needed. Setup scripts do not install system packages.

## Build and run

From the repository root:

```sh
# Download, verify and build pinned LibRaw 0.22.0, LLVM OpenMP 22.1.8
# and OpenColorIO 2.6.0 (for opt-in reference checks).
./scripts/setup-native.sh

# Obtain Adobe DNG SDK 1.7.1 build 2768 and review its terms first; no implicit download.
python3 scripts/build-dng-sdk.py --archive /absolute/path/to/SDK.zip

# Build the editor.
cargo build-app

# Build and launch the editor, optionally opening a RAW or saved project.
cargo app
cargo app -- /path/to/photo.ARW
cargo app -- /path/to/grade.antitone
```

The native setup needs network access and writes ignored `.cache/native-deps/` (LibRaw/OpenMP), `.cache/deps/` (OCIO) and `.cache/dng-sdk/` (explicitly supplied Adobe SDK). Cargo downloads Rust dependencies into its normal cache and writes build products to `target/`. Subsequent builds do not need to rerun native setup unless those dependencies are removed or changed.

The aliases are ordinary Cargo commands defined in `.cargo/config.toml`. The equivalent explicit commands are:

```sh
cargo build --locked --release --features editor --bin antitone
cargo run --locked --release --features editor --bin antitone -- /path/to/photo.ARW
```

Plain `cargo build --locked` builds the lightweight numerical core, not the editor. Editor features remain opt-in so core development does not require the native libraries or desktop stack.

For an existing compatible native build or a separate worktree, see [reusing native builds](docs/native-development.md#reusing-native-builds).

The editor has its Dock icon when launched with `cargo app`. To package the built release binary with a Finder icon as a local macOS application:

```sh
./scripts/package-macos.py
open .cache/app/Antitone.app
```

The bundle is disposable and can be regenerated after `cargo build-app`. It still links OpenMP through an absolute local library path; keep that library available. Normal editor rendering/export does not link OCIO; the opt-in `ocio-reference` feature adds that diagnostic dependency. This is local development packaging, not a distributable release: dependency bundling, signing/notarization and file associations remain unfinished.

[Branding assets](assets/branding/README.md) include self-contained SVG wordmarks, portable PNG/ICO icons, the macOS icon and their regeneration command.

### Direct DNG SDK setup

The editor includes the bounded [direct-SDK DNG backend](docs/research/dng-sdk.md#maintained-direct-sdk-adapter). Non-DNG/Sony development remains on LibRaw. The `dng-sdk` feature also exposes the independent adapter and read-only probe without the editor or LibRaw. Its current build needs full Xcode on Apple Silicon, not just command-line tools.

Obtain Adobe DNG SDK **1.7.1 build 2768** yourself and review its [license and dependency terms](docs/research/dng-sdk.md#release-build-and-licensing). Setup never downloads it implicitly. Using the SDK accepts Adobe's terms. With the supplied ZIP:

```sh
python3 scripts/build-dng-sdk.py --archive /absolute/path/to/SDK.zip
cargo test --locked --features dng-sdk --lib dng::
# Explicit main-sensor pixel bound; inputs remain read-only.
cargo run --locked --features dng-sdk --bin antitone-dng-probe -- 1000000 /path/to/authored.dng
# Compare bounded execution budgets; reports split timings and pixel/CFA/eligibility digests.
cargo run --locked --release --features dng-sdk --bin antitone-dng-probe -- \
  --threads 8 60000000 /path/to/photo.dng
```

SDK source/build products stay under `.cache/dng-sdk/`. To reuse an existing build of that exact SDK, use `--source /absolute/path/to/dng_sdk_1_7_1 --reuse-build` instead of `--archive`; omit `--reuse-build` to invoke the official Xcode project. `DNG_SDK_PREFIX` can point Cargo at another prepared prefix containing `source-path.txt`, `defines.txt` and `lib/libantitone_dng_sdk.a`; its referenced SDK headers must remain available. This local dependency build includes validator instrumentation; it is not a distributable SDK package.

## Checks

Core checks do not require native setup:

```sh
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

For feature-specific tests, see [development checks](docs/development.md). Numerical/reference, RAW, export and presentation probes are in [diagnostics](docs/diagnostics.md). `--offline` is useful with a populated cache, but is not required for normal builds. Local RAW samples are optional, read-only and never downloaded by setup.

## Contributing

The [contributor guide](docs/README.md) helps you choose an area without reading every document. Start with its explanation, follow the implementation links, and run the relevant [development checks](docs/development.md).

[Architecture](docs/architecture.md) maps ownership. [The roadmap](PLAN.md) and [work plans](docs/plans/README.md) distinguish current behavior from unfinished features and acceptance.
