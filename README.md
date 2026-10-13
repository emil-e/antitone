<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/wordmark-amber.svg">
    <img src="assets/branding/wordmark-density.svg" alt="Antitone" width="320">
  </picture>
</h1>

An editor for camera-scanned film negatives. Analyze a roll to establish shared colour and individual exposures, then refine the photographs manually and export TIFF or EXR.

Antitone works from linear camera measurements rather than inverting a displayed JPEG. Calibration and response are shared across a roll; Photo adjustments provide individual grading. [How processing works](docs/processing.md) and [how Analyze chooses a starting grade](docs/roll-analysis.md) explain those two separate jobs.

The current photo input is ARW RAW, with Sony ILCE-1M2 lossless-compressed ARW as the supported camera baseline. Saved grades use JSON projects. Developed JPEG/TIFF/EXR import, dust removal, catalogs and HDR display output are not implemented; this checkout is not a distribution-ready app.

## Prerequisites

The long-term goal is a cross-platform application. The implemented desktop application and development workflow currently target **macOS on Apple Silicon**; other platforms are not yet supported. Broader device testing and macOS 14 acceptance remain open; see [project status](PLAN.md).

Install these tools using your normal developer setup:

- [Rust through rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects Rust 1.99.0, rustfmt and Clippy without changing your global default. First use downloads missing components into your usual rustup installation. `Cargo.toml` separately declares an MSRV of 1.95.
- Apple's Xcode command-line tools (`xcode-select --install`), or Xcode with its developer directory selected.
- CMake and Ninja. If you use Homebrew: `brew install cmake ninja`.

Cargo and native builds select Apple Clang; no GCC installation is needed. Setup scripts do not install system packages.

## Build and run

From the repository root:

```sh
# Download, verify and build pinned LibRaw 0.22.0, LLVM OpenMP 22.1.8
# and OpenColorIO 2.6.0.
./scripts/setup-native.sh

# Build the editor.
cargo build-app

# Build and launch the editor, optionally opening a RAW or saved project.
cargo app
cargo app -- /path/to/photo.ARW
cargo app -- /path/to/grade.antitone.json
```

The native setup needs network access and writes ignored `.cache/native-deps/` (LibRaw/OpenMP) and `.cache/deps/` (OCIO). Cargo downloads Rust dependencies into its normal cache and writes build products to `target/`. Subsequent builds do not need to rerun native setup unless those dependencies are removed or changed.

The aliases are ordinary Cargo commands defined in `.cargo/config.toml`. The equivalent explicit commands are:

```sh
cargo build --locked --release --features editor --bin antitone
cargo run --locked --release --features editor --bin antitone -- /path/to/photo.ARW
```

Plain `cargo build --locked` builds the lightweight numerical core, not the editor. Editor features remain opt-in so core development does not require the native libraries or desktop stack.

For an existing compatible native build or a separate worktree, see [reusing native builds](docs/native-development.md#reusing-native-builds). The ordinary setup remains the command above.

The editor has its Dock icon when launched with `cargo app`. To package the built release binary with a Finder icon as a local macOS application:

```sh
./scripts/package-macos.py
open .cache/app/Antitone.app
```

The bundle is disposable and can be regenerated after `cargo build-app`. It still links OCIO and OpenMP through absolute local library paths; keep those libraries available. This is local development packaging, not a distributable release: dependency bundling, signing/notarization and file associations remain unfinished.

[Branding assets](assets/branding/README.md) include self-contained SVG wordmarks, portable PNG/ICO icons, the macOS icon and their regeneration command. These assets do not imply that the application supports other platforms yet.

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
