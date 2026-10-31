# Building and running

Commands for building the editor, running it and producing a local package. [Making and checking a change](development.md) covers the edit/test workflow; [build and delivery](build-and-delivery.md) covers what the packaging stages own and their limits.

## Prerequisites

These instructions cover macOS on Apple Silicon and Windows 11 on x64; [current state](../README.md#current-state) has the platform status. Choose your platform's prerequisites, then follow [build and run](#build-and-run).

Both platforms use [Rust through rustup](https://rustup.rs/). The checked-in `rust-toolchain.toml` selects Rust 1.99.0, rustfmt, Clippy, rust-analyzer and Rust sources without changing your global default. First use downloads missing components into your usual rustup installation. `Cargo.toml` separately declares an MSRV of 1.95.

### macOS

Install these tools using your normal developer setup:

- Xcode with its developer directory selected (Apple Clang, macOS SDK and Swift formatter). This workflow is tested with full Xcode; command-line-tools-only acceptance remains separate.
- CMake and Ninja. If you use Homebrew: `brew install cmake ninja`.

Cargo and native builds select Apple Clang; no GCC installation is needed. Setup scripts do not install system packages.

### Windows

Windows 11 on x64 with an MSVC toolchain builds from a plain PowerShell session; no developer prompt is needed.

- Visual Studio 2022 with the C++ tools (or Build Tools) and a Windows 11 SDK. `build.rs` finds MSVC itself and captures `VsDevCmd.bat`'s environment for the CMake steps, which need the SDK resource compiler.
- Rust through rustup with the `x86_64-pc-windows-msvc` target.
- CMake, Ninja and the DirectX Shader Compiler are not installed system-wide: `scripts/setup-windows.ps1` downloads pinned, hash-verified copies into the ignored `.cache/tools/` and builds LibRaw 0.22.3 into `.cache/native-deps/`.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1
cargo build-app
cargo app -- G:/path/to/photo.ARW
```

The editor presents through Vulkan, so the display driver must provide it; the build needs network access for the first setup. [Local delivery](#local-delivery-on-windows) produces the portable ZIP and checksums.

## Build and run

Run native setup once from the repository root: `./scripts/setup-native.sh` on macOS or `powershell -ExecutionPolicy Bypass -File scripts/setup-windows.ps1` on Windows. The macOS setup includes OpenColorIO for opt-in reference checks; Windows setup prepares the production editor dependencies.

Then, on either platform:

```sh
# Build the editor.
cargo build-app

# Build and launch the editor, optionally opening a RAW or saved project.
cargo app
cargo app -- /path/to/photo.ARW
cargo app -- /path/to/grade.antitone
```

Native setup needs network access and writes ignored `.cache/native-deps/`, plus `.cache/deps/` for macOS OCIO and `.cache/tools/` for Windows tools. Cargo downloads Rust dependencies into its normal cache and writes build products to `target/`. Subsequent builds do not need to rerun native setup unless those dependencies are removed or changed. [Build ownership](build-and-delivery.md#build-ownership) explains which layer builds what.

The aliases are ordinary Cargo commands defined in `.cargo/config.toml` and work on both platforms. The equivalent explicit commands are:

```sh
cargo build --locked --release --features editor --bin antitone
cargo run --locked --release --features editor --bin antitone -- /path/to/photo.ARW
```

Plain `cargo build --locked` builds the lightweight numerical core, not the editor. Editor features remain opt-in so core development does not require the native libraries or desktop stack.

For an existing compatible native build or a separate worktree, see [reusing native builds](native-development.md#reusing-native-builds).

### Local delivery on Windows

After native setup, build and package with:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/package-windows.ps1
```

The outputs are `.cache/dist/Antitone-windows-x64.zip` and `.cache/dist/SHA256SUMS`. The ZIP includes the app-local Visual C++/OpenMP runtime; packaging smoke-tests both staging and a freshly extracted copy. Use `-SkipBuild` to package an existing release binary.

### End-to-end local delivery on macOS

With Git, Rust/rustup, Xcode, Python 3.11 or later (for the release tooling/tests), CMake and Ninja available:

```sh
cargo xtask all
```

This prepares native dependencies and project-local pinned formatters, runs the automated checks, builds the production editor and creates `.cache/app/Antitone.app` and `.cache/app/Antitone-macos-arm64.zip`.

The stages can also be run separately:

```sh
cargo xtask doctor          # Inspect required tools.
cargo xtask setup           # Download/verify/build dependencies and prepare formatters.
cargo xtask setup-native    # Download/verify/build dependencies only; no formatters.
cargo xtask check           # Authored-source formatting, lints, production/reference and UI tests.
cargo xtask package         # Build the production editor, bundle libraries/notices, create ZIP.
cargo xtask verify-package  # Extract and verify the ZIP outside the checkout.
open .cache/app/Antitone.app
```

What the packaging and verification stages establish — and what they do not — is [build and delivery](build-and-delivery.md)'s subject. Bundle icons and the wordmark live in [branding assets](../assets/branding/README.md).
