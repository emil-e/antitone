# Antitone — historical product plan and UX handoff

Archived product/architecture proposal with later amendments. **Not the current contract or execution queue.** Reference provenance, stage layout, schema statements and outstanding gates below include superseded material. Use [architecture](../../architecture.md), [contracts](../../README.md#current-contracts) and [roadmap](../../../PLAN.md) for current work. The companion mockup is illustrative history, not implemented UI.

Status: implementation in progress, 8 October 2026. The single-image prototype now opens RAWs, samples four full-resolution corrections, presents GPU edits and full-source-derived settled previews, and exports TIFF/EXR. Numerical/probe evidence is recorded in `docs/research/history/execution-plan.md`; physical monitor matching, Lightroom/Resolve interoperability and durable roll/project workflow remain pending. Companion: `antitone-ux-mockup.html`. Output policy update: explicitly approved **sRGB + Display P3** now replaces ProPhoto after the ProPhoto/SDR Rec.2020 yellow-collapse gates; see [implementation and evidence](../p3-output-update.md). Other feedback amendments are recorded in `feedback-plan.md`.

## 1. Product goal

A focused desktop application for converting camera-scanned film negatives into useful, approximately scene-linear positive images. Calibrate the roll manually, make small photographic adjustments per image, and export for finishing elsewhere.

Implement and test the explicit inversion contract below; matching the Resolve prototype is not an acceptance requirement. Its development/rendering may contain unintended processing. The application should make its workflow explicit and repeatable, without opaque per-image adaptive inversion. It is not intended to reconstruct exact film dye concentrations or original scene radiometry.

Initial platform: macOS on Apple Silicon. Windows and other platforms are later targets. Keep processing and UI portable where practical, without sacrificing correct macOS presentation. A roll is a project, containing an explicitly selected, ordered set of files rather than an automatically watched folder.

Initial supported camera: Sony ILCE-1M2, user-described lossless-compressed ARW. Supporting this camera first is sufficient, but do not introduce camera-specific hacks where a general development route works. The first validation roll is `samples/DSC04378.ARW` through `samples/DSC04416.ARW` (39 files); the first contains a leader, and film rebate provides base regions. Assume consistent capture settings. EXIF inspection confirms ISO 100, 1/10-second exposure, manual WB mode, orientation 1, and zero exposure compensation throughout that range. Aperture and focal length are recorded as zero/unknown, not verified. LibRaw probes confirm 14-bit lossless JPEG decoding and slightly different as-shot WB gains; development uses one roll-fixed tuple. Numerical evidence is recorded in `docs/research/history/execution-plan.md`.

This is primarily a personal tool, with possible distribution later. Retain dependency notices and review packaging obligations before distribution; elaborate commercial packaging is not initial scope.

## 2. Agreed scope

- Open a selected set of RAW photographs; add or remove individual files later.
- Main image viewer, bottom filmstrip, and a sidebar with **Roll pre / Photo / Roll post** stages.
- Sample unexposed film base and a fully exposed leader. References may be in different source images. Store the measurements and their provenance.
- Roll controls: global gamma, gamma white balance, and post-inversion exposure.
- Per-photo controls: exposure and white balance.
- Fit, 100%, zoom, pan, and previous/next image navigation.
- ACES 2.0 output rendering for the preview and rendered TIFF export.
- Scene-linear EXR export and ICC-tagged rendered TIFF export.
- Save/reopen a project with repeatable numerical processing.

Reserve extension points, but do not implement initially: selectable inversion spaces, flat-field correction, vectorscope. The supplied scans have illumination/lens vignetting; future flat-field correction is a known requirement, not justification for silently estimating correction now. Roll post is identity initially. Contrast/pivot, crop, and rotation are deferred beyond the first usable build. Dust removal, catalog management, arbitrary node graphs, and custom GPU RAW development are out of scope. A platform RAW framework's internal acceleration is not a request to implement GPU demosaicing.

## 3. Fixed processing pipeline

1. Decode and develop RAW with controlled, recorded settings.
2. Convert to the selected linear inversion basis (initially AP0/D60).
3. **Roll pre:** apply base/leader-referenced inversion, gamma balance and global gamma; then roll exposure.
4. Convert the inversion result to the photographic working space: linear AP1/ACEScg, D60.
5. **Photo:** exposure and positive-image white balance in AP1.
6. **Roll post:** identity, with a reserved stage boundary.
7. Fork into AP0 scene-linear EXR conversion or display rendering.
8. Preview: the selected TIFF target's ACES 2.0 rendering from AP1 working RGB, conversion to the declared presentation space, then OS monitor conversion. Do not apply a second ACES rendering for preview.

Keep inversion basis, photographic working space and output/file encoding separate. Develop into linear RGB with a documented definition, then convert to the initial AP0 inversion basis with explicit primaries and white-point adaptation. Camera-native RGB alone is not a standard color-space definition. AP0 is a practical initial inversion basis, not a claim that its channels correspond to film layers. A configurable inversion basis remains a separate deferred feature; selecting one must not implicitly select the photographic working space.

Use AP1 for post-inversion photographic processing and convert to AP0 at the EXR boundary. AP0 and AP1 share D60, so their conversion requires no chromatic adaptation. Preserve negative/out-of-gamut coordinates and values above one in both spaces; never clamp at the AP1 working-space conversion. Keep conversion into the inversion basis and conversion out to the working space explicit around the nonlinear inversion. Do not introduce an intermediate sRGB gamut clamp or an undocumented pre-inversion clamp.

The sidebar names identify scopes, not a user-editable node graph. Roll exposure follows inversion even though it lives in the Roll pre panel. Photo exposure and roll exposure combine additively in stops while adjacent in the linear pipeline.

## 4. Inversion contract

For channel c, define:

- `s[c]`: measured linear scan value.
- `b[c]`: linear scan value of unexposed film base (high transmission).
- `w[c]`: linear scan value of the saturated leader (low transmission).
- `gamma[c]`: positive film-response slope parameter; the exponent is its reciprocal.

```text
positive[c] = ((b[c] / s[c]) ** (1 / gamma[c]) - 1)
            / ((b[c] / w[c]) ** (1 / gamma[c]) - 1)
```

Equivalently, with density D and density endpoints Dmin and Dmax:

```text
positive[c] = (10 ** ((D[c] - Dmin[c]) / gamma[c]) - 1)
            / (10 ** ((Dmax[c] - Dmin[c]) / gamma[c]) - 1)
```

This maps the sampled base to 0 and the sampled leader to 1. Leader = 1 is an exposure reference, not a clipping ceiling or a promise of display white. Exposure can put that reference anywhere on the output transform. A saturated leader has lost scene chromaticity; treating it as neutral is an intentional convention.

Changing gamma must recompute the denominator, preserving both anchors. Do not substitute a simple power on an already normalized positive, and do not use `1 - scan`.

Gamma balance parameterization:

```text
gamma[c] = global_gamma * exp(balance[c])
sum(balance) = 0
```

Two independent balance coordinates separate relative channel slopes from global gamma. Specify and version the mapping between the UI's two axes and these three values. Use the actual numeric channel gammas for diagnosis. This is not the same operation as white balance on the positive image.

Apply exposure as `positive *= 2 ** EV`. Keep unclipped floating-point values throughout photographic processing. Validate `b[c] > w[c] > 0`, finite samples, positive gamma, and a denominator sufficiently far from zero. Define and test handling of zero/negative scan values and values above the base; do not hide invalid calibrations with arbitrary clamps. A small input floor may be necessary for numerical safety but must be documented and distinguishable from recoverable image data. Preserve values above 1.

Negative RGB components indicate coordinates outside a space's nonnegative RGB cone, but may also arise from black-subtracted noise, demosaic ringing, or approximate camera matrices. High-CRI illumination does not guarantee positive developed AP0 coordinates. Distinguish nonpositive scan inputs (outside the inversion formula's domain) from negative positive-image outputs when `s > b` (valid results). Clamping a scan to exactly zero does not resolve division by zero. Agreed policy, with the floor magnitude to be established by the probe: reject invalid reference measurements; apply a documented strictly positive numerical floor to nonpositive/too-small per-pixel scan values, report affected counts, and preserve negative post-inversion values. Choose the floor in recorded scan units based on measured noise/precision, not an unexplained constant. If materially many image pixels or reference regions fall outside the domain, investigate development/conversion before accepting flooring as a solution.

Use a numerically stable implementation (`expm1`-equivalent near zero) where cancellation matters. Specify absolute/relative tolerances against a double-precision scalar reference before optimizing WGSL.

## 5. Calibration and photographic controls

### Roll calibration

Provide four explicitly named pickers: base, leader, neutralize roll gamma, and neutralize photo WB. Sampling uses a click with a visible adjustable patch, with dragged rectangles for larger regions. Regions are stored in source-image coordinates. The viewer switches to the chosen source for base/leader sampling; applying or cancelling returns to the working photo. Base/leader sampling averages full-resolution linear scan values before nonlinear inversion, not preview/display pixels. Persist source identity/fingerprint, region, numerical RGB, development configuration, and calibration model version.

Positive preview and processed export require both valid references; otherwise show the developed source negative and explain what is missing. Do not invent provisional calibration. Source view is always available as an explicit inversion bypass, even after calibration, without changing processing state. Convert the developed source to AP1 for the same rendering entry point, bypassing photographic edits. Allow resampling without losing other roll edits. Changed development settings invalidate measurements and must not silently reuse incompatible calibration.

Calibration measurements own their stored values/provenance independently of filmstrip membership. Removing a sampled source from the project is normal and does not clear or invalidate the measurement, warn about the removal, or force relinking for normal processing. The source is needed again only to resample. Actual changed source content is distinct from removing a filmstrip entry; detect fingerprint mismatches and do not silently reinterpret them.

