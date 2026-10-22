<h1>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/branding/wordmark-amber.svg">
    <img src="assets/branding/wordmark-density.svg" alt="Antitone" width="320">
  </picture>
</h1>

An editor for camera-scanned film negatives. Analyze a roll to establish shared colour and individual exposures, then refine the photographs manually and export TIFF or EXR.

Antitone works from linear camera measurements rather than inverting a displayed JPEG. Calibration and response are shared across a roll; Photo adjustments provide individual grading. [How processing works](docs/processing.md) and [how Analyze chooses a starting grade](docs/roll-analysis.md) explain those two separate jobs.

The current photo input is camera RAW from the accepted three-colour Bayer domain: Sony ARW, with Sony ILCE-1M2 lossless-compressed ARW as the supported camera baseline, plus Canon CR2/CR3 (including CRAW), Nikon NEF, Olympus ORF and Panasonic RW2. Saved grades use `.antitone` project files (JSON). Developed JPEG/TIFF/EXR import, catalogs and HDR display output are not implemented; this checkout is not a distribution-ready app. The [provisional Dust workflow](docs/editor.md#provisional-dust-cleanup) saves source-owned manual repairs and applies them to completed positive views, proper thumbnails and TIFF/AP0 EXR exports.

## Prerequisites

The long-term goal is a cross-platform application. The implemented desktop application and development workflow currently target **macOS on Apple Silicon**; other platforms are not yet supported. Broader device testing and macOS 14 acceptance remain open.

Install these tools using your normal developer setup:

- [Rust through rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects Rust 1.99.0, rustfmt, Clippy, rust-analyzer and Rust sources without changing your global default. First use downloads missing components into your usual rustup installation. `Cargo.toml` separately declares an MSRV of 1.95.
- Xcode with its developer directory selected (Apple Clang, macOS SDK and Swift formatter). This workflow is tested with full Xcode; command-line-tools-only acceptance remains separate.
- CMake and Ninja. If you use Homebrew: `brew install cmake ninja`.

Cargo and native builds select Apple Clang; no GCC installation is needed. Setup scripts do not install system packages.

## Build and run

From the repository root:

```sh
# Download, verify and build pinned LibRaw 0.22.0, LLVM OpenMP 22.1.8
# and OpenColorIO 2.6.0 (for opt-in reference checks).
./scripts/setup-native.sh

# Build the editor.
cargo build-app

# Build and launch the editor, optionally opening a RAW or saved project.
cargo app
cargo app -- /path/to/photo.ARW
cargo app -- /path/to/grade.antitone
```

The native setup needs network access and writes ignored `.cache/native-deps/` (LibRaw/OpenMP) and `.cache/deps/` (OCIO). The pinned Adobe SDK and its dependencies are vendored as source; Cargo builds them through CMake/Ninja without an Adobe download or validator Xcode project. Cargo downloads Rust dependencies into its normal cache and writes build products to `target/`. Subsequent builds do not need to rerun native setup unless those dependencies are removed or changed.

The aliases are ordinary Cargo commands defined in `.cargo/config.toml`. The equivalent explicit commands are:

```sh
cargo build --locked --release --features editor --bin antitone
cargo run --locked --release --features editor --bin antitone -- /path/to/photo.ARW
```

Plain `cargo build --locked` builds the lightweight numerical core, not the editor. Editor features remain opt-in so core development does not require the native libraries or desktop stack.

For an existing compatible native build or a separate worktree, see [reusing native builds](docs/native-development.md#reusing-native-builds).

### End-to-end local delivery

With Git, Rust/rustup, Xcode, Python 3, CMake and Ninja available:

```sh
cargo xtask all
```

This prepares native dependencies and project-local pinned formatters, runs the automated checks, builds the production editor and creates `.cache/app/Antitone.app` and `.cache/app/Antitone-macos-arm64.zip`. It extracts the ZIP outside the checkout, audits its library/search paths and signature, and runs the packaged executable with isolated environment/storage. No system packages are installed. Setup requires network access; checks require Metal. It opens no editor windows or private photographs.

The stages can also be run separately:

```sh
cargo xtask doctor          # Inspect required tools.
cargo xtask setup           # Download/verify/build dependencies and prepare formatters.
cargo xtask check           # Authored-source formatting, lints, production/reference and UI tests.
cargo xtask package         # Build the production editor, bundle libraries/notices, create ZIP.
cargo xtask verify-package  # Extract and verify the ZIP outside the checkout.
open .cache/app/Antitone.app
```

The package contains its OpenMP runtime; it no longer needs a library in the checkout. Normal rendering/export does not link OCIO; the opt-in `ocio-reference` feature remains a diagnostic dependency. Bundles are ad-hoc signed for local execution, **not Developer ID signed or notarized**. Public-release source distribution, asset/license clearance, file associations and clean-machine/macOS-14 acceptance remain separate. See [build and delivery](docs/build-and-delivery.md) for boundaries and verification.

Changes land through pull requests: CI runs the hardware-independent subset of this workflow on every PR, while the Metal-dependent suites still run in agent sessions or on the developer's machine before integration.

[Branding assets](assets/branding/README.md) include self-contained SVG wordmarks, portable PNG/ICO icons, the macOS icon and their regeneration command.

### Direct DNG SDK development

The editor includes the bounded direct-SDK DNG backend. Non-DNG/Sony development remains on LibRaw. The `dng-sdk` feature also exposes the independent adapter and read-only probe without the editor or LibRaw.

Adobe DNG SDK **1.7.1 build 2768** and its source dependencies are [vendored with provenance and original notices](vendor/dng-sdk/PROVENANCE.md). Using it accepts Adobe's terms; Antitone's MIT license does not replace its license. With the Apple compiler/SDK, CMake and Ninja available, Cargo builds it automatically:

```sh
cargo test --locked --features dng-sdk --lib dng::
# Explicit main-sensor pixel bound; inputs remain read-only.
cargo run --locked --features dng-sdk --bin antitone-dng-probe -- 1000000 /path/to/authored.dng
# Compare bounded execution budgets; reports split timings and pixel/CFA/eligibility digests.
cargo run --locked --release --features dng-sdk --bin antitone-dng-probe -- \
  --threads 8 60000000 /path/to/photo.dng
```

SDK build products are generated under Cargo's target directory. No source/build prefix or pre-existing validator objects are needed. Validation instrumentation is retained in the native library; the validator executable is not built.

## Checks

Core checks do not require native setup:

```sh
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

For feature-specific tests, see [development checks](docs/development.md). Numerical/reference, RAW, export and presentation probes are in [diagnostics](docs/diagnostics.md). `--offline` is useful with a populated cache, but is not required for normal builds. Local RAW samples are optional, read-only and never downloaded by setup.

## License

Antitone's original code is licensed under **MIT**; see [LICENSE](LICENSE) and [acknowledgments/distribution boundaries](NOTICE.md). Third-party code and assets retain their own terms.

## Contributing

The [contributor guide](docs/README.md) helps you choose an area without reading every document. Start with its explanation, follow the implementation links, and run the relevant [development checks](docs/development.md).

[Architecture](docs/architecture.md) maps ownership.
