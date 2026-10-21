# Working colour, rendering and output

A working positive and a displayed photograph are not the same representation. The working image needs room for grading; the displayed image must fit a particular output. Antitone keeps those responsibilities separate.

```text
signed/HDR AP1 working positive
  ├→ AP1-to-AP0 → scene-linear EXR
  └→ optional ACEScct/AP1 creative LUT → ACES output transform → linear sRGB or Display P3
       ├→ target encoding + ICC → TIFF
       ├→ target encoding → scopes
       └→ extended-sRGB presentation → OS monitor conversion
```

[Processing](processing.md) explains how the working positive is made.

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

The undeveloped negative is not a graded working positive. LibRaw's highlight-safe development divides fixed WB gains by their maximum, which can leave green below sensor white. Source presentation alone restores a green-referenced scale, then applies a common **+1 EV** ACES preview baseline — a rendering convention that makes the default less dark without changing the RAW buffer or camera-to-AP1 conversion. It is not a camera exposure calibration, and neither it nor WB normalization promises camera-JPEG/LCD matching across cameras.

Selected source views and uncalibrated developed thumbnails use the same default; missing or invalid WB falls back to the baseline alone. Manual source-inspection EV is relative to that default and affects only the selected view. See [source inspection](editor.md#source-inspection-is-a-view) for transient editor state.

## Let the OS handle the monitor

The preview converts linear target RGB to extended-sRGB, encodes it once and submits it to an explicitly extended-sRGB-tagged macOS layer (SDR, with EDR disabled). The OS performs the monitor conversion; applying a monitor profile ourselves as well would introduce a second conversion. A correct tagged layer is not proof of a calibrated monitor or of correct behavior on every display.

## TIFF and EXR serve different purposes

**TIFF** is a rendered photograph: full-resolution processing, ACES, the selected target transfer and RGB16 quantization, with the matching ICC profile — suitable for viewing in colour-managed applications. **EXR** retains a scene-linear-style working positive: the accepted grade converted from signed/HDR AP1 to AP0/D60, bypassing both the creative look and ACES, without display transfer or SDR clipping. Half and full precision are available; half overflow rejects rather than silently clipping.

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. One bounded GPU engine processes originals into FP32 linear pixels, reconstructs with source-cell area weights and reads back only destination pixels; TIFF applies the captured creative LUT and pinned GPU ACES per original, while EXR bypasses those stages and converts to AP0 only at serialization. The engine plans conservative source bounds plus a real-cell halo per destination block; repaired edges need no alternate engine, and cold repair relevance uses the same whole-composition authority as output. Excluded supports must have zero exact composition coverage. An applicable no-donor or evaluation failure fails export without replacing the destination — there is no implicit unrepaired fallback.

RGB16 quantization uses a [codec-independent GPU stage](../src/gpu_rgb16.rs) after reconstruction, recording transfer and packing in the same GPU submission and validating the complete block's status on the CPU. RGB16 colour target, encoded sample representation and file codec are separate choices; sRGB and Display P3 currently share their transfer curve.

Quality means preserving that photographic model and reconstruction, avoiding unnecessary precision loss and checking numerical errors independently. It does **not** require bit identity with scalar/OCIO arithmetic: near quantization thresholds, small implementation differences can change a stored code. The shared FP32 working comparison budget is `2e-5 + 2e-4 × |reference|`, with output quantization accounted for separately — an engineering accuracy/cost choice, not a visibility threshold. [Processing](processing.md#preserve-information-reduce-mathematical-work) explains the guarded arithmetic; [encoding observations](diagnostics.md#tiff-encoding-cost) retain the Deflate workload and compatibility limits; [geometry](geometry.md#process-before-averaging) explains how rotated/reduced outputs reconstruct pixels.

Exports use immutable settings snapshots, including ordered local processing and the accepted look, so subsequent editor changes cannot affect an in-flight file. TIFF identifies its numeric look in metadata; EXR bypasses it and does not claim it was applied. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. Worker stop tokens cover repair preparation, bounded source work and the final publication barrier; a stop observed before publication leaves the destination unchanged, while a stop after successful publication does not turn a committed file into a cancellation. This is not a complete power-loss durability guarantee.

Selected/all-filmstrip exports run sequentially from captured recipes and source specifications, independently of viewing history. Each photo resolves applicable repairs, renders, encodes and publishes before the next original is acquired. Keep both/Skip use exclusive publication and handle competing destination creation; confirmed Replace permits atomic replacement. Duplicate and filesystem-equivalent batch names are disambiguated so one photo cannot replace an earlier output. Source/project destinations are protected. Individual-photo failures continue; shared renderer/kernel initialization failure or worker panic stops the batch. Completed files survive cancellation and are counted in the partial summary. [Editor](editor.md#export-photographs) owns the dialog and progress interaction.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- [Presentation](../src/platform/macos.rs): macOS layer tagging.
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, metadata and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer.