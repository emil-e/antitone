# Specialist diagnostics

Use [development checks](development.md) for the ordinary contributor loop. These recipes investigate specific native, numerical, image or presentation boundaries; they are not prerequisites for making every change.

Run from the repository root after the relevant [setup](../README.md#build-and-run). Examples use `--offline` with populated caches. RAW names refer to read-only local samples, not downloadable fixtures. Keep outputs under ignored `.cache/` and use fresh export/report directories.

A numerical comparison, a scripted visible window and physical photographic acceptance are different evidence. Missing hardware or zero submitted frames does not become a pass. Sample-dependent probes can be expensive; run the relevant recipe, not this entire page.

## Editor and layout smoke

Build the editor, then supply diagnostic calibration or open a saved project:

```sh
cargo build-app --offline
target/release/antitone /path/to/grade.antitone.json
ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

`--smoke` exercises the GPU/worker path. The environment flag adds normal/narrow stage-layout holds. Diagnostic anchors are not a user's saved or physically measured grade.

For scripted Save/Open, include an unvisited source:

```sh
target/release/antitone --smoke-project \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW samples/DSC04393.ARW
```

This uses temporary projects and checks restored settings/pixels and failed-Open preservation. It requires submitted visible frames, but ordinary/project smoke has no explicit completed-success result: early closure after a frame can return success before the scenario finishes. These routes drive editor state directly, not full widget input. [Verification restructuring](plans/verification-tooling.md) tracks those gaps; flat-field/Analyze routes reject early closure.

### Window capture

With an unlocked desktop and normal screen-capture permissions:

```sh
mkdir -p .cache/probes/controls
/usr/bin/clang -fobjc-arc native/controls_window_id.m \
  -framework Foundation -framework CoreGraphics \
  -o .cache/probes/controls-window-id
python3 scripts/capture-controls.py
python3 scripts/capture-viewer-tools.py --samples samples
# Reference-support layouts only:
python3 scripts/capture-viewer-tools.py --flat-only --samples samples \
  --output .cache/probes/flat-layout
```

Capture only the application window. Do not bypass occlusion or automate Lightroom/Resolve. Partial captures/timeouts are not completed route acceptance.

### Camera-roll fixtures

```sh
cargo test --locked --offline --features editor --lib \
  capture_camera_roll_review_fixtures -- --ignored --nocapture
```

The fixture writes synthetic GPU-painted cards to `.cache/probes/camera-roll`. It checks composed painting at normal/compact strip heights, not native presentation, photographs or VoiceOver.

## Flat-field correction

The headless editor/worker regression uses temporary projects and small corrected exports:

```sh
cargo test --locked --offline --release --all-features --lib \
  editor::flat::tests::real_reference_editor_save_open_and_corrected_exports \
  -- --ignored --nocapture
```

For visible reference/area transitions and both settled targets:

```sh
target/release/antitone --smoke-flat-field samples/DSC01737.ARW samples/DSC01739.ARW
target/release/antitone --smoke-flat-field samples/DSC01811.ARW samples/DSC01813.ARW
```

The full-size export/reopen exercise is more expensive:

```sh
cargo test --locked --offline --release --all-features --test flat_field_export \
  -- --ignored --nocapture
```

These check the implemented [correction](flat-field.md), not physical reference suitability or native-dialog gestures.

## Analyze roll

Run the [synthetic checks](development.md#roll-analysis) first. Explicit worker regressions use the two local twelve-frame scan sets:

```sh
ANTITONE_SAMPLES="$PWD/samples" cargo test --locked --offline --release \
  --features editor --lib editor::analysis::tests::real_ \
  -- --ignored --nocapture --test-threads=1
```

For picture-area acceptance, atomic analysis and both settled targets in a visible window:

```sh
target/release/antitone --smoke-analysis - \
  samples/DSC043{21,22,23,12,13,14,15,16,17,18,19,20}.ARW
# Replace '-' with a matching bare-light reference RAW for correction.
```

Neither recipe certifies [photographic success](plans/roll-assistance.md).

### Private all-roll contacts

The native helper creates ImageIO source previews and six-column contact sheets:

```sh
mkdir -p .cache/roll-check
/usr/bin/clang -fobjc-arc -Wall -Wextra -Werror native/image_contacts.m \
  -framework Foundation -framework ImageIO -framework CoreGraphics -framework CoreText \
  -o .cache/roll-check/image-contacts
.cache/roll-check/image-contacts previews .cache/roll-check/source /absolute/samples/*.ARW
.cache/roll-check/image-contacts sheet .cache/roll-check/source-contact.png .cache/roll-check/source/*.png
```

Source previews are not LibRaw or processing references. Group scan sets deliberately; frame counts do not establish physical roll identity.

Supply a private JSON case array, for example:

```json
[{"name":"roll-name","sources":["/absolute/photo.ARW"],"reference":null,"area":null}]
```

`sources` are ordered photos. Optional `reference` selects matching correction; optional `area` supplies `[x,y,width,height]` statistics geometry.

```sh
ANTITONE_ROLL_CASES=/absolute/cases.json \
ANTITONE_ROLL_OUTPUT="$PWD/.cache/roll-check/FRESH_OUTPUT" \
cargo test --locked --offline --release --features editor --lib diagnose_sample_rolls \
  -- --ignored --nocapture --test-threads=1
.cache/roll-check/image-contacts sheet .cache/roll-check/positive-contact.png \
  .cache/roll-check/FRESH_OUTPUT/roll-name/*.tif
```

The diagnostic drives actual import/correction/Analyze and renders approximate contacts. Completion is not acceptance: inspect failures and every image. [Implementation](../src/editor_analysis_diagnostics.rs) owns report fields.

## Responsive image scopes

```sh
cargo test --locked --offline --features editor --test scopes \
  scope_reduction_latency -- --ignored --nocapture
ANTITONE_SMOKE_SCOPES=1 ANTITONE_SMOKE_CONTROLS=1 target/release/antitone --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

The latency test times reductions, not end-to-end gestures. The window route exercises [scope modes and amplification](scopes.md); physical interpretation remains separate.

## Rendering/import performance probes

Run one ignored editor probe with:

```sh
cargo test --locked --offline --release --features editor --lib \
  selected_proxy_switching_latency -- --ignored --nocapture
```

Other filters are `visible_thumbnail_import_latency`, `import_switching_latency`, `import_and_refinement_latency`, `initial_import_with_renderer_prewarm` and `project_relative_locator_cold_thumbnail`. Their code defines measured phases and workload. Cache readiness, backend Ready and paint-command recording are not GPU completion or physical presentation; filesystem caching is uncontrolled.

`raw_byte_edits_reopen_without_identity_rejection_and_old_seed_keeps_its_revision` uses a **temporary RAW copy**, not an original sample. `embedded_jpeg_does_not_unpack_or_change_camera_development` checks sampled development independence from preview extraction. `development_and_region_picks` and `native_development_foreground_contention` check native diagnostics/priority; the latter requires the [native probe](native-development.md).

Runtime logs expose import, development, selected-preview/switch, thumbnail and refinement phases. Compare matching workloads and boundaries; a timing field is not necessarily a hardware timestamp. Worktrees use ordinary Cargo caches and may [reuse pinned native builds](native-development.md#reusing-native-builds).

## RAW and complete GPU chain

For a supplied-source metadata/development diagnostic:

```sh
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- --metadata /absolute/photo.ARW
cargo run --locked --offline --release --features raw \
  --bin antitone-raw-probe -- /absolute/photo.ARW
```

Full mode checks sampled repeatability, flooring rate and eligible in-sRGB LibRaw matrix comparisons. Inspect `compared_grid`: zero eligible comparisons does not fail and cannot establish matrix agreement. This is not photographic calibration; optional diagnostic PPMs are clipped source previews, not ACES/ICC output.

For native-camera checks and generated reference fixtures:

```sh
cargo test --locked --offline --release --features raw,gpu \
  --test camera_raw -- --ignored --nocapture
cargo run --locked --offline --features raw,gpu --bin antitone-calibration-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  --gpu-fixture samples/DSC04416.ARW
./scripts/probe-ocio.sh
```

Regenerate calibration fixtures before OCIO comparisons. These are regenerable RAW-derived fixtures, not full-image or photographic acceptance.

For corrected affine reconstruction against scalar coverage:

```sh
cargo test --locked --offline --features editor --lib \
  --test scopes --test viewer --test affine_resample --test geometry_export
cargo test --locked --offline --release --features editor --lib \
  corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

## Rendering, gamut and ICC

After regenerating the OCIO fixtures:

```sh
./scripts/probe-rendering.sh
cargo test --locked --offline --features render --test rendering
./scripts/probe-gamut.sh
# Independent target check without RAW fixtures:
cargo run --locked --offline --features render --bin antitone-rendering-probe -- --gamut-check
```

CPU/GPU agreement, profile/transfer checks and independent ColorSync/Core Image comparisons address different boundaries. [Artifact](../assets/ocio/README.md) and [profile](../assets/profiles/README.md) guides retain provenance.

## Export and cached sampling

```sh
cargo run --locked --offline --release --features raw,export --bin antitone-export-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW .cache/probes/NEW_OUTPUT_DIRECTORY
cargo run --locked --offline --release --features raw --bin antitone-scan-probe -- \
  samples/DSC04378.ARW 4000 2600 128 128
```

The export CLI uses diagnostic settings and full developed dimensions; editor exports use accepted composition. Large exports can consume substantial time/memory. For full TIFF/ImageIO reopening:

```sh
/usr/bin/clang++ -std=c++17 -O2 native/tiff_reference.mm \
  -framework Foundation -framework CoreGraphics -framework ImageIO \
  -o .cache/probes/tiff-reference
.cache/probes/tiff-reference .cache/probes/NEW_OUTPUT_DIRECTORY
```

The ImageIO helper checks the fixed diagnostic filenames/dimensions, 16-bit storage and ICC identity, not every pixel. Numerical pixel reopening has separate Rust tests; neither boundary proves photographic interoperability.

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
# Leave the chart open:
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --interactive
```

Zero submitted frames fails the onscreen check. Layer/encoding/readback agreement does not establish physical monitor matching, monitor transitions, actual macOS 14 or broader-device behavior. Keep those limits distinct from the tested [output boundary](colour-and-output.md).
