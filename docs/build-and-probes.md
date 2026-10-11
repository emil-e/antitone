# Build and verification runbook

Run from the repository root. macOS/Apple Silicon is the current supported checkout; samples are read-only/local. This is a command guide, not a claim that all checks ran today. Historical outcomes are in [research evidence](research/evidence/README.md); acceptance gaps/status are in [PLAN.md](../PLAN.md).

## Toolchain and native dependencies

Use `./scripts/cargo.sh`: checkout-local Rust 1.99.0 under `.cache/rustup`, package MSRV 1.95. Global default/toolchain selection is not changed. Native dependencies are pinned LibRaw 0.22.0 and OCIO 2.6.0:

```sh
./scripts/build-libraw.sh
./scripts/build-ocio.sh
```

Bootstrap/downloads require network and populate ignored `.cache/deps`; do not install global/Homebrew packages. `LIBRAW_SOURCE` and `OCIO_PREFIX` override native source/prefix defaults. The current OCIO library rpath is checkout-local, not distributable packaging. Little CMS is built statically through the pinned Rust dependency; do not set `LCMS2_LIB_DIR` to override that choice. Cargo/native commands below use Apple Clang.

## Focused checks

Choose checks relevant to the change before running the broad suite:

```sh
./scripts/cargo.sh fmt --all -- --check
./scripts/cargo.sh test --offline --no-default-features --test processing --test film_response
./scripts/cargo.sh test --offline --features project --test project --test film_project
./scripts/cargo.sh test --offline --features gpu --test gpu_film_response -- --nocapture
```

Project/scalar checks do not require native RAW/render libraries. GPU checks require available Metal. For a processing/rendering integration closure, when warranted:

```sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh test --offline --all-features
CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh clippy --offline --all-features --all-targets -- -D warnings
```

Do not treat unavailable hardware or an unrelated test failure as permission to weaken gates or broaden scope. Numerical gates and actual onscreen/device acceptance are separate.

## Editor and layout smoke

```sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh build --offline --release --features editor --bin antitone
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

The editor supports explicit Save/Open, stable-ID photo state and dirty New/Open/Close protection; recovery/relink/undo remain unwired. Launch a document with `target/release/antitone /path/to/grade.antitone.json`. The same-row File and Export menus replace action buttons. Fit, physical-pixel 100%, source bypass, picker return and full-source regional refinement are implemented. New grades use model 3; old/default diagnostic grades may deliberately use model 2. The current smoke verifies actual GPU/worker/layout paths, not photographic calibration or broader-device performance.

For Save/Open, append one additional unvisited source:

```sh
target/release/antitone --smoke-project \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW samples/DSC04393.ARW
```

This uses temporary project files, leaves RAWs untouched, and verifies exact model-2/model-3 state, custom saved WB after New/different-WB import, unvisited records, working/sRGB/P3 pixel reproduction, live refinement and failed Open preservation. A zero-onscreen-frame result remains failure; a locked macOS session can pass persistence assertions but fail this presentation gate. [Save/Open closure](plans/completed/save-open.md).

The flat-field editor/worker integration has an explicit read-only local-RAW/Metal test (temporary projects/32×32 corrected TIFF/AP0 EXR crops):

```sh
./scripts/cargo.sh test --offline --release --all-features --lib \
  editor::flat::tests::real_reference_editor_save_open_and_corrected_exports \
  -- --ignored --nocapture
```

It uses `DSC01739`/`DSC01737`, tests cancellation/failure, automatic support acceptance, corrected thumbnail completion, Save/Open and Enabled/view preservation. It is headless; it does not establish actual reference-picker/rectangle onscreen or native-dialog acceptance. [Checkpoint/limits](plans/completed/flat-field-correction.md#verified-editor-checkpoint).

Scripted corrected onscreen acceptance (keeps the nonzero-visible-frame gate and rejects early window closure):

```sh
target/release/antitone --smoke-flat-field samples/DSC01737.ARW samples/DSC01739.ARW
target/release/antitone --smoke-flat-field samples/DSC01811.ARW samples/DSC01813.ARW
./scripts/cargo.sh test --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

