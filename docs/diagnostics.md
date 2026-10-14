# Specialist diagnostics

Use [development checks](development.md) for the ordinary contributor loop. These recipes investigate specific native, numerical, image or presentation boundaries; they are not prerequisites for making every change.

Run from the repository root after the relevant [setup](../README.md#build-and-run). Examples use `--offline` with populated caches. RAW names refer to read-only local samples, not downloadable fixtures. Keep outputs under ignored `.cache/` and use fresh export/report directories.

A numerical comparison, a scripted visible window and physical photographic acceptance are different evidence. Missing hardware or zero submitted frames does not become a pass. Sample-dependent probes can be expensive; run the relevant recipe, not this entire page.

## Editor and layout checks

Build the editor and open a saved project for real-use inspection:

```sh
cargo build-app --offline
target/release/antitone /path/to/grade.antitone.json
```

Calibration, grade/source controls, native-pixel refinement, picker-source return and normal/narrow stage interaction use authored [framework UI tests](development.md#framework-ui-regressions), without private photographs or desktop capture permission. These offscreen checks do not establish native resize/presentation; [native-host](development.md#explicit-native-host-checks) and [macOS presentation](#macos-presentation) checks remain separate.

Authored [project UI regressions](development.md#framework-ui-regressions) cover Save/New, transition decisions, exact restored settings/pixels and failed-Save/Open preservation with real workers and temporary files. Native file dialogs and photographic Save/Open inspection remain separate real-use checks.

### Viewer-tool evidence

[Framework tool tests](development.md#framework-ui-regressions) render authored Crop/Dmin/WB and flat-field support states at normal/narrow sizes. Optional `ANTITONE_UI_CAPTURE` outputs are labelled offscreen evidence, not native-window appearance approval. [Explicit native-host checks](development.md#explicit-native-host-checks) separately exercise focus, size forwarding, surface paint and completion/close failures. Physical tool interaction and composed Editor-in-window appearance remain real-use checks; do not automate Lightroom/Resolve or bypass window occlusion.

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

For authored reference/area transitions, corrected picker measurements, both settled targets/thumbnails, Enabled/Clear and Save/Open:

```sh
cargo test --locked --offline --features ui-tests --lib \
  editor::kittest_tests::correction -- --test-threads=1
```

This is offscreen framework input, not native dialogs or physical reference acceptance. For a supplied negative/reference pair, use the normal editor to choose the reference, inspect support, re-pick calibration, compare both targets and Save/Open.

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

[Framework Analyze tests](development.md#roll-analysis) exercise authored picture-area transactions, confirmation/cancellation/failure, atomic initialization and settled rendering in both targets, including real correction. They are offscreen; [native-host completion checks](development.md#explicit-native-host-checks) are separate. Neither these nor the sample-dependent worker checks certify [photographic success](plans/roll-assistance.md) or composed Editor-in-window appearance. For other supplied rolls, use the explicit case manifest below or the normal editor; no scripted Analyze application route is maintained.

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
cargo test --locked --offline --features ui-tests --lib \
  scope_plot_scale_and_target_widgets_produce_valid_completed_reductions \
  -- --test-threads=1
```

The latency test times reductions, not end-to-end gestures. The authored framework test exercises [scope modes and amplification](scopes.md) through real widgets and completed GPU reductions in both targets, without a desktop window. Physical interpretation and native appearance remain separate.

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

The [authored RAW checks](development.md#feature-specific-checks) preserve Scan immutability, measured black/white anchors (`2e-12`), per-channel flooring rate (at most 1%) and sampled LibRaw matrix agreement (`2/65535`). The authored matrix case requires nonzero eligible comparisons; the supplied-source diagnostic still reports skips without treating an empty grid as agreement.

For the independent upstream OCIO → Metal chain:

```sh
# Synthetic AP1 inputs only; ignores camera artifacts already in the cache.
./scripts/probe-ocio.sh
# Regenerates authored RGB16/model-4 inputs, then includes them explicitly.
./scripts/probe-ocio.sh --camera
```

The camera extension uses the committed patch DNG and two explicit code-floor cases, not private photographs. Its CPU working values are implementation references; upstream OCIO supplies the independent rendering comparison. Missing or stale requested camera inputs fail. Both modes retain the existing finite/neutral/basis and `2e-5 + 2e-4*abs(reference)` agreement gates. They do not establish full-image or photographic acceptance.

For a separately supplied camera's numerical diagnostic:

```sh
cargo test --locked --offline --release --features raw,gpu \
  --test camera_raw -- --ignored --nocapture
```

For corrected affine reconstruction against scalar coverage:

```sh
cargo test --locked --offline --features editor --lib \
  --test scopes --test viewer --test affine_resample --test geometry_export
cargo test --locked --offline --release --features editor --lib \
  corrected_affine_refinement_matches_scalar_coverage -- --ignored --nocapture
```

## Rendering, gamut and ICC

The CPU rendering/ICC route regenerates its OCIO references itself; no preceding Metal check or cached input is required:

```sh
./scripts/probe-rendering.sh
# Optionally include freshly generated authored camera inputs; no Metal device is used.
./scripts/probe-rendering.sh --camera
cargo test --locked --offline --features render --test rendering
./scripts/probe-gamut.sh
# Independent target check without RAW fixtures:
cargo run --locked --offline --features render --bin antitone-rendering-probe -- --gamut-check
```

Both rendering scripts share `scripts/generate-ocio-reference.sh`, which regenerates the upstream CPU/GLSL/LUT artifacts. It can also run directly when only reference generation is needed, with optional `--camera`; it does not open a window or request a GPU device. Existing camera artifacts are ignored unless explicitly requested. A generation failure stops the dependent check rather than falling back to cached files.

CPU/GPU agreement, profile/transfer checks and independent ColorSync/Core Image comparisons address different boundaries. [Artifact](../assets/ocio/README.md) and [profile](../assets/profiles/README.md) guides retain provenance.

## Export

The ordinary [export tests](../tests/export.rs) check exact TIFF codes/profile/compression, signed/HDR AP0 EXR and destination safety. The [authored RAW tests](development.md#feature-specific-checks) additionally exercise actual decoding, calibration and model-4 processing before export.

ImageIO supplies a separate decoder boundary. Run its named opt-in check with:

```sh
cargo test --locked --offline --features raw,export --test raw_dng \
  authored_dng_tiffs_reopen_with_independent_imageio_storage_and_profiles \
  -- --ignored --nocapture
```

This compiles [the native checker](../native/tiff_reference.mm) with Apple Clang and creates all executable/export products in an isolated temporary directory. It reopens both targets from the authored DNG and an awkward-aspect synthetic image, checking dimensions, RGB16 decoding and exact pinned ICC bytes. It also exercises rejection of wrong dimensions/profiles, malformed or multi-image TIFFs and RGB8 storage. No private photograph or cached export is required.

ImageIO storage/profile agreement is not a pixel-rendering or photographic-quality oracle. Numerical tests remain responsible for pixels; large-file/resource and Lightroom/Resolve acceptance remain separate.

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
cargo run --locked --offline --features ui --bin antitone-presentation-probe
# Resize repeatedly while a bounded background worker submits on the same queue:
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --submission-stress
# Leave the chart open:
cargo run --locked --offline --features ui --bin antitone-presentation-probe -- --interactive
```

The submission stress route brings its own window forward and checks repeated native size changes, background-work completion and presented frames. It exercises the shared submission gate during repeated small resizes, not RAW refinement quality or prolonged physical use.

Zero submitted frames fails the onscreen check. Layer/encoding/readback agreement does not establish physical monitor matching, monitor transitions, actual macOS 14 or broader-device behavior. Keep those limits distinct from the tested [output boundary](colour-and-output.md).
