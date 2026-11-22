# Working colour, rendering and output

A working positive and a displayed photograph are not the same representation. The working image needs room for grading; the displayed image must fit a particular output. Antitone keeps those responsibilities separate.

```text
signed/HDR AP1 photographic grade
  → optional emulation (image MTF → independent halation → grain)
  ├→ scene-linear reconstruction → AP0 float samples → file codec
  └→ optional LUT → optional Film & paper → ACES output transform → linear sRGB or Display P3
       ├→ display-referred reconstruction → target transfer + quantization → file codec
       ├→ target encoding → scopes
       └→ extended-sRGB presentation → OS monitor conversion
```

[Processing](processing.md) explains the working positive and [Film texture](processing.md#film-texture).

## A working space is not a display gamut

AP1/ACEScg supplies common RGB primaries and a D60 white for photographic processing. Negative components and values above one are retained: they may result from the camera matrix, inversion or grading, and should not be clipped merely to make intermediate data look display-ready.

AP0 is used for scene-linear interchange and internally by ACES, not as the space in which to invert the negative. The distinction prevents output conventions from changing the calibration model.

## Render for a declared target

The supported display targets are **100-nit sRGB and Display P3**, both with D65 white. Antitone uses the pinned, unmodified ACES 2 output transform through OCIO. Its range limits apply to display rendering, not to working data or scene-linear output. Selecting P3 changes the rendering and limiting primaries, not just the file tag. Preview, thumbnails and scopes use the project's Rendering target; display-referred export starts from that target and can override it. Display P3 uses the segmented sRGB transfer, not cinema P3's gamma 2.6.

The GPU [DRT stage](../src/gpu/drt.rs) owns the target algorithms and tables. It adapts the pinned transform’s resource layout without rewriting its arithmetic; check changes against the independent OCIO reference and in composed consumers. [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md) own generation and provenance.

## Colour treatments precede output rendering

Rendering owns two independently bypassable project-wide colour assets: an imported **LUT**, followed by **Film & paper**. Both execute after photographic grading and grain, before selected-output ACES. Every resolved treatment uses the same AP1/D60 LUT contract: ACEScct input coordinates and node values, tetrahedral interpolation, then decoding to linear AP1. Enabling both slots composes two image nodes in that order. Selected positive views, thumbnails, scopes and display-referred exports capture the same ordered assets. Source inspection, calibration measurements, Analyze and ungraded repair evaluation bypass them.

Import declares this working-space contract: a `.cube` cannot prove colour intent, so use a creative ACEScct/AP1 LUT rather than an output-rendering, display or camera-conversion LUT. Import checks dimensions, domain, sample layout and that the samples decode to finite scene-linear float32. Finite values outside each LUT's declared input window clip component-wise at that sampling boundary; upstream working data and stored samples remain unchanged. Reusable LUT/Display values identify ordered numeric content and output target independently of any texture attachment.

[Projects embed both accepted numeric assets](projects.md#embed-accepted-colour-assets), independently of source paths, catalogues and derived caches. [Spektrafilm](../assets/spektrafilm/README.md) supplies factored upstream data and native algorithms for combined negative/paper or paper-only processing. Paper-only consumes the interpreted positive through a declared exposure adapter rather than recovering a real negative's spectrum. Independently sourced [B&W materials](../assets/papers/README.md) supply scalar negative formation and fixed-grade paper responses with neutral transmission/reflection. Providers retain their own selection and source models; shared preparation samples them into 65³ float32 LUTs. [Frontier A/B](../assets/frontier/README.md) support Negative None or a colour stock and bake their complete formation, scanner readout and density response through this same boundary. Provider provenance does not select a pixel executor. Ordinary selected-output ACES follows for both sRGB and P3.

## Source inspection has its own exposure convention

[Negative source viewing](../src/gpu/interpretation.rs) balances unity-developed camera light for inspection before ACES. LibRaw uses green-relative daylight/as-shot gain ratios, expressed in the cached source's AP1 basis. The DNG SDK supplies profile-adapted viewing matrices; applying generic gains on top would balance it twice. Header preparation retains these per-source transforms separately from development identity. Unavailable as-shot WB falls back to daylight; unavailable daylight leaves the source's existing balance.

Canvas and source-preview thumbnails use a common **+1 EV** ACES inspection baseline, with manual inspection EV relative to it. They consume the [shared source exposure scale](processing.md#start-with-the-cameras-measurements) without repeating WB normalization. This extra stop belongs only to inspection; Digital positives and exports use their [per-capture starting exposure](processing.md#digital-starting-brightness). [Source inspection](editor.md#source-inspection-is-a-view) owns the transient controls.

## Let the OS handle the monitor

The canvas and filmstrip share a [presentation pipeline](../src/viewer/presentation.rs) that maps the finished target-linear texture to the viewport and converts it to encoded extended-sRGB. Presentation is SDR on both platforms: macOS uses an explicitly extended-sRGB-tagged layer with EDR disabled; Windows uses a 10-bit or 8-bit Vulkan swapchain. The OS performs the monitor conversion; applying a monitor profile ourselves as well would introduce a second conversion. A correct tagged layer is not proof of a calibrated monitor or of correct behavior on every display.

## Render and encode a file

The target selects what the [photographic graph](../src/gpu/photograph/composition.rs) renders. **Display-referred** output includes the ordered colour assets and ACES for the chosen display target. **Scene-linear** output takes the graded AP1 image after Film texture, before either colour asset or display rendering. File settings select the sample layout. The [export plan](../src/export.rs) checks that the two are compatible before rendering. The Export dialog pairs them in presets: TIFF with display-referred RGB16, or EXR with scene-linear half or full float.

Export executes the selected `Value` using accepted composition and full-resolution source pixels. A [bounded executor](../src/gpu_export.rs) applies the graph on original samples, reconstructs with source-cell area weights and reads back only destination pixels. It acquires real source neighbours from the stages' [composed support](architecture.md#pixel-stages-and-spatial-support), plus reconstruction roundoff, so tile edges never become filter boundaries. A cropped-out repair can still influence a filtered edge. The [repair-local fallback](processing.md#ungraded-positive-and-repair-placement) preserves underlying pixels for failed heals. Other evaluation failures abort export without replacing the destination.

Sample conversion follows reconstruction. The [RGB16 stage](../src/gpu/rgb16.rs) applies the target transfer curve and quantizes in the same GPU submission. Scene-linear export converts AP1 to linear AP0/D60 and uses only floating-point layouts; signed and above-white values survive within storage range. Finite values outside that range saturate at the signed limits of the chosen precision. Non-finite pixels still fail.

[Codecs](../src/export/codec.rs) receive prepared blocks and derive colour tags from the target. [Streaming encoders](../src/export/stream.rs) retain row bands, not complete output sample arrays. TIFF uses eight-row, horizontally predicted Deflate strips and a matching ICC profile. EXR uses ZIP16 chunks and declares AP0 chromaticities. Workspace admission includes bands and offset tables; native decoding still retains its complete source.

Quality means preserving the photographic model and reconstruction, with additional arithmetic cost justified by meaningful differences in real photographs. Scalar/OCIO comparisons check the model and expose differences for investigation; their numerical tolerances are not visibility thresholds. Encoder tests separately require exact serialization of supplied samples. [Processing](processing.md#one-f32-photographic-implementation) explains the shared f32 arithmetic; [encoding observations](diagnostics.md#tiff-encoding-cost) cover Deflate cost and compatibility; [geometry](geometry.md#process-before-averaging) explains rotated and reduced reconstruction.

Exports use immutable settings snapshots, including ordered adjustment layers, grain/halation and both accepted colour assets, so subsequent editor changes cannot affect an in-flight file. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. Worker stop tokens cover repair preparation, bounded source work and the final publication barrier; a stop observed before publication leaves the destination unchanged, while a stop after successful publication does not turn a committed file into a cancellation. This is not a complete power-loss durability guarantee.

Selected/all-filmstrip exports run sequentially from captured recipes and source specifications, independently of viewing history. Each photo executes its demanded graph, encodes and publishes before the next original is acquired. Keep both/Skip use exclusive publication and handle competing destination creation; confirmed Replace permits atomic replacement. Duplicate and filesystem-equivalent batch names are disambiguated so one photo cannot replace an earlier output. Source/project destinations are protected. Individual-photo failures continue. Failure to initialize shared display resources stops a display-referred batch; scene-linear exports do not acquire them. A worker panic stops the batch. Completed files survive cancellation and are counted in the partial summary. [Editor](editor.md#export-photographs) owns the dialog and progress interaction.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- Presentation: [shared host](../src/platform/host.rs), [macOS layer tagging](../src/platform/macos.rs) and [Windows surface selection](../src/platform/windows.rs).
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, colour tagging and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer.