# Build and verification runbook

Run from the repository root. Use [the README](../README.md) for initial developer setup; humans and agents use the same Cargo/native setup path. macOS/Apple Silicon is the current supported checkout; samples are read-only/local. This is a command guide, not a claim that all checks ran today. Historical outcomes are in [research evidence](research/evidence/README.md); acceptance gaps/status are in [PLAN.md](../PLAN.md).

## Toolchain and native dependencies

Run ordinary `cargo` commands. `rust-toolchain.toml` selects Rust 1.99.0 with rustfmt/Clippy through normal rustup project selection; package MSRV is separately 1.95. Your global default toolchain is unchanged. `.cargo/config.toml` selects Apple Clang for Cargo native builds, including when an ambient `CC`/`CXX` points elsewhere. Older historical records use the removed `scripts/cargo.sh` wrapper and checkout-local Rust; those are not current setup instructions.

Native dependencies are pinned LibRaw 0.22.0, LLVM OpenMP 22.1.8 and OCIO 2.6.0:

```sh
./scripts/setup-native.sh
```

Setup checks host prerequisites, then runs `build-libraw.sh` (which builds OpenMP first) and `build-ocio.sh`; these can also be invoked individually. Bootstrap/downloads require network. LibRaw/OpenMP use private ignored `.cache/native-deps`; OCIO uses `.cache/deps`. OpenMP's LLVM release archive is SHA256-pinned and extracted selectively, source-built arm64/macOS-14 with `@rpath/libomp.dylib`. Licenses remain beside local products. Agents must obtain any required environment permissions and must not install system/Homebrew packages without a separate request.

`LIBRAW_SOURCE` and `OCIO_PREFIX` select existing compatible native builds for Cargo/probes; setup always builds the pinned local defaults. OpenMP remains required at `.cache/native-deps/openmp-22.1.8-install`, with no inherited runtime override. Existing serial LibRaw builds in `.cache/deps` are not the production default. OpenMP/OCIO rpaths remain checkout-local, not distributable packaging. Little CMS is built statically through the pinned Rust dependency; do not set `LCMS2_LIB_DIR` to override that choice.

The focused commands below use `--offline` to avoid downloads during verification with an already populated cache. For a first build, omit it; retain `--locked` to use the committed dependency resolution.

## Focused checks

Choose checks relevant to the change before running the broad suite:

```sh
cargo fmt --all -- --check
cargo test --locked --offline --no-default-features --test camera_inversion --test processing --test film_response
cargo test --locked --offline --features project --test camera_project --test project --test film_project
cargo test --locked --offline --features gpu --test gpu_film_response -- --nocapture
```

Project/scalar checks do not require native RAW/render libraries. GPU checks require available Metal. For a processing/rendering integration closure, when warranted:

```sh
cargo test --locked --offline --all-features
cargo clippy --locked --offline --all-features --all-targets -- -D warnings
```

Do not treat unavailable hardware or an unrelated test failure as permission to weaken gates or broaden scope. Numerical gates and actual onscreen/device acceptance are separate.

## Editor and layout smoke

```sh
cargo build-app --offline
target/release/antitone samples/DSC04416.ARW
# Explicit diagnostic calibration, not the user's saved grade:
target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
# Four normal/narrow Roll/Photo layout holds:
ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

The editor supports explicit Save/Open, stable-ID photo state and dirty New/Open/Close protection; recovery/relink/undo remain unwired. Launch a document with `target/release/antitone /path/to/grade.antitone.json`. The same-row File and Export menus replace action buttons. Fit, physical-pixel 100%, source bypass, picker return and full-source regional refinement are implemented. New grades use camera model 4/toe 2.5; classic-curve diagnostics also use model 4. Old AP0 projects reject rather than migrate. The current smoke verifies actual GPU/worker/layout paths, not photographic calibration or broader-device performance.

For Save/Open, append one additional unvisited source:

```sh
target/release/antitone --smoke-project \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW samples/DSC04393.ARW
```

This uses temporary project files, leaves RAWs untouched, and verifies exact model-4 response state, custom saved WB after New/different-WB import, unvisited records, working/sRGB/P3 pixel reproduction, live refinement and failed Open preservation. A zero-onscreen-frame result remains failure; a locked macOS session can pass persistence assertions but fail this presentation gate. [Save/Open closure](plans/completed/save-open.md).

The flat-field editor/worker integration has an explicit read-only local-RAW/Metal test (temporary projects/32×32 corrected TIFF/AP0 EXR crops):

```sh
cargo test --locked --offline --release --all-features --lib \
  editor::flat::tests::real_reference_editor_save_open_and_corrected_exports \
  -- --ignored --nocapture