Use consistent scan/development settings across the roll. No automatic per-image exposure normalization, contrast, or creative look in RAW development. Record clipping information when available; a comfortably centered histogram does not prove all channels are unclipped. Agreed development WB policy: initialize from one selected source, then choose and record one roll-fixed set of gains rather than independently using each file's as-shot WB. Do not let file-order or decoder defaults silently change it after calibration.

A single base sample cannot correct spatial vignetting. Sampling darker rebate can bias the entire inversion. Surface this limitation and let the user choose reference regions deliberately; do not silently compensate. Reserve future flat-field correction before calibration measurements and inversion, with invalidation of measurements if the correction changes.

### Gamma white balance

Use a two-dimensional correction wheel plus global gamma. Numerical coordinates and expandable opponent sliders expose the same two balance values, not separate adjustments. Document/version the mapping into zero-sum channel log-gamma offsets and show the resulting channel gammas for diagnosis. Wheel artwork indicates correction directions; it is not a vectorscope or automatic neutral detector.

The roll-neutral picker fits two gamma-balance coordinates from one full-resolution patch, preserving global gamma, exposure, and the base/leader anchors. Evaluate the roll-inversion stage independently of photo WB. Do not use final rendered pixels or a photo-specific correction to fit the shared roll. Validate the fit against the full-resolution patch's mean positive RGB; do not assume nonlinear inversion of its mean scan is the same statistic. Near the anchors, gamma balance is underdetermined. Reject unusable, clipped, ambiguous, or unsolvable samples with an explanation and leave controls unchanged. A successful pick replaces the existing balance as one undo operation; there is no strength control, persistent fitting constraint, or multi-patch fitting in the first version.

### Per-photo white balance

Operate on the positive scene-linear result with a documented chromatic-adaptation model, independently from roll gamma balance. Offer a two-dimensional correction wheel and warmth/tint sliders/numerical coordinates for creative control over the same WB state. Picking a neutral moves those controls to the solved values; it does not create a hidden neutral baseline and reset the displayed values to zero.

The photo-neutral picker uses one full-resolution positive-image AP1 patch after the inversion-to-working conversion, before the existing photo WB and before ACES rendering. Only the selected photo's WB changes; exposure and shared roll parameters remain unchanged. A successful pick replaces the correction as one undo operation, after which wheel/sliders can refine it. Reject unusable samples with an explanation and no state change.

Processing model 2: Bradford cone-space adaptation in AP1/D60, with versioned opponent coordinates and explicit gain normalization. Photo picking validates cone responses rather than rejecting a patch merely for negative AP1 coordinates. Do not label warmth values as physical Kelvin without a defined mapping. No automatic neutral-region detection.

## 6. UX specification

- Top bar: project name, Open/Add photos, Save, Export.
- Viewer toolbar: selected filename, explicit positive/source-negative toggle, fit/100% and zoom controls. Source view bypasses inversion/photographic edits, not RAW development.
- Main viewer: image on a quiet neutral surround, with pan/zoom and visible sampling state.
- Bottom filmstrip: explicit file order, selected image, reference-only entries clearly marked. References can remain part of the project without becoming exported photographs.
- Sidebar: three persistent stage tabs. The active panel states either “All photos in this roll” or the selected filename.
- Roll pre: base and leader reference rows, global gamma, gamma-balance wheel/expandable sliders, roll-neutral picker, and roll exposure.
- Photo: exposure, WB wheel/warmth-tint sliders and photo-neutral picker. Values follow the selected photo.
- Roll post: empty identity state; no placeholder adjustment sliders.
- Bottom status: output rendering preset and actual monitor-management status. Do not claim OS management based only on OS version.
- Export: selected/all photos, EXR or TIFF, destination, overwrite handling, progress and cancellation. Reference-only images excluded by default.

Keyboard: left/right change photo unless editing a field; fit/100% shortcuts and Escape to cancel sampling. Sliders support numerical entry and reset-to-default. Undo/redo distinguishes shared edits from photo edits; a slider drag is one undo operation. Zoom and pan do not dirty the project unless deliberately persisted as session state.

The companion mockup explores layout and scope. It uses synthetic artwork, local state, and approximate preview adjustments. It is not an inversion implementation, RAW decoder, ACES reference, or monitor-management test. Its file and export dialogs are demonstrations.

## 7. Rendering and monitor color management

Use **ACES 2.0**, pinned OCIO implementation/transform definitions, and a 100-nit SDR rendering convention. Current TIFF/preview targets are Display P3/D65 and sRGB/D65; rendering/limiting and encoding gamut match for each target. ProPhoto and the SDR Rec.2020 candidate failed color-preservation gates, after which the user explicitly approved P3; no ACES mathematics were patched. Keep output rendering independent from AP0 inversion.

The viewer follows the selected TIFF rendering target, then converts the rendered image to a declared presentation space for OS monitor conversion. This is not an independent sRGB-limited ACES rendering of a wider-gamut export. The presentation path must preserve colors outside sRGB where the monitor can display them, using a correctly declared wide-gamut/extended-linear surface or equivalent managed layer. Wide-gamut SDR and HDR creative output are distinct requirements. If using a P3/extended-linear surface, convert ordinary sRGB UI content correctly as well. Validate UI blending and image output separately; do not assume generic UI-host surface defaults meet this contract.

First release must have properly managed macOS SDR presentation. Declare the actual rendered content color space on `CAMetalLayer` and let Core Animation/ColorSync perform monitor conversion. An explicit sRGB declaration is the baseline presentation probe, not a complete wide-gamut-preview solution. An sRGB GPU texture format does not establish this contract. Important released-source finding: wgpu 30.0.1 maps `SurfaceColorSpace::Srgb` to `CAMetalLayer.colorspace = nil`; its source calls this sRGB default behavior, but Apple's API documentation says `nil` means the content is not color-matched. Do not claim managed output from that default. Probe an explicit `kCGColorSpaceSRGB` declaration; use the narrow native adapter after every surface configuration. The released Metal HAL exposes `Surface::render_layer()` and wgpu exposes unsafe `Surface::as_hal()`. The application now owns its winit/wgpu surface and uses unmodified egui renderer/input APIs; no eframe surface-access patch is retained. The probe establishes native tagging and transfer/readback, not completed physical monitor validation. Do not build a general presentation framework before this probe.

egui-wgpu 0.36.2 normally uses an ordinary UNORM attachment and emits gamma-encoded UI values. For the sRGB baseline, a custom image callback must emit encoded sRGB for that target, or linear values if the attachment itself applies sRGB encoding. A wide-gamut surface requires the corresponding image and UI color conversions, not reuse of sRGB encoded values under a different tag. Test that encoding occurs exactly once. Do not feed linear working AP1 or inversion AP0 directly into egui's ordinary image-texture path.

Verify known patches against a ColorSync-managed reference on actual hardware, including moving the window between monitors. Never apply an application monitor-profile conversion and OS conversion to the same pixels. A correct declaration establishes the intended managed path, not proof that the monitor itself is physically calibrated to 100 nits.

Later platforms must eventually be color managed. Until a platform's path is implemented/verified, label it unmanaged or unverified, as appropriate. On Windows, automatic color management can manage ordinary sRGB integer surfaces when Advanced Color is active; FP16/HDR is not required just to present managed sRGB. Detection and multi-monitor behavior need a separate platform probe. Application ICC conversion is an alternative when OS management is unavailable, not initial macOS scope. HDR display output remains a separate future feature.

## 8. Export contract

**EXR:** RGB scene-linear AP0, converted from processed AP1 working RGB at export, including inversion and photographic edits, excluding ACES output transform and monitor conversion. Do not simply tag AP1 pixels as AP0. Default half-float, optionally full-float. Refuse non-finite/half-overflow output and offer full-float where it resolves the range problem; never silently clip or change precision. Write chromaticities and documented metadata identifying AP0/D60, processing version and exposure convention. Do not claim full ACES container compliance merely because the pixels are AP0.

**TIFF:** 16-bit RGB with ACES 2.0 rendering, intended for finishing in Lightroom. Current choices: Display P3/D65 (default wide option) and sRGB/D65. Match rendering/limiting gamut to encoding gamut and embed the corresponding original pinned ICC profile. Both transfers use the standard segmented sRGB curve; Display P3 is not cinema P3 gamma 2.6. ProPhoto was retired after validation failures, not retained as a container for P3 rendering. Never embed the user's monitor profile. Serialized parameters, colorants, PCS adaptation, ColorSync conversion, TIFF embedding and independent ImageIO reopening pass software checks; dense yellow/exposure regressions verify color preservation beyond CPU/GPU consistency. Output contract v2 and project schema 2 reject obsolete ProPhoto state rather than reinterpreting it. Lightroom/Resolve interoperability and source metadata propagation remain pending. Use tested lossless compression. Quantization follows rendering/encoding; preserve ACES's specified gamut/white limiting operations.

Export is recomputed from full-resolution source data, independent of preview resolution. Batch export uses a consistent snapshot of project settings. Default filenames use the source basename with the selected output extension. Write temporary outputs then rename atomically; ask before overwriting. Detect destination-name collisions. Report per-photo failures and continue unaffected photographs. Support cancellation between units of work. EXR is for further scene-referred processing, such as in Resolve, and never includes the TIFF rendering.

## 9. Proposed technology stack

| Area | Initial choice | Reason / boundary |
|---|---|---|
| Language | Rust | Processing, state, workers and platform integration; checkout-local Rust 1.99.0 selected as latest stable |
| Desktop UI | egui with an owned winit/wgpu host | Public egui-winit/egui-wgpu 0.36.2 integrations; winit 0.30.13; direct surface ownership without an eframe/backend fork |
| GPU | wgpu, Metal first | Interactive inversion/edits and OCIO-generated GPU rendering; one shared device/queue; WGSL for owned kernels, GLSL/Naga input for generated ACES |
| RAW | LibRaw behind a controlled wrapper | Reuse existing black subtraction/scaling/demosaic, then float color conversion; no custom developer or camera hacks |
| EXR | `exr` crate | Direct control of float pixels and metadata |
| TIFF | `tiff` crate | Explicit encoding, tags, compression and ICC payload |
| Thumbnails | `image` crate as needed | Auxiliary IO, not the authoritative export contract |
| Project | `serde` + `serde_json` | Readable, versioned, inspectable state |
| Display transform | OCIO CPU export/reference + generated GPU preview (option B) | Reusable owned CPU bridge and pinned GPU artifacts share native definitions; actual onscreen presentation now passes, hardware matching remains pending |
| Output profiles | Pinned original ICC payloads; Little CMS validation | `lcms2 6.2.0` / bundled static `lcms2-sys 4.0.7`; matching transfer, primaries and PCS adaptation |

Pin mutually compatible egui/egui-winit/egui-wgpu/winit/wgpu releases rather than accidentally creating duplicate wgpu stacks. The host owns native surface configuration and reconfiguration; egui remains the widget layer. Review dependency licenses, notably LibRaw's selected license/build and any color-management integration, before distribution.

### RAW development options and probe

The requirement is known-space linear RGB, not float arithmetic at every internal RAW stage. Reuse an existing decoder/demosaicer where it meets the numerical contract. Do not write a new RAW developer just to avoid integer intermediates.

- **LibRaw, agreed route subject to numerical validation:** use its black subtraction, fixed WB/scaling and demosaic; request camera-RGB output (`output_color = 0`), no automatic brightness/maximum adjustment, no tone curve, and no highlight reconstruction. Read the developed integer buffer before output curve encoding, promote to float, and apply the documented camera matrix and standard-space/AP0 conversion in float. Verify the WB/matrix convention and record normalization. This owns a small matrix/development wrapper, not demosaicing. Source inspection confirms `convert_to_rgb_loop()` clips matrix results into unsigned 16-bit; bypassing that standard-output conversion avoids hidden gamut clipping there. Internal scale/demosaic/black handling still uses bounded integers and must be measured. `highlight = 1` normalizes scaling by the largest WB gain rather than clipping weaker channels to white, but is not proof that all upstream values survive.
- **Investigated alternative, not selected: LibRaw standard linear wide-gamut output:** least wrapper code, using linear gamma and a documented output matrix. Accept only if the probe establishes that integer matrix clipping and quantization are immaterial for the intended scans; do not substitute a narrow-gamut intermediate merely because it is convenient. LibRaw's `ProPhoto D65` label is not interchangeable with standard D50 ProPhoto without checking the matrix.
- **Investigated alternative, not selected: Apple CIRAWFilter:** render RGBA float into explicitly declared extended-linear sRGB (D65), then convert/adapt to AP0. Disable baseline exposure, global/shadow boost, local tone mapping, gamut mapping, sharpening/noise/lens adjustments where supported; fix WB and decoder version. A local metadata-only diagnostic on macOS 26.6.2 successfully constructed filters for `DSC04378` and `DSC04416`, reports Sony ILCE-1M2 support, 8640 × 5760 pixels, and decoder versions 7/8. Defaults include baseline exposure 0.35, boost 1, shadow boost 0.9, and gamut mapping enabled; these are not suitable unchanged. Initial neutral coordinates differ slightly between those files, so do not assume manual WB mode guarantees identical development WB. No float render or linearity/clipping test has yet run. Apple documents decoder-version selection for appearance compatibility, but not bit-identical results across OS/compiler/hardware. Choosing this route relaxes the strongest cross-platform development reproducibility goal.

Probe black subtraction, saturation/headroom, WB, exact camera-to-standard-space conversion, demosaic behavior, clipping/quantization and repeated output on the actual Sony samples. Use general settings rather than camera-specific repairs. Only choose a larger/custom developer if these existing routes demonstrably fail the contract.

### ACES integration options

Use the official ACES package `v2.0.0+2025.04.04` and its pinned component SHAs as the CTL reference, or a separately pinned OCIO implementation/config checked against it. ACES 2 packages live in the aggregate `aces` repository; the historical `aces-core` tags alone are not evidence that ACES 2 has no reference release.

**Agreed option B: OCIO CPU reference/export plus generated GPU interactive preview.** Use a narrow C++ bridge and pinned transform definitions. A CPU-only interactive viewer is not the requested product. wgpu's native Rust API accepts GLSL, SPIR-V and Naga IR in addition to WGSL; no standalone WGSL translation is inherently required. Probe OCIO Vulkan GLSL 4.6 through Naga/wgpu on Metal, including combined-sampler adaptation, bindings, texture precision/interpolation, shader entry-point glue and numerical agreement. OCIO can generate its internal 1D tables as 2D textures; prefer that documented option to handwritten algorithm changes if needed. Build-time frozen shader artifacts can reduce runtime generation complexity.

First prove actual shader/pipeline compilation, CPU/GPU agreement, color-preserving SDR target rendering and managed presentation. If integration requires extensive shader rewriting or renderer forks, discuss direct Metal (OCIO supports generated MSL) or another stack before proceeding. Do not silently switch to CPU preview, a baked LUT, or option C. A standalone Rust/WGSL ACES 2 reference port remains a higher-maintenance alternative, not agreed implementation scope.