The host smoke renders Fit/reference support, exercises scripted geometry move/resize/cancel and support edits, verifies actual corrected Dmin/Dmax/Gamma/WB backend results against independent CPU calls, both-target settled previews/thumbnails, +2 EV native-pixel inspection, Enabled/Clear and exact Save/Open. It does not automate the native file dialog or physical gestures. The explicit full-size test writes temporary 8640×5760 sRGB/P3 TIFF and half/full AP0 EXR, decodes every pixel, checks metadata and 247 independent scalar-derived positions, and verifies signed/HDR EXR range. Both commands leave RAWs unchanged. [Acceptance snapshot/limits](research/evidence/flat-field-acceptance.txt).

For window capture use [the relocated capture helper](research/tools/README.md#manufacturer-study--capture); outputs stay under `.cache/probes`. Requires normal desktop/screen-capture permissions. Do not bypass occlusion or automate Lightroom/Resolve.

## RAW and complete GPU chain

```sh
./scripts/cargo.sh run --offline --features raw,gpu \
  --bin antitone-calibration-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  --gpu-fixture samples/DSC04416.ARW
./scripts/probe-ocio.sh
```

Calibration writes full-resolution camera sample grids, matrix/parameters and CPU AP1 reference fixtures under `.cache/probes`; OCIO adds them to synthetic cases before the GPU complete-chain comparison. Regenerate in that order after changing fixtures. Camera source is `camera-source-model2.rgb16` plus column-major `camera-to-ap0-model2.f64`; there is no float-source fallback. These are regenerable RAW-derived fixtures, not hidden authored source or a full-image comparison.

Production film-response real-camera comparison and the broader candidate precision probe now live in [durable research tools](research/tools/README.md#current-gate-reproduction). Build named bins using the relocated manifest. Some historical research bins predate current APIs; never run all of them as the production suite.

## Rendering, gamut and ICC

After regenerating OCIO fixtures:

```sh
./scripts/probe-rendering.sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh test --offline --features render --test rendering
./scripts/probe-gamut.sh
# Independent current-target check, no RAW fixtures needed:
CC=/usr/bin/clang CXX=/usr/bin/clang++ \
  ./scripts/cargo.sh run --offline --features render \
  --bin antitone-rendering-probe -- --gamut-check
```

CPU renderer/scalar reference, exhaustive profile/transfer and independent ColorSync/Core Image checks address different boundaries. CPU/GPU agreement alone is insufficient for target colour preservation. Pinned production artifacts/checksums: [OCIO](../assets/ocio/README.md), [profiles](../assets/profiles/README.md).

`./scripts/probe-gamut.sh --compare-rec2020` intentionally exits 1 for the rejected 100-nit SDR Rec.2020 cases and includes a 500-nit control. That is a historical negative control, not a broken active-target check or authorization for HDR output. [Mechanism](research/aces-yellow-collapse.md).

## Export and cached sampling

```sh
./scripts/cargo.sh run --offline --release --features raw,export \
  --bin antitone-export-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW .cache/probes/NEW_OUTPUT_DIRECTORY
./scripts/cargo.sh run --offline --release --features raw \
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
./scripts/cargo.sh run --offline --features ui --bin antitone-presentation-probe
# Leave the chart open:
./scripts/cargo.sh run --offline --features ui \
  --bin antitone-presentation-probe -- --interactive
```

Core Image provides an encoding/reference comparison, not interactive rendering. The host explicitly tags extended-sRGB `RGBA16Float` SDR/EDR=false after configuration. The chart checks f32/half transfer/readback, ordinary UI and two resizes. **Zero submitted frames fails the onscreen check.** Layer/readback evidence does not establish physical-monitor matching, monitor transitions, UI blending, forced surface recovery, macOS-14 or other-device performance.

## Documentation/cache work

Inspect local Markdown links/anchors and moved references, and record exactly which relocation checks ran. Do not reflexively rerun the full GPU/RAW/export suite for documentation-only changes. Authored source and selected small decisive evidence belong outside ignored cache; dependencies, generated fixtures/private images and third-party downloads stay local. [Cache policy/tool inventory](research/tools/README.md).
