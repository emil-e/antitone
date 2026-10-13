# Antitone

A macOS editor for camera-scanned film negatives, with native-camera inversion, GPU previews, saveable projects and TIFF/EXR export.

## Prerequisites

The current development platform is **macOS on Apple Silicon**. Broader device testing and macOS 14 acceptance remain open; see [project status](PLAN.md).

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

Existing compatible native builds can be selected with `LIBRAW_SOURCE` (LibRaw source/build directory containing `lib/libraw_r.a`) and `OCIO_PREFIX` (installed OCIO prefix containing `include/` and `lib/`). These are build/probe overrides; `setup-native.sh` always builds the pinned checkout-local dependencies. LibRaw must use the accepted OpenMP build, and the pinned runtime remains required at `.cache/native-deps/openmp-22.1.8-install`; `LIBRAW_SOURCE` does not relocate it. System/Homebrew OpenMP runtimes are not supported inputs. Do not substitute different library versions or build options without validating the processing/rendering contracts.

The editor currently links OCIO and OpenMP through absolute local library paths. Keep those libraries available when running the binary; this workflow is not a distributable app bundle.

## Checks

Core checks do not require native setup:

```sh
cargo fmt --all -- --check
cargo test --locked --no-default-features
```

For feature-specific tests, see [development checks](docs/development.md). Numerical/reference, RAW, export and presentation probes are in [specialist verification](docs/build-and-probes.md). `--offline` is useful with a populated cache, but is not required for normal builds. Local RAW samples are optional, read-only and never downloaded by setup.

## More context

- [Roadmap](PLAN.md)
- [Architecture](docs/architecture.md)
- [Documentation map](docs/README.md)
