# Working colour, rendering and output

A working positive and a displayed photograph are not the same representation. The working image needs room for grading; the displayed image must fit a particular output. Antitone keeps those responsibilities separate.

```text
signed/HDR AP1 photographic grade
  → optional Film Emulation (image MTF → grain)
  ├→ AP1-to-AP0 → scene-linear EXR
  └→ optional ACEScct/AP1 creative LUT → ACES output transform → linear sRGB or Display P3
       ├→ target encoding + ICC → TIFF
       ├→ target encoding → scopes
       └→ extended-sRGB presentation → OS monitor conversion
```

[Processing](processing.md) explains the working positive and [Film Emulation](processing.md#film-emulation).

## A working space is not a display gamut

AP1/ACEScg supplies common RGB primaries and a D60 white for photographic processing. Negative components and values above one are retained: they may result from the camera matrix, inversion or grading, and should not be clipped merely to make intermediate data look display-ready.

AP0 is used for EXR interchange and internally by ACES, not as the space in which to invert the negative. The distinction prevents output conventions from changing the calibration model.

## Render for a declared target

The current preview and TIFF targets are **100-nit sRGB and Display P3**, both with D65 white. Antitone uses the pinned, unmodified ACES 2 output transform through OCIO. The transform's input/output range limits belong here, at rendering; they do not limit working data or EXR. Selecting P3 changes the actual rendering/limiting primaries, not just the file's label. Preview, thumbnails and scopes use that target; the Export dialog initializes its TIFF target from Rendering, then allows an independent choice for that export. Display P3 uses the segmented sRGB transfer, not cinema P3's gamma 2.6.

Generated GPU algorithms and tables implement the same pinned transform as the independent OCIO reference. The shared renderer adapts resource layout without rewriting the transform — which is why changes must be checked across both paths and their composed consumers rather than adjusted in a shader alone. [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md) own generation and provenance.

## Creative looks precede output rendering

Rendering owns one **project-wide** look: None, a built-in material-derived look, or an imported 3D `.cube`. None is a true bypass. Both sources resolve to the same immutable ACEScct/AP1/D60 asset and tetrahedral sampler, after photographic grading and before ordinary selected-output ACES. Source acquisition, calibration, Analyze and ungraded repair evaluation remain look-free. Selected positive views, thumbnails, scopes, original references and TIFF share the accepted asset.

Import explicitly declares this scene-referred contract: a `.cube` cannot prove colour intent, so output-rendering transforms, display LUTs and arbitrary camera/log conversions are not alternative modes. Dimensions, domain, sample layout and finite float32 execution are validated with bounded limits. Finite values outside the LUT's declared input window are clipped component-wise to its boundary **at the sampling boundary** — this does not change working AP1, calibration, photographic adjustments or the stored samples. Nonfinite values and malformed assets reject rather than becoming an edge colour or a substitute look.

The [project embeds that accepted numeric asset](projects.md#embed-the-accepted-look-not-a-catalog-reference); saved appearance does not depend on the original path, catalog or derived cache. The four selected [independent models](../assets/looks/README.md) currently generate 129³ float32 LUTs for new built-in resolutions; derivation and pinned-reference generation belong to that guide. These are creative looks, not calibrated physical printing or projection.

## Source inspection has its own exposure convention

Negative source viewing balances unity-developed camera light for inspection before ACES. LibRaw uses green-relative daylight/as-shot gain ratios, expressed in the cached source's AP1 basis. The DNG SDK supplies profile-adapted viewing matrices; applying generic gains on top would balance it twice. Header preparation retains these per-source transforms separately from development identity. Unavailable as-shot WB falls back to daylight; unavailable daylight leaves the source's existing balance.

Canvas and source-preview thumbnails use a common **+1 EV** ACES inspection baseline, with manual inspection EV relative to it. They consume the [shared source exposure scale](processing.md#start-with-the-cameras-measurements) without repeating WB normalization. This extra stop belongs only to inspection; Digital positives and exports use their [per-capture starting exposure](processing.md#digital-starting-brightness). [Source inspection](editor.md#source-inspection-is-a-view) owns the transient controls.

## Let the OS handle the monitor

The preview converts linear target RGB to extended-sRGB and encodes it once. Presentation is SDR on both platforms: macOS uses an explicitly extended-sRGB-tagged layer with EDR disabled; Windows uses a 10-bit or 8-bit Vulkan swapchain. The OS performs the monitor conversion; applying a monitor profile ourselves as well would introduce a second conversion. A correct tagged layer is not proof of a calibrated monitor or of correct behavior on every display.

## TIFF and EXR serve different purposes

**TIFF** is a rendered photograph: full-resolution processing, ACES, the selected target transfer and RGB16 quantization, with the matching ICC profile — suitable for viewing in colour-managed applications. **EXR** retains a scene-linear-style working positive: the accepted grade and enabled Film Emulation converted from signed/HDR AP1 to AP0/D60, bypassing both the creative look and ACES, without display transfer or SDR clipping, and it declares its linear AP0/D60 chromaticities. Half and full precision are available; half overflow rejects rather than silently clipping. Both declare their colour space in the file, so a reader needs no external convention.

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. One bounded GPU engine processes originals into FP32 linear pixels, reconstructs with source-cell area weights and reads back only destination pixels; TIFF applies the captured creative LUT and pinned GPU ACES per original, while EXR bypasses those stages and converts to AP0 only at serialization. The engine reads real source neighbours for the image-MTF halo as well as reconstruction roundoff; tile edges never become filter boundaries. Cold repair relevance includes the MTF neighbourhood of the captured composition, so an otherwise cropped-out repair can still affect its blurred edge. An applicable no-donor or evaluation failure fails export without replacing the destination — there is no implicit unrepaired fallback.

RGB16 quantization uses a [codec-independent GPU stage](../src/gpu_rgb16.rs) after reconstruction, recording transfer and packing in the same GPU submission and validating the complete block's status on the CPU. RGB16 colour target, encoded sample representation and file codec are separate choices; sRGB and Display P3 currently share their transfer curve.

Quality means preserving the photographic model and reconstruction, with additional arithmetic cost justified by meaningful differences in real photographs. Scalar/OCIO comparisons check the model and expose differences for investigation; their numerical tolerances are not visibility thresholds. Encoder tests separately require exact serialization of supplied samples. [Processing](processing.md#one-f32-photographic-implementation) explains the shared f32 arithmetic; [encoding observations](diagnostics.md#tiff-encoding-cost) cover Deflate cost and compatibility; [geometry](geometry.md#process-before-averaging) explains rotated and reduced reconstruction.

Exports use immutable settings snapshots, including ordered adjustment layers, Film Emulation and the accepted look, so subsequent editor changes cannot affect an in-flight file. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. Worker stop tokens cover repair preparation, bounded source work and the final publication barrier; a stop observed before publication leaves the destination unchanged, while a stop after successful publication does not turn a committed file into a cancellation. This is not a complete power-loss durability guarantee.

Selected/all-filmstrip exports run sequentially from captured recipes and source specifications, independently of viewing history. Each photo resolves applicable repairs, renders, encodes and publishes before the next original is acquired. Keep both/Skip use exclusive publication and handle competing destination creation; confirmed Replace permits atomic replacement. Duplicate and filesystem-equivalent batch names are disambiguated so one photo cannot replace an earlier output. Source/project destinations are protected. Individual-photo failures continue; shared renderer/kernel initialization failure or worker panic stops the batch. Completed files survive cancellation and are counted in the partial summary. [Editor](editor.md#export-photographs) owns the dialog and progress interaction.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- Presentation: [shared host](../src/platform/host.rs), [macOS layer tagging](../src/platform/macos.rs) and [Windows surface selection](../src/platform/windows.rs).
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, colour tagging and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer.