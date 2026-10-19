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

AP1/ACEScg supplies common RGB primaries and a D60 white for photographic processing. Negative components and values above one are retained. They may result from the camera matrix, inversion or grading and should not be clipped merely to make intermediate data look display-ready.

AP0 is used for EXR interchange and internally by ACES, not as the space in which to invert the negative. The distinction prevents output conventions from changing the calibration model.

## Render for a declared target

The current preview and TIFF targets are **100-nit sRGB and Display P3**, both with D65 white. Antitone uses the pinned, unmodified ACES 2 output transform through OCIO. The transform's input/output range limits belong here, at rendering; they do not limit working data or EXR.

Selecting P3 changes the actual rendering/limiting primaries, not just the file's label. Preview, thumbnails, scopes and TIFF use that same selected target. Display P3 uses the segmented sRGB transfer, not cinema P3's gamma 2.6.

Generated GPU algorithms and tables implement the same pinned transform as the independent OCIO reference. The shared renderer adapts resource layout without rewriting the transform; its immutable resources serve viewer, thumbnails, scopes and refinement. This is why changes must be checked across both paths and their composed consumers rather than adjusted in a shader alone. [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md) own generation and provenance.

## Creative looks precede output rendering

Rendering owns one **project-wide** look: None, a built-in material-derived look, or an imported 3D `.cube`. None is a true bypass. Both sources resolve to the same immutable ACEScct/AP1/D60 input-and-output asset and tetrahedral sampler, after photographic grading and before ordinary selected-output ACES. Source acquisition, calibration, Analyze and ungraded repair evaluation remain look-free. Selected positive views, thumbnails, scopes, original references and TIFF share the accepted asset; this does not make filtered working-view processing an exact original-resolution reconstruction.

Import explicitly declares this scene-referred contract. A `.cube` cannot prove colour intent: output-rendering transforms, display LUTs, arbitrary camera/log conversions and 1D shapers are not alternative modes. Dimensions, domain, sample layout and finite float32 execution are validated with bounded input/storage limits. Imports retain their admitted grid. Finite values outside the LUT's declared input window are clipped component-wise to its boundary before tetrahedral sampling. This clipping occurs at the LUT sampling boundary; it does not change working AP1, calibration, photographic adjustments or the stored samples. In-window values are unchanged. Unlike soft compression, clipping does not reshape an interior shoulder or require per-image tuning. Both built-ins and imports use this same rule; None bypasses it, and EXR remains unaffected. Nonfinite source/evaluated values and malformed assets still reject rather than becoming an edge colour or a substitute look. Failure checks follow the rendered footprint, so unused tile padding cannot reject a crop.

Selection prepares on the worker and retains the accepted asset until a current successful reply. Pending/cancelled/stale work is not a project edit. The [project embeds that accepted numeric LUT](projects.md#embed-the-accepted-look-not-a-catalog-reference), for built-ins as well as imports. Saved appearance does not depend on the original path, catalog, derived cache or old generator. Export captures the accepted asset, not a later UI selection.

The four selected/accepted [independent models](../assets/looks/README.md) currently generate 129³ float32 LUTs for new built-in resolutions; existing embedded assets and imports retain their own dimensions. Model derivation, pinned fixed-reference generation and independent numerical checks belong to that guide. These are creative looks, not calibrated physical printing/projection. Content acceptance, approximation quality, cross-device behavior and release clearance remain separate questions.

## Source inspection has its own exposure convention

The undeveloped negative is not a graded working positive. LibRaw's highlight-safe development divides fixed WB gains by their maximum, which can leave green below sensor white. Source presentation alone restores a green-referenced scale with the scalar `max(fixed WB) / green WB`, then applies a common **+1 EV** ACES preview baseline. This makes the default less dark without changing the RAW buffer or camera-to-AP1 conversion. The baseline is a rendering convention, not a camera exposure calibration; neither it nor WB normalization promises camera-JPEG/LCD matching across cameras or ISOs.

Selected source views (including provisional and settled views and calibration/reference tools) and uncalibrated developed thumbnails use the same default. Missing, invalid or pathological WB falls back to the baseline alone; exact defensive limits follow the implementation. No histogram, embedded JPEG or manufacturer-specific exposure constant participates. Signed and above-one values still reach the existing ACES rendering boundary without an added source clamp.

Manual source-inspection EV is relative to that default and affects only the selected view. Positive previews, scopes, calibration/fitting, analysis and exports retain their existing numerical inputs and processing. Embedded camera JPEG placeholders remain independently camera-rendered. See [source inspection](editor.md#source-inspection-is-a-view) for transient editor state.

## Let the OS handle the monitor

The preview converts linear target RGB to extended-sRGB, encodes it once and submits it to an explicitly extended-sRGB-tagged macOS layer. P3 colours outside ordinary sRGB remain representable on this floating-point path. The layer is SDR, with EDR disabled.

The OS performs the monitor conversion. Applying a monitor profile ourselves as well would introduce a second conversion. Ordinary UI colours and image rendering have separate encoding responsibilities, so one must not inherit the other's transfer handling accidentally.

A correct tagged layer is not proof of a calibrated monitor, physical 100-nit luminance or correct behavior on every display. The [presentation decision](decisions/sdr-output-and-presentation.md) explains this boundary.

## TIFF and EXR serve different purposes

**TIFF** is a rendered photograph: full-resolution processing, ACES, the selected target transfer and RGB16 quantization, with the matching ICC profile. It is suitable for viewing in colour-managed applications. Nonfinite or out-of-range rendered/encoded values reject rather than being silently clipped during quantization.

**EXR** retains a scene-linear-style working positive: the accepted grade is converted from signed/HDR AP1 to AP0/D60, bypassing both the creative look and ACES, without display transfer, monitor conversion or SDR clipping. Half and full precision are available; half overflow rejects rather than silently clipping. AP0 pixels alone do not certify complete ACES-container compliance.

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. One bounded GPU engine processes originals into FP32 linear pixels, reconstructs with source-cell area weights and reads back only destination pixels. TIFF applies the captured creative LUT and pinned GPU ACES per original, so reconstruction averages linear target RGB. EXR bypasses those stages, reconstructs signed/HDR scene AP1, then converts to AP0 for serialization. Exact and rotated compositions use the same engine. Transfer/quantization, metadata and file encoding remain CPU responsibilities.

Export captures the accepted source's image recipe and resolves applicable enabled repairs on its owning worker, independently of viewer warmth. Absolute canonical f32 neutral-AP1 values replace support samples before shared photographic/local grading, the captured creative look, ACES or reconstruction. Untouched originals retain their established processing and floor/validity flags. Both formats use the same repair-grading stage. Integer source coordinates remain authoritative for correction, repair placement and local masks regardless of scratch placement. Logical sparse results do not require all patches to stay GPU-resident: offscreen consumers upload contributing pixels per bounded original unit.

Pending repair preparation is ordinary asynchronous export work. An applicable no-donor, unsupported-domain or evaluation failure fails export without replacing the destination; there is no implicit unrepaired fallback. Supports excluded from output must have zero exact composition coverage, not merely be unseen or uncached. Repair preparation does not change accepted state or dirty status.

The shared engine plans conservative source bounds plus a real-cell halo per 128-square destination block, with maximum source side 185. Source correction, repair replacement and source-point locals precede averaging; TIFF's scene look and ACES do too. Unit-local native mapping clips real source cells and normalizes positive area weights without inventing a border, shifting the crop or clamping working RGB. FMA product and addition residuals preserve small representable signals under signed HDR cancellation; weights use the same arithmetic to conserve constants.

At outer-frame units, a bounded bit mask admits only source cells with positive exact whole-composition coverage. Cold repair relevance uses that same authority, so native mapping roundoff cannot introduce an unresolved repair outside the captured frame. Repaired edges need no alternate engine. Public exports and captured editor jobs share this processing and lifecycle.

One original scratch rectangle lives until its destination block completes; overlapping originals may be evaluated again. There is no completed-original cache, pair processor, demand forecast, geometry producer or full-photo GPU atlas. The renderer shares a single cached export kernel and ACES/look infrastructure. Existing editor background prewarm requests that kernel; direct callers initialize it lazily. A TIFF job retains its LUT independently of changing source scratch dimensions. Declared payloads include host/GPU mask and repair uploads, but are not aggregate process-memory admission. TIFF retains LZW with horizontal prediction. [Processing](processing.md#preserve-information-reduce-mathematical-work) explains the guarded arithmetic choice.

Quality means preserving that photographic model and reconstruction, avoiding unnecessary precision loss and checking numerical errors independently. It does not require bit identity with scalar/OCIO arithmetic: near quantization thresholds, small implementation differences can change a stored code. The current shared FP32 working comparison budget is `2e-5 + 2e-4 × |reference|`, with output quantization accounted for separately. It is an engineering accuracy/cost choice, not a visibility threshold or an EXR requirement; full EXR storage does not promise near-f64 internal processing. Exact cancellation/conservation and flag witnesses supplement those comparisons, and selected TIFF cases retain tighter one-code scalar/OCIO checks. Encoder tests separately require exact serialization of supplied samples. Scalar/OCIO, GPU arithmetic, geometry and file/profile checks retain their own boundaries. The opt-in `ocio-reference` feature keeps CPU rendering and composed scalar adapters available only as diagnostic/test oracles; normal editor builds do not link OCIO. [Geometry](geometry.md#process-before-averaging) explains how rotated/reduced outputs reconstruct those pixels.

Exports use immutable settings snapshots, including ordered local processing and the accepted look, so subsequent editor changes cannot change an in-flight file. TIFF identifies its numeric look in metadata; EXR bypasses it and does not claim it was applied. Local stages evaluate at each original source pixel before TIFF rendering or AP1-to-AP0 EXR averaging. Nonempty local snapshots identify the processing contract in TIFF/EXR metadata, even when their records are inactive; the metadata is not a reconstructible recipe or edit history. Editor export validates the complete document against the GPU domain before dispatch; [broader local-grade acceptance](plans/local-adjustments.md) remains open. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. Worker stop tokens cover cold repair preparation, bounded source work and the final publication barrier after file flushing/sync. A stop observed there leaves the destination unchanged and retires the temporary file; already-submitted GPU work and codec calls are cooperative rather than interruptible. Captured exports remain independent of later selection/grade changes. This is not a complete power-loss durability guarantee. Batch export and a user-facing export-cancellation control remain unfinished. [Export performance](plans/export-performance.md) tracks remaining attribution and broader acceptance. Matched reference/GPU measurements and bounded resource checks establish workload-specific costs, not universal hardware speedups or photographic quality.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- [Presentation](../src/platform/macos.rs): macOS layer tagging.
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, metadata and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer. Numerical reopening and tagged presentation do not establish photographic preference or Lightroom/Resolve interoperability.