A baked LUT is an optional measured preview approximation, not a silent replacement. Specify a signed-domain shaper, finite domain, interpolation/extrapolation policy and measured error over the intended HDR range. Check CPU/GPU implementations against the authoritative reference; do not substitute a generic “ACES-like” tone curve.

## 10. Project and runtime architecture

Proposed project extension: `.antitone.json`; source files remain external and untouched. Use stable photo IDs, ordered photo records, relative paths where practical, and a missing-file relink flow. Store:

- schema and processing model versions;
- source paths/fingerprints and reference-only flags;
- RAW backend/decoder version, development settings, matrix/normalization convention and working/inversion space IDs;
- base/leader sample provenance and measured values;
- roll gamma, gamma balance and exposure;
- per-photo exposure and white balance;
- output transform version/preset and export defaults.

Use atomic explicit saves and separate recoverable autosaves; recovery must not silently overwrite the saved project. Derived thumbnails, decoded tiles and GPU textures are disposable caches outside the project. Keep UI state separate from processing state. Invalidate caches by source/development/model/settings dependencies.

Workers handle RAW decode/development, full-resolution export and IO; interactive GPU processing shares the viewer's device/queue. Parameter adjustments must reuse decoded data rather than redevelop RAW. Bound worker concurrency and memory, cancel obsolete work, and reject stale results after switching photos or changing parameters. Prioritize the selected image and adjacent thumbnails. Avoid continuously repainting an idle app.

Accepted preview policy: responsive approximate previews during interaction, then full-resolution-derived settled previews, subject to measured latency and memory budgets. Inversion of downsampled negative data is not equivalent to downsampling inverted pixels. Label approximate/refining states, and never use preview pixels for calibration/export. The exact resampling/rendering order for the settled view must be defined and compared against full-resolution export.

Base resource policy on observed needs, without an assumed minimum chip/RAM baseline. Aim for smooth GPU interaction, then refinement; measure rather than promise a frame rate before the probe. A 50-megapixel RGBA32F buffer is about 800 MB. Do not retain every full-resolution intermediate or every decoded photograph on the GPU. Use a preview-resolution cache, reuse buffers and establish explicit CPU/GPU budgets. Full-resolution export may need tiling, with correct demosaic neighborhood handling if tiling occurs before development.

## 11. Delivery sequence and acceptance gates

1. **Processing probes:** supplied Sony roll, controlled known-space linear development, measured clipping/nonpositive-input behavior, two calibrated anchors, CPU inversion reference, verified ACES2 preset, EXR/TIFF metadata round-trip, explicit macOS Metal color-space experiment. Resolve critical assumptions before broad UI work.
2. **Vertical slice:** selected RAW → calibrate base/leader → adjust roll gamma/WB → managed GPU ACES preview → EXR and both TIFF targets. Base = 0 and leader = 1 across several gammas within stated tolerance. Compare interactive GPU processing numerically against the CPU reference before adopting it.
3. **Roll workflow:** ordered selection, filmstrip, shared and per-photo state, project persistence, reference-only images, batch export. Shared edits affect all photos; photo edits affect only one. Reopen reproduces output.
4. **Usability:** zoom/pan, full-resolution region sampling, undo/redo, missing-file recovery, cancellation and memory limits. Verify sampling does not change with viewer zoom and preview resolution.
5. **Hardware validation:** Apple Silicon managed SDR preview against a known ColorSync-managed reference; internal/external-monitor transitions; export and settled-preview agreement under matched viewing transforms. Windows ACM detection and inactive-state reporting are acceptance gates for the later Windows implementation, not the first macOS release.

Stop expanding scope once the calibration-to-export workflow is dependable. Defer extra color spaces, scopes and retouching.

## 12. Remaining engineering gates

At this historical checkpoint, product choices and the original execution plan were approved; continue incrementally through its verification gates. Do not substitute unverified technical assumptions for completed decisions or ask the user to choose arbitrary numerical tolerances/library versions.

