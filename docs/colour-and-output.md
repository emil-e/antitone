# Working colour, rendering and output

A working positive and a displayed photograph are not the same representation. The working image needs room for grading; the displayed image must fit a particular output. Antitone keeps those responsibilities separate.

```text
signed/HDR AP1 photographic grade
  → optional emulation (image MTF → independent halation → grain)
  ├→ AP1-to-AP0 → scene-linear EXR
  └→ optional LUT → optional Film & paper → ACES output transform → linear sRGB or Display P3
       ├→ target encoding + ICC → TIFF
       ├→ target encoding → scopes
       └→ extended-sRGB presentation → OS monitor conversion
```

[Processing](processing.md) explains the working positive and [Film texture](processing.md#film-texture).

## A working space is not a display gamut

AP1/ACEScg supplies common RGB primaries and a D60 white for photographic processing. Negative components and values above one are retained: they may result from the camera matrix, inversion or grading, and should not be clipped merely to make intermediate data look display-ready.

AP0 is used for EXR interchange and internally by ACES, not as the space in which to invert the negative. The distinction prevents output conventions from changing the calibration model.

## Render for a declared target

The current preview and TIFF targets are **100-nit sRGB and Display P3**, both with D65 white. Antitone uses the pinned, unmodified ACES 2 output transform through OCIO. The transform's input/output range limits belong here, at rendering; they do not limit working data or EXR. Selecting P3 changes the actual rendering/limiting primaries, not just the file's label. Preview, thumbnails and scopes use that target; the Export dialog initializes its TIFF target from Rendering, then allows an independent choice for that export. Display P3 uses the segmented sRGB transfer, not cinema P3's gamma 2.6.

Generated GPU algorithms and tables implement the same pinned transform as the independent OCIO reference. The shared renderer adapts resource layout without rewriting the transform — which is why changes must be checked across both paths and their composed consumers rather than adjusted in a shader alone. [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md) own generation and provenance.

## Colour treatments precede output rendering

Rendering owns two independently bypassable project-wide colour assets: an imported **LUT**, followed by **Film & paper**. Both execute after photographic grading and grain, before selected-output ACES. Each uses the same ACEScct-shaped AP1/D60 table contract and tetrahedral operation; enabling both composes two image nodes in that order. Selected positive views, thumbnails, scopes, original references and TIFF capture the same ordered assets. Source inspection, calibration, Analyze and ungraded repair evaluation bypass them.

Import declares this working-space contract: a `.cube` cannot prove colour intent, so use a creative ACEScct/AP1 LUT rather than an output-rendering, display or camera-conversion LUT. Dimensions, domain, sample layout and finite float32 execution are validated. Finite values outside each LUT's declared input window clip component-wise at that sampling boundary; upstream working data and stored samples remain unchanged. Nonfinite values and malformed assets reject. Reusable LUT/Display values identify ordered numeric content and output target; texture attachments and output validation belong to each encoding. Reusing failed pixels must still fail the new readout.

[Projects embed both accepted numeric assets](projects.md#embed-accepted-colour-assets), independently of source paths, catalogues and derived caches. [Spektrafilm](../assets/spektrafilm/README.md) supplies factored upstream data and native algorithms for combined negative/paper or paper-only processing. Paper-only consumes the interpreted positive through a declared exposure adapter rather than recovering a real negative's spectrum. Independently sourced [B&W materials](../assets/papers/README.md) supply scalar negative formation and fixed-grade paper responses with neutral transmission/reflection. Newly generated material treatments use 65³ float32 samples.

## Source inspection has its own exposure convention

Negative source viewing balances unity-developed camera light for inspection before ACES. LibRaw uses green-relative daylight/as-shot gain ratios, expressed in the cached source's AP1 basis. The DNG SDK supplies profile-adapted viewing matrices; applying generic gains on top would balance it twice. Header preparation retains these per-source transforms separately from development identity. Unavailable as-shot WB falls back to daylight; unavailable daylight leaves the source's existing balance.

Canvas and source-preview thumbnails use a common **+1 EV** ACES inspection baseline, with manual inspection EV relative to it. They consume the [shared source exposure scale](processing.md#start-with-the-cameras-measurements) without repeating WB normalization. This extra stop belongs only to inspection; Digital positives and exports use their [per-capture starting exposure](processing.md#digital-starting-brightness). [Source inspection](editor.md#source-inspection-is-a-view) owns the transient controls.

## Let the OS handle the monitor

The preview converts linear target RGB to extended-sRGB and encodes it once. Presentation is SDR on both platforms: macOS uses an explicitly extended-sRGB-tagged layer with EDR disabled; Windows uses a 10-bit or 8-bit Vulkan swapchain. The OS performs the monitor conversion; applying a monitor profile ourselves as well would introduce a second conversion. A correct tagged layer is not proof of a calibrated monitor or of correct behavior on every display.

## TIFF and EXR serve different purposes

Processing selects **scene-referred** or **display-referred** output independently of file codec and sample storage. Scene-referred output retains graded signed/HDR AP1, including Film texture, without either colour asset or display rendering. Display-referred output includes the ordered colour assets and ACES for its declared target. Display-referred pixels can remain linear floats; transfer encoding and integer quantization are downstream choices.

The current **TIFF** exporter selects display-referred output, target transfer and RGB16 quantization, with a matching ICC profile. **EXR** selects scene-referred output and serializes linear AP0/D60 with declared chromaticities, without display transfer or SDR clipping. Half and full precision are available; half overflow rejects rather than silently clipping.

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. Shared recipe assembly supplies the captured effect pipeline; the file adapter selects original-sample planning and output representation without interpreting individual effects or applying view-quality sampling limits. One bounded GPU executor processes originals into FP32 linear pixels, reconstructs with source-cell area weights and reads back only destination pixels. Display rendering applies the captured LUT chain and pinned GPU ACES per original before reconstruction, regardless of eventual codec; scene-referred processing bypasses those stages. EXR's AP0 conversion remains at serialization. The engine acquires real source neighbours from the stages' [composed support](architecture.md#pixel-stages-and-spatial-support), plus reconstruction roundoff, so tile edges never become filter boundaries. Cold repair selection consumes that support around the captured composition; a cropped-out repair can still influence a filtered edge. An applicable evaluation failure fails export without replacing the destination — there is no implicit unrepaired fallback.

RGB16 quantization uses a [codec-independent GPU stage](../src/gpu/rgb16.rs) after reconstruction, recording transfer and packing in the same GPU submission and validating the complete block's status on the CPU. RGB16 colour target, encoded sample representation and file codec are separate choices; sRGB and Display P3 currently share their transfer curve.

[File encoding](../src/export/stream.rs) retains row bands rather than complete output sample arrays. TIFF preserves block visitation while writing eight-row, horizontally predicted Deflate strips and matching ICC. EXR serially writes ZIP16 chunks from a planar band with AP0 chromaticities and selected precision. Workspace admission includes bands and offset tables; native decoding still retains its complete source.

Quality means preserving the photographic model and reconstruction, with additional arithmetic cost justified by meaningful differences in real photographs. Scalar/OCIO comparisons check the model and expose differences for investigation; their numerical tolerances are not visibility thresholds. Encoder tests separately require exact serialization of supplied samples. [Processing](processing.md#one-f32-photographic-implementation) explains the shared f32 arithmetic; [encoding observations](diagnostics.md#tiff-encoding-cost) cover Deflate cost and compatibility; [geometry](geometry.md#process-before-averaging) explains rotated and reduced reconstruction.

Exports use immutable settings snapshots, including ordered adjustment layers, grain/halation and both accepted colour assets, so subsequent editor changes cannot affect an in-flight file. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. Worker stop tokens cover repair preparation, bounded source work and the final publication barrier; a stop observed before publication leaves the destination unchanged, while a stop after successful publication does not turn a committed file into a cancellation. This is not a complete power-loss durability guarantee.

Selected/all-filmstrip exports run sequentially from captured recipes and source specifications, independently of viewing history. Each photo resolves applicable repairs, renders, encodes and publishes before the next original is acquired. Keep both/Skip use exclusive publication and handle competing destination creation; confirmed Replace permits atomic replacement. Duplicate and filesystem-equivalent batch names are disambiguated so one photo cannot replace an earlier output. Source/project destinations are protected. Individual-photo failures continue; shared renderer/kernel initialization failure or worker panic stops the batch. Completed files survive cancellation and are counted in the partial summary. [Editor](editor.md#export-photographs) owns the dialog and progress interaction.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- Presentation: [shared host](../src/platform/host.rs), [macOS layer tagging](../src/platform/macos.rs) and [Windows surface selection](../src/platform/windows.rs).
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, colour tagging and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer.