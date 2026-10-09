# Historical build and probe checkpoints

Archived accumulated runbook/checkpoints. Timings and test counts belong to different historical stages; commands may refer to the old documentation/tool layout. For supported reproduction use [the current runbook](../../build-and-probes.md); for ongoing work use [the roadmap](../../../PLAN.md). Numerical response evidence is retained separately in [the precision report](../film-response-precision-gate.md).

macOS/Apple Silicon only at present. A single-image editing prototype now accompanies the numerical probes; it is not a release-ready roll/project application. Samples are read-only inputs and never committed.

## Single-image editor

```sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ ./scripts/cargo.sh run --offline --release --features editor --bin antitone
# Optional initial source:
./scripts/cargo.sh run --offline --release --features editor --bin antitone -- samples/DSC04416.ARW
# Explicit diagnostic calibration, automatic real-image smoke test:
./scripts/cargo.sh run --offline --release --features editor --bin antitone -- --smoke \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 samples/DSC04416.ARW
```

Native dialogs use pinned `rfd 0.17.2`. One worker owns the selected full-resolution LibRaw source; adding sources does not develop the whole roll. The first successful source fixes the development WB tuple for this session. Dmin/Dmax pickers always start on the current image with no prior rectangle. References store only linear AP0/D60 RGB; no sampled source/path/rectangle/fingerprint is retained. Choose another reference RAW through the picker dialog or filmstrip, including a source outside the photo list. Applying, rejecting or cancelling returns to the working photo with its prior source/positive view, zoom/pan and edits. Click for a 32×32 full-resolution patch or drag a rectangle; move it, drag corner handles or edit bounded source coordinates before applying. Both references are required for positive editing/export. Four pickers, linked wheels/sliders/numeric entry, roll/photo exposure, source bypass and both TIFF/half/full EXR exports are wired. Photo edits survive navigation during this in-memory session. Pending overwrite confirmation disables underlying controls and is invalidated by navigation. Failed picks preserve processing state and restore the prior view.

Control sections, parameter rows and calibration/color inputs are shared in `src/editor_controls.rs`; see [editor control contract](../../contracts/editor-controls.md). Only Roll/Photo tabs are shown. Output remains pinned below scrollable edits. Exposure sliders span ±3 EV with 0.05-EV steps/Shift-fine adjustment; numeric entry retains ±16 EV and existing manual precision. Both RGB wheels use Resolve orientation (blue right/yellow left) without changing gamma/Bradford processing. Row and section resets have independent scopes. Disabled pending numeric edits cannot commit through focus loss.

**Unsaved prototype:** no project persistence/recovery, undo, reference-only flags, remove/relink workflow or app-level export cancellation yet. Closing while decode/picking/export is active is deferred; settled-preview work is disposable. Fit, physical-pixel 100%, pointer-centred wheel/pinch zoom, middle/space drag pan, previous/next arrows outside text editing, and Escape to cancel sampling are available. Export uses the active metadata image area (8640×5760 for these RAWs); the older diagnostic export CLI intentionally uses the full developed extent.

Interactive previews retain nearest-center LibRaw-developed camera RGB16 samples (8 bytes/pixel), with accurate camera→AP0 conversion in shader temporaries, GPU inversion/AP1 edits and the pinned generated OCIO shaders under `assets/ocio/`. There is no CPU interactive rendering/readback or approximate ACES substitute. After 250 ms without image-setting/view changes, a cancellable worker iterates **all full-resolution pixels in the visible source region** through the same GPU processing/rendering, then performs area-weighted averaging in **linear rendered target RGB**, before target-to-extended-sRGB conversion/encoding. This order deliberately does not invert averaged negative pixels. Superseded generations cannot replace the current preview. Only one settled job is outstanding; tile queue completion bounds staging-buffer accumulation.