- Verify LibRaw normalization, camera-matrix/WB convention, clipping/quantization, compression support and nonpositive-input incidence on the supplied roll.
- Specify/version gamma-wheel and Bradford-WB coordinate mappings, default/range choices, normalization, sample statistics and fit rejection criteria; test manual controls and neutral-pick round-trips.
- Choose and document absolute/relative scalar/CPU/GPU tolerances before optimization; report measured ACES differences, including negatives and HDR inputs.
- Preserve generated OCIO shader integration and the current sRGB/P3 color/ICC/encoding agreement. Reopen TIFFs in an ICC-aware reference; Lightroom interoperability remains intentionally deferred.
- Prove wide-gamut managed macOS presentation through the owned winit/wgpu host and its native tagging/reconfiguration behavior. Never treat an explicit surface declaration as completed hardware color validation.
- Set worker/cache/buffer budgets and latency targets from measurements on available hardware; establish full-resolution-derived preview resampling order.
- Proposed deployment default: macOS 14+ on Apple Silicon, subject to actual pinned dependency/API minimums. Test hardware and monitor availability remain environment-dependent; report unavailable checks rather than claiming them.
- Source-change detection/relink behavior: proposed default is to flag fingerprint mismatch and require explicit source acceptance without silently resampling. This is separate from the agreed no-action removal of reference filmstrip entries.

Flat-field correction, crop/rotation, contrast/pivot, vectorscope, extra inversion spaces and later-platform implementation stay deferred.

## Reference documentation

- [LibRaw documentation](https://www.libraw.org/docs)
- [egui wgpu callbacks](https://docs.rs/egui-wgpu/latest/egui_wgpu/trait.CallbackTrait.html)
- [egui custom wgpu example](https://github.com/emilk/egui/blob/main/crates/egui_demo_app/src/apps/custom3d_wgpu.rs)
- [Rust EXR crate](https://docs.rs/exr/latest/exr/)
- [Rust TIFF crate](https://docs.rs/tiff/latest/tiff/)
- [Little CMS Rust wrapper](https://docs.rs/lcms2/6.2.0/lcms2/)
- [Little CMS parametric curve implementation](https://github.com/mm2/Little-CMS/blob/master/src/cmsgamma.c)
- [ACES output transforms](https://docs.acescentral.com/system-components/output-transforms/)
- [Windows Advanced Color and ICC profiles](https://learn.microsoft.com/en-gb/windows/win32/wcs/advanced-color-icc-profiles)
- [Windows HDR / Advanced Color application guidance](https://learn.microsoft.com/en-us/windows/win32/direct3darticles/high-dynamic-range)
- [wgpu 30.0.1 surface color spaces](https://docs.rs/wgpu/30.0.1/wgpu/enum.SurfaceColorSpace.html)
- [wgpu 30.0.1 Metal surface configuration](https://github.com/gfx-rs/wgpu/blob/v30.0.1/wgpu-hal/src/metal/surface.rs)
- [Metal layer color space and nil behavior](https://developer.apple.com/documentation/quartzcore/cametallayer/colorspace)
- [LibRaw 0.22 development/matrix/scaling implementation](https://github.com/LibRaw/LibRaw/blob/0.22.0/src/postprocessing/postprocessing_utils_dcrdefs.cpp)
- [LibRaw 0.22 integer clipping implementation](https://github.com/LibRaw/LibRaw/blob/0.22.0/src/postprocessing/postprocessing_utils.cpp)
- [Apple CIRAWFilter](https://developer.apple.com/documentation/coreimage/cirawfilter)
- [Apple RAW decoder-version compatibility](https://developer.apple.com/documentation/coreimage/cirawfilter/decoderVersion)
- [Official ACES 2 package tags](https://github.com/aces-aswf/aces/tags)
- [OCIO 2.6 release and config-version behavior](https://opencolorio.readthedocs.io/en/latest/releases/ocio_2_6.html)
- [OCIO 2.6 shader languages](https://opencolorio.readthedocs.io/en/v2.6.0/api/enums.html#gpulanguage)
- [wgpu 30 native shader input formats](https://docs.rs/wgpu/30.0.1/wgpu/enum.ShaderSource.html)
- [OCIO 2.6 ACES rendering/limiting versus encoding implementation](https://github.com/AcademySoftwareFoundation/OpenColorIO/blob/v2.6.0/src/OpenColorIO/transforms/builtins/ACES.cpp)