```

It uses `DSC01739`/`DSC01737`, tests cancellation/failure, automatic support acceptance, corrected thumbnail completion, Save/Open and Enabled/view preservation. It is headless; it does not establish actual reference-picker/rectangle onscreen or native-dialog acceptance. [Checkpoint/limits](plans/completed/flat-field-correction.md#verified-editor-checkpoint).

Scripted corrected onscreen acceptance (keeps the nonzero-visible-frame gate and rejects early window closure):

```sh
target/release/antitone --smoke-flat-field samples/DSC01737.ARW samples/DSC01739.ARW
target/release/antitone --smoke-flat-field samples/DSC01811.ARW samples/DSC01813.ARW
cargo test --locked --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

The host smoke renders Fit/reference support, exercises scripted geometry move/resize/cancel and support edits, verifies actual corrected Dmin/Dmax/Gamma/WB backend results against independent CPU calls, both-target settled previews/thumbnails, +2 EV native-pixel inspection, Enabled/Clear and exact Save/Open. It does not automate the native file dialog or physical gestures. The explicit full-size test writes temporary 8640×5760 sRGB/P3 TIFF and half/full AP0 EXR, decodes every pixel, checks metadata and 247 independent scalar-derived positions, and verifies signed/HDR EXR range. Both commands leave RAWs unchanged. [Acceptance snapshot/limits](research/evidence/flat-field-acceptance.txt).

For window capture use [the relocated capture helper](research/tools/README.md#manufacturer-study--capture); outputs stay under `.cache/probes`. Requires normal desktop/screen-capture permissions. Do not bypass occlusion or automate Lightroom/Resolve.

## Responsive image scopes

The above-controls slot provides positive-grade-only whole-frame approximate waveform/vectorscope, in selected output-native codes. RGB/luma reuse bins; vectorscope scale re-bins cached rendering without rerunning ACES. Source/reference views and crop drafts suspend scopes. Accepted crop/orientation follows canonical whole-frame mapping, independently of viewer navigation. [Contract](contracts/scopes.md), [implementation/integration closure](plans/completed/vectorscope-and-waveform.md).

```sh
cargo test --locked --offline --features editor --lib --test scopes --test viewer
# Bounded 1024×1024 flat/varied reduction probe; Metal, no RAWs/window:
cargo test --locked --offline --features editor --test scopes \
  scope_reduction_latency -- --ignored --nocapture
# Release editor built as above; scripted scope modes/amplification and layout holds:
ANTITONE_SMOKE_SCOPES=1 ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

The latency probe times encode/submit/wait for reductions only, not RAW/ACES processing or end-to-end gesture latency. GPU-bin/pixel readbacks are test-only; the live UI maps just 16 diagnostic bytes asynchronously with one outstanding job. Smoke retains nonzero-visible-frame acceptance and uses diagnostic calibration, not the user's grade. Capture only the application window during layout holds with the existing [helper](research/tools/README.md#manufacturer-study--capture); photographic screenshots remain ignored/private.

Worktrees share the normal rustup/Cargo download caches and keep separate `target/` products and native build directories by default. To reuse existing native builds, set `LIBRAW_SOURCE` and `OCIO_PREFIX` to their absolute paths; OpenMP must also be available at the fixed checkout-local path above, for example through an ignored shared `.cache/native-deps` link. `LIBRAW_SOURCE` does not relocate the OpenMP runtime. Use absolute RAW paths or read-only sample links for photographic checks; no per-worktree Rust bootstrap is needed.

## Rendering/import performance probes

The ignored editor probes leave samples unchanged and use local Metal. `import_and_refinement_latency` compares bounded queue depths 1/2/3 with and without simulated foreground GPU submissions and checks bitwise reconstruction agreement; it takes about 34 seconds on the recorded M5 snapshot. This is not physical control/presentation latency. `initial_import_with_renderer_prewarm` separates header/hash/unpack/process/resource phases, then verifies whole-frame scope processing/diagnostics remain coherent during seeded-thumbnail creation and disk restart. `project_relative_locator_cold_thumbnail` uses an ignored temporary symlink/project/cache to verify cold development against a resolved locator.

```sh
cargo test --locked --offline --release --features editor --lib import_and_refinement_latency -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib initial_import_with_renderer_prewarm -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib project_relative_locator_cold_thumbnail -- --ignored --nocapture
```

Combined geometry integration (corrected real RAW, shared affine refinement, tile seams, both targets and independent scalar coverage; no sample edits):

```sh
cargo test --locked --offline --features editor --lib --test scopes --test viewer --test affine_resample --test geometry_export --test camera_inversion --test gpu_film_response
cargo test --locked --offline --release --features editor --lib corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

Additional ignored native follow-up checks:

```sh
cargo test --locked --offline --release --features editor --lib development_and_region_picks -- --ignored --nocapture
# Build .cache/native-deps/probe per the linked guide before the contention probe:
cargo test --locked --offline --release --features editor --lib native_development_foreground_contention -- --ignored --nocapture
```

[Native complete-RGB/metadata/diagnostics comparison and build guide](research/tools/raw-development.md). Native thread stress excludes physical input/layout/surface latency, as with tile stress; scoped CPU policy and lazy decoding do not change source/development identity. `raw_development` logs include priority/requested thread count.

Runtime `source_registration`, `raw_import`, `raw_development`, `selected_source`, `thumbnail` and `settled_preview` logs identify phases. Thumbnail logs include memory/disk hits, cancellation and failure; source verification uses full-byte fingerprints, not timestamps. Settled `gpu_seconds` includes encoding/upload/submission/wait wall time, not hardware timestamps. [Selected results/limits](research/evidence/rendering-performance.txt); [completed integration boundaries](plans/completed/rendering-performance.md).

## RAW and complete GPU chain

Camera-model replacement checks use read-only train samples/diagnostic anchors, not a recovered photographic grade:

```sh
cargo test --locked --offline --release --features raw,gpu \
  --test camera_raw -- --ignored --nocapture
```

The ignored real flat-field Save/Open/corrected-export check above exercises the same native-camera model with an accepted map. These headless checks do not establish actual onscreen or physical colour acceptance.


```sh
cargo run --locked --offline --features raw,gpu \
  --bin antitone-calibration-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  --gpu-fixture samples/DSC04416.ARW
./scripts/probe-ocio.sh
```

Calibration writes full-resolution camera sample grids, matrix/parameters and CPU AP1 reference fixtures under `.cache/probes`; OCIO adds them to synthetic cases before the GPU complete-chain comparison. Regenerate in that order after changing fixtures. Camera source is `camera-source-model4.rgb16` plus post-inversion column-major `camera-to-ap1-model4.f64`; there is no float-source fallback. These are regenerable RAW-derived fixtures, not hidden authored source or a full-image comparison.

The older AP0/model-2/3 real-camera comparison and candidate precision probes are preserved as [historical research tools](research/tools/README.md#current-gate-reproduction), requiring their matching baseline APIs. Do not run them as a model-4 production gate or adapt them into fallback inversion paths; current root regressions are maintained.

## Rendering, gamut and ICC

After regenerating OCIO fixtures:

```sh
./scripts/probe-rendering.sh
cargo test --locked --offline --features render --test rendering
./scripts/probe-gamut.sh
# Independent current-target check, no RAW fixtures needed:
cargo run --locked --offline --features render \
  --bin antitone-rendering-probe -- --gamut-check
```

CPU renderer/scalar reference, exhaustive profile/transfer and independent ColorSync/Core Image checks address different boundaries. CPU/GPU agreement alone is insufficient for target colour preservation. Pinned production artifacts/checksums: [OCIO](../assets/ocio/README.md), [profiles](../assets/profiles/README.md).

`./scripts/probe-gamut.sh --compare-rec2020` intentionally exits 1 for the rejected 100-nit SDR Rec.2020 cases and includes a 500-nit control. That is a historical negative control, not a broken active-target check or authorization for HDR output. [Mechanism](research/aces-yellow-collapse.md).

## Export and cached sampling

```sh
cargo run --locked --offline --release --features raw,export \
  --bin antitone-export-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW .cache/probes/NEW_OUTPUT_DIRECTORY
cargo run --locked --offline --release --features raw \
  --bin antitone-scan-probe -- \
  samples/DSC04378.ARW 4000 2600 128 128
```

Export CLI uses explicit diagnostic calibration/default edits, full developed extent and no overwrite. Choose a **fresh** output directory; the UI instead exports the active metadata area. Exports can consume hundreds of MiB and significant time, so run only when relevant. Numerical TIFF/EXR reopen tests are cheaper checks for state/metadata changes. See [output limits/safety](contracts/output.md).

For an independent full TIFF/ImageIO reopening after a diagnostic export:

```sh
/usr/bin/clang++ -std=c++17 -O2 native/tiff_reference.mm \
  -framework Foundation -framework CoreGraphics -framework ImageIO \
  -o .cache/probes/tiff-reference
.cache/probes/tiff-reference .cache/probes/NEW_OUTPUT_DIRECTORY
```

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
# Leave the chart open:
cargo run --locked --offline --features ui \
  --bin antitone-presentation-probe -- --interactive
```

Core Image provides an encoding/reference comparison, not interactive rendering. The host explicitly tags extended-sRGB `RGBA16Float` SDR/EDR=false after configuration. The chart checks f32/half transfer/readback, ordinary UI and two resizes. **Zero submitted frames fails the onscreen check.** Layer/readback evidence does not establish physical-monitor matching, monitor transitions, UI blending, forced surface recovery, macOS-14 or other-device performance.

## Documentation/cache work

Inspect local Markdown links/anchors and moved references, and record exactly which relocation checks ran. Do not reflexively rerun the full GPU/RAW/export suite for documentation-only changes. Authored source and selected small decisive evidence belong outside ignored cache; dependencies, generated fixtures/private images and third-party downloads stay local. [Cache policy/tool inventory](research/tools/README.md).