The first settled smoke took 14.07 s because unchanged provisional images were being rerendered during every UI repaint. Parameter-keyed preparation removed that work: the latest 8640×5760 → 1024×683 sRGB settled preview takes **2.34 s**, including 0.20 s source sampling and 2.11 s GPU submission/completion, on M5 Pro. The complete three-source smoke takes 9.26 s and submits 364 frames. This is not M1 validation. Both source/positive viewer outputs match OCIO CPU references, including signed/HDR/floored inputs; half-float extended-sRGB presentation and fractional/tiled area averaging have independent scalar tests. Latest viewport smoke: active area 8640×5760 → 1598×1066 fit refinement **2.34 s**; visible 1598×1320 region → exactly 1598×1320 native-pixel refinement **0.17 s**. The complete calibration/fit/100%/reference-switch-and-return smoke takes 14.08 s and submits 520 frames. The source-coordinate picker transform is tested through zoom/pan and near exact pixel boundaries; GPU cropped presentation is tested against the corresponding source pixels. Current all-feature suite: **88 tests**. The updated P3 smoke completes in 14.18 s / 522 frames, with P3 fit refinement 2.375 s and native region refinement 0.161 s; see `p3-output-update.md`.


### RGB16 source-upload checkpoint

The float-AP0 input path was replaced, not retained as a compatibility shim. Preview and full-resolution tiles now upload the exact developed RGB16; source tiles use 2 MiB instead of 4 MiB. Rendered-only settled images allocate no source/working buffer or unused processing pipeline. f32 working/rendered textures and RGBA16Float presentation are unchanged. [Source format, precision and model compatibility](../../contracts/processing-model.md#camera-source-representation).

Checks: exact f64 camera conversion and original f32 source bits over 393,216 generated samples/three matrices; 3,072 actual camera samples and 4,816 complete-rendering pixels per target; all 72 tests, strict all-target Clippy, formatting and release build. The real-image M5 editor smoke passed in 13.64 s / 519 submitted frames, including both targets/source bypass, full-resolution calibration, fit/native-pixel refinement and picker source return. Fit refinement processed 8640×5760 → 1538×1026 P3 in 2.174 s (0.081 s sampling / 2.085 s GPU); native 1538×1320 refinement took 0.135 s. These are local checkpoints, not a controlled speed comparison to the older differently-sized view or M1 acceptance.

Logs: `.cache/probes/film-model-images/rgb16-{all-tests,clippy,release-build,conversion-tests,gpu-viewer-tests,camera-fixture,complete-gpu-chain,cached-scan,editor-smoke}.txt`. This preceding source-only checkpoint preserves model 2 and existing grades; the later response integration is recorded below. Two previously expanded original-model extreme-exposure discrepancies remain unresolved, not masked by new tolerances.

### Adjustable-response checkpoint

New editor/project grades use `RollSettings::new_grade()` with model 3 / toe 2.5 / shoulder off. Schema 3 remains; absent response/model 2 retains original scalar/GPU execution and metadata. Explicit **Use adjustable response** opt-in preserves other edits. Controls use Calibration → Gamma & balance → Toe & shoulder → Exposure; Photo uses White balance → Exposure. Scope resets, fine shoulder steps, invalid-pole/range rollback and pinned-output scrolling are tested. Export APIs receive actual response and persist model/shape without changing ACES, ICC or AP0 pixels. [Contract](../../contracts/processing-model.md#versioned-film-response).

**88 tests**, strict all-target Clippy, formatting/release and original real-camera chain pass. New production gate: 5,120 actual full-resolution camera samples × seven grades, both targets; exact floor flags, unchanged working tolerance, linear/encoded error <4.20e-6 and no GPU-black/CPU-nonblack samples. Model-3 beta 1/off resolves both candidate high-EV cancellation cases; preserved model-2 expanded-domain limits remain documented, not silently regraded. The 224-byte processing signature carries small precise constants; source stays 8 bytes/pixel and output precision is unchanged.

M5 normal smoke: 14.61 s / 567 frames. Four-hold layout smoke: 31.49 s / 1,576 frames; standalone P3 fit refinement 8640×5760 → 1538×1026 **2.572 s**, native region **0.164 s**. Normal/narrow screenshots inspected, plus event tests scroll the response/exposure rows into view while Output stays pinned. No M1/physical-monitor/macOS-14 acceptance claim. Logs/screenshots: `.cache/probes/film-model-images/response-*`, `film-wide-*`, `film-production-*`; [full numerical evidence](../film-response-precision-gate.md#production-film-response-checkpoint).

## Project-state foundation

`src/project.rs` is a versioned JSON state model, not yet connected to the Editor's Save/Open actions. It records stable source IDs, ordered membership, independent per-photo settings, fixed development gains/versions/spaces, source dimensions/matrix/floor, full-byte BLAKE3 fingerprints, output presets, and RGB-only `dmin`/`dmax` arrays. Schema 3 rejects older provenance-bearing records rather than migrating them silently. Calibration does not depend on source records; changing/removing/missing photos neither invalidates nor resamples measured RGB. Matching relink retains photo IDs; changed photo content still requires explicit acceptance. Development/basis/model compatibility is checked at roll level, not through a reference's former source association. Atomic New/Replace saves validate before writing, sync a same-directory temporary file and preserve the old document on failure. There is no complete power-loss/recovery claim.

Pinned serde 1.0.229, serde_json 1.0.151 (`float_roundtrip`) and BLAKE3 1.8.7 are project-feature dependencies. Standalone `--features project --test project` checks do not require RAW/GPU native libraries. Ten project tests cover original/new-response reopen and exact f64 shape/version identity, inverse-pole rejection, exact reopen/output agreement, RGB-only serialization and complete source-record removal, ordered removal/missing sources, changed bytes/explicit acceptance without calibration invalidation, matching relink, roll-level runtime compatibility, malformed identities/numerics/old schemas and atomic overwrite safety. UI integration, autosave/recovery and undo/redo remain unfinished.

## Toolchain and dependencies

Use `./scripts/cargo.sh`; it pins Rust **1.99.0**, the latest stable available when selected, under ignored `.cache/rustup`. It installs rustfmt/Clippy there if missing, disables rustup self-updates before bootstrap, and leaves the global default toolchain unchanged. Package/dependency MSRV is 1.95.

During the initial bootstrap, rustup automatically updated its shared manager from 1.29.0 to 1.29.1 before self-updates were disabled. Toolchains remain checkout-local; the global default remains 1.94.1. No Homebrew/system packages were installed.

Native sources/builds are also checkout-local:

```sh
./scripts/build-libraw.sh
./scripts/build-ocio.sh
./scripts/cargo.sh test --offline --all-features
./scripts/cargo.sh clippy --offline --all-features --all-targets -- -D warnings
```

First-time dependency downloads need network access. `LIBRAW_SOURCE` may override the LibRaw source root. The OCIO probe and `render` feature use `.cache/deps/ocio-install`; `OCIO_PREFIX` may override the bridge prefix. The native bridge currently embeds a checkout-local library rpath, not a distributable app-bundle dependency layout. Native linking currently prints a harmless ambient stale GCC search-path warning; compiler invocations are explicitly Apple Clang where required.

Pinned UI stack: egui/egui-winit/egui-wgpu 0.36.2, winit 0.30.13, wgpu/Naga 30.0.1. There is no eframe dependency, vendored backend, or duplicate wgpu stack. `src/platform/host.rs` owns the root window, event loop, surface, device/queue and repaint scheduling; public egui integrations handle widgets, input and rendering. Additional native viewports and host screenshot commands are not implemented.

## RAW and complete GPU-chain comparison

The following rectangles are diagnostic candidates, not saved or selected user calibration:

```sh
./scripts/cargo.sh run --offline --features raw,gpu \
  --bin antitone-calibration-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  --gpu-fixture samples/DSC04416.ARW
./scripts/probe-ocio.sh
```

The calibration probe measures full-resolution means and sensor diagnostics, checks the anchors, and writes camera-developed sample grids and CPU AP1 references under `.cache/probes`. OCIO adds those to its synthetic cases, then the GPU probe compares inversion/exposure/WB and complete ACES rendering for the current sRGB and P3 targets. The optional fixture is not a full-image pixel-by-pixel comparison. Regenerate OCIO artifacts after changing camera fixtures. Input fixtures are `camera-source-model2.rgb16` (four u16 slots/pixel) and `camera-to-ap0-model2.f64` (nine column-major f64 coefficients). Working/reference and parameter artifacts identify AP1/model 2. The GPU probe consumes only this integer source format; no old float-source artifact or fallback remains in its callers.

## CPU rendering and output profiles

After regenerating the OCIO artifacts above:

```sh
./scripts/probe-rendering.sh
CC=/usr/bin/clang CXX=/usr/bin/clang++ ./scripts/cargo.sh test --offline --features render --test rendering
```

The reusable `CpuRenderer` takes linear AP1/D60 buffers, uses the exact same target/group definition as the shader probe, and returns limited linear output RGB. Transfer encoding and 16-bit quantization are separate; EXR does not use this rendering path. Empty buffers, chunks and invalid/non-f32 inputs are checked; caller input is never mutated. The native OCIO processor is RAII-owned and C++ exceptions become Rust errors. A worker can construct its own processor; no cross-thread sharing contract is claimed yet.

Little CMS is compiled statically from `lcms2-sys 4.0.7` through `lcms2 6.2.0`, without a global library install. The build was checked to contain `liblcms2.a`, and `otool -L` shows no dynamic LCMS dependency. Do not supply `LCMS2_LIB_DIR`, which can override the sys crate's static build selection.

Production profile payloads are pinned in `../../../assets/profiles/`, with definitions/checksums in its README. They are original generated ICC v4.3 matrix-shaper profiles, not copied system profiles. Tests validate every 16-bit encoded value, monotonicity, the sRGB transfer toe for both targets, serialized colorants, PCS adaptation, and float RGB-to-XYZ conversion. The rendering probe matches 4,816 scalar reference pixels exactly, writes these pinned payloads and color fixtures into the ignored cache, then independently checks them with macOS ColorSync/Core Image. These are encoding/profile checks, not actual TIFF files, Lightroom or physical-monitor checks.

**Current gamut gates pass:** `./scripts/probe-gamut.sh` checks the active sRGB/P3 pair over 990 colored samples per target; the rendering probe `-- --gamut-check` also checks an ordinary yellow/neutral without RAWs or fixtures. CPU and Metal regressions separately check dense yellow/exposure ramps, continuity and preservation of colors outside sRGB. See [P3 update/evidence](../p3-output-update.md).

Historical [ProPhoto](../prophoto-gamut-diagnosis.md) and [100-nit SDR Rec.2020](../rec2020-rendering-gate.md) failures motivated the explicitly approved P3 replacement. `./scripts/probe-gamut.sh --compare-rec2020` retains the failed Rec.2020 diagnostic plus the 500-nit control and intentionally exits 1. No ACES math patch or unapproved fallback was applied.

## File exports and cached sampling

```sh
./scripts/cargo.sh run --offline --release --features raw,export --bin antitone-export-probe -- \
  samples/DSC04397.ARW 4000 5644 128 32 \
  samples/DSC04378.ARW 4000 2600 128 128 \
  samples/DSC04416.ARW .cache/probes/NEW_OUTPUT_DIRECTORY
./scripts/cargo.sh run --offline --release --features raw --bin antitone-scan-probe -- \
  samples/DSC04378.ARW 4000 2600 128 128
```

The export CLI uses explicit diagnostic reference rectangles and default edits, exports the full developed extent, and never overwrites an existing destination. Use a new ignored output directory for another run. It produces both 16-bit TIFF targets plus default half-float scene-linear AP0 EXR. The public module additionally supports full-float EXR and explicitly confirmed replacement. This CLI is not the final app's calibration selection or batch workflow.

TIFF uses the pinned ICC in an UNDEFINED tag and lossless LZW/prediction. The crate's compressed API requires a whole-image RGB16 buffer; processing is chunked but the code buffer is retained. EXR retains three half/full planes and uses single-thread ZIP16 encoding. Each sample buffer is capped at 1 GiB (the supplied Sony image requires ~306 MiB half/RGB16 or ~612 MiB full). Aggregate source/cache/runtime bounds still need measurement on M1 Pro.

Numerical tests reopen compressed TIFF and EXR, check stored codes, signed/HDR AP0 conversion, channel precision and metadata, and exercise overflow/cancellation/no-overwrite/confirmed replacement. Temporary files are in the destination directory; macOS exclusive rename prevents a competing file being overwritten. Failures leave no final partial file. This is not cross-platform rename support or full crash/power-loss durability.

For an independent Apple decoder check of the existing `DSC04416` fixture:

```sh
/usr/bin/clang++ -std=c++17 -O2 native/tiff_reference.mm \
  -framework Foundation -framework CoreGraphics -framework ImageIO \
  -o .cache/probes/tiff-reference
.cache/probes/tiff-reference .cache/probes/export-p3-1
```

The fresh `.cache/probes/export-p3-1` diagnostic contains Display P3/sRGB TIFFs and half EXR. ImageIO decodes the full 8704×6144 16-bit files and returns the exact pinned ICC bytes for the new targets. Lightroom/Resolve interoperability and full source metadata propagation remain pending.

`Scan` owns the cached native camera development; repeated full-resolution region picks and preview-grid generation do not redecode. Clipped/floored patch rejection returns an explanation without altering corrections. The grid is provisional nearest-center sampling. The editor's separate full-source GPU refinement processes/renders before area averaging; grid generation alone is not a settled preview or a proof that nonlinear processing commutes with resampling.

## macOS presentation

```sh
./scripts/probe-presentation-reference.sh
./scripts/cargo.sh run --offline --features ui --bin antitone-presentation-probe
# Leave the chart open instead of automatically closing:
./scripts/cargo.sh run --offline --features ui --bin antitone-presentation-probe -- --interactive
```

The reference utility uses Core Image as a color-encoding reference, not as the interactive renderer. The GPU probe checks the signed extended-sRGB curve in RGBA32Float and RGBA16Float, then checks ordinary opaque egui UI encoding. The window uses RGBA16Float with explicit `kCGColorSpaceExtendedSRGB` and EDR disabled. Every configuration reapplies native properties after wgpu can clear them; no application monitor-profile conversion is applied.

The automatic chart requests two resizes, processes close commands even when no drawable is available, and reports actual submitted frame count. **Zero frames is an onscreen-check failure**, even if the offscreen reference tests and layer declarations pass. Locked-session runs were occluded with `CGSSessionScreenIsLocked=true`. After the user unlocked the desktop, the probe submitted **11 frames / 12 UI passes**, including both requested resizes (`.cache/probes/presentation-unlocked.txt`); the real-image editor also presents successfully. Never bypass occlusion to report success. Do not treat native tagging/readback as physical monitor matching, monitor-transition validation or M1 Pro performance evidence. Explicit sRGB baseline, UI blending and forced lost-surface recovery are still pending.

The old system-ROMM inspection was removed from the active presentation reference when ProPhoto was retired; its historical diagnostic remains in the ignored cache. Current sRGB/Display P3 profile conversion is independently checked by `scripts/probe-rendering.sh`, separate from the extended-sRGB carrier/encoding test.

Current evidence and remaining release gates are tracked in `../../../PLAN.md`. Tests were run on an M5 Pro/48 GiB machine, not the M1 Pro/16 GiB baseline.
