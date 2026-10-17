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

Import explicitly declares this scene-referred contract. A `.cube` cannot prove colour intent: output-rendering transforms, display LUTs, arbitrary camera/log conversions and 1D shapers are not alternative modes. Dimensions, domain, sample layout and finite float32 execution are validated with bounded input/storage limits. Imports retain their admitted grid. Finite values outside the LUT's declared input window are clipped component-wise to its boundary before tetrahedral sampling. This is a rendering-boundary policy, not a change to working AP1, calibration, photographic adjustments or the stored samples. In-window values are unchanged. Unlike soft compression, clipping does not reshape an interior shoulder or require per-image tuning. Both built-ins and imports use this same rule; None bypasses it, and EXR remains unaffected. Nonfinite source/evaluated values and malformed assets still reject rather than becoming an edge colour or a substitute look. Failure checks follow the rendered footprint, so unused tile padding cannot reject a crop.

Selection prepares on the worker and retains the accepted asset until a current successful reply. Pending/cancelled/stale work is not a project edit. The [project embeds that accepted numeric LUT](projects.md#embed-the-accepted-look-not-a-catalog-reference), for built-ins as well as imports. Saved appearance does not depend on the original path, catalog, derived cache or old generator. Export captures the accepted asset, not a later UI selection.

The four selected/accepted [independent models](../assets/looks/README.md) currently generate 65³ float32 LUTs provisionally. Model derivation, pinned fixed-reference generation and independent numerical checks belong to that guide. These are creative looks, not calibrated physical printing/projection. Content acceptance, approximation quality, cross-device behavior and release clearance remain separate questions.

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

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. Production export processes bounded original tiles with the precise GPU photographic kernel. TIFF applies the accepted creative LUT and pinned GPU ACES per original before exact f64 coverage reconstruction in linear target RGB. EXR reads the kernel's compensated AP1 components, retaining their precision through f64 reconstruction and AP0 conversion rather than rounding through a preview-sized or f32 AP1 intermediate. Transfer/quantization, metadata and file encoding remain CPU responsibilities.

Export captures the accepted source's image recipe and resolves applicable enabled repairs on its owning worker, independently of viewer warmth. Absolute canonical f32 neutral-AP1 values replace support samples before shared photographic/local grading, ACES or reconstruction. Untouched originals retain their established processing and floor/validity flags. EXR grades replacements with compensated arithmetic and keeps high/low/tail components through reconstruction; it does not attach an ordinary-f32 viewer overlay to a compensated buffer. Each member of a paired TIFF batch uses its own source coordinates. Logical sparse results do not require all patches to stay GPU-resident: offscreen consumers upload contributing pixels per bounded original unit.

Pending repair preparation is ordinary asynchronous export work. An applicable no-donor, unsupported-domain or evaluation failure fails export without replacing the destination; there is no implicit unrepaired fallback. Supports excluded from output must have zero exact composition coverage, not merely be unseen or uncached. Repair preparation does not change accepted state or dirty status.

Straightened compositions reuse completed original-tile values in a small bounded job-local cache. The immutable source, correction, accepted grade and output format belong to that job; cached tiles never cross job boundaries. Reuse preserves their precision and leaves reconstruction's coverage weights and reduction order unchanged. Exact compositions avoid this additional retention. This is not a full-source floating-point store or a preview approximation. Coverage planning also shares exactly mapped f64 corners between neighboring cells using two short block-local edge rows; it does not step coordinates incrementally or substitute the preview's f32 affine map. Clipping classifies vertices once and skips only wholly inside/outside planes; mixed-plane intersection arithmetic, vertex order, footprint validation, weights and reduction order remain unchanged. GPU TIFF reads cached origins before misses without changing pixels within a block or the public encoder's row-major callback contract.

For straightened cached TIFF work, an optional job-owned source-demand forecast visits short 1×5 output-block strips and retains values with the nearest predicted future use. Four exact f64 block-edge corners provide conservative source-origin hints, not samples, weights or validation authority. Partial terminal source bins use the same clamped loaded origins as reconstruction. A hint may change eviction or decline retention of a current value; it never suppresses a required load, evaluates a speculative original or changes exact per-pixel reduction.

The immutable flat index is bounded to 1 MiB of explicit vector capacity and 8,192 blocks, built with fixed stack hints and one fallible allocation; allocator bookkeeping is separate. Unavailable, unsafe or over-budget forecasts retain the existing 2×2 order and LRU. The consumer alone owns its cursor, and one captured order drives both producer and encoder. Exact, uncached and EXR paths remain unforecasted. Source cache payload stays 4 MiB; index payload, source-pair temporaries and completed-plan bounds are distinct, not aggregate admission.

Before reconstruction or file encoding, TIFF batches up to two already-needed original-tile misses into one photographic dispatch, ACES render and readback. Each member keeps its validated integer source rectangle; concatenating storage never rescales coordinates, correction or local masks. Cached hits are consumed first, and the same sample slots and per-pixel f64 reduction order are filled afterward. No speculative neighboring original is evaluated. Source/cache/destination units are 128-square; smaller edge units and sources outside the pair-coordinate domain retain serial GPU processing. EXR remains unbatched.

Pair ownership is job-local: at most one extra pair processor is allocated lazily, sharing immutable grade and field resources. Its declared source/working/readback/texture payload is 2.25 MiB without a creative look, excluding small constants, bindings and driver accounting. A selected look adds a look-result RGBA32F buffer per live original processor (512 KiB for a full pair), plus renderer-shared immutable LUT samples. One decoded second original (512KiB) survives only until the next planned miss in the same block; it is separate from the 4-MiB completed-tile cache. Packed host input, the 1-MiB decoded pair, splitting/boxing/allocator transients and geometry/frame/view ownership are also separate, not an aggregate memory guarantee. Stop checks still surround every requested tile, including cache/pending hits; cancellation may wait for one two-original GPU pass. Batching shares pre-codec rendering work; TIFF compression remains independent.

TIFF overlaps exact coverage planning with GPU work: one job-local CPU helper prepares the next block while the calling worker alone owns source reads, cache ordering, GPU submissions and encoding. A rendezvous handoff permits at most two completed plans, each admitted against a block-sized payload bound; it does not queue images or retain a full-source float store. Cached-first ordering is resolved when a plan is consumed, preserving the same weights, sample slots and reduction order. The helper is joined before return, including failures; cancellation can wait for the one in-progress bounded plan. Completed-plan accounting excludes construction transients and stack, and is not an aggregate process-memory guarantee. EXR remains serial, and TIFF retains LZW with horizontal prediction.

Quality means preserving that photographic model and reconstruction, avoiding unnecessary precision loss and checking numerical errors independently. It does not require bit identity with scalar/OCIO arithmetic: near quantization thresholds, small implementation differences can change a stored code. Encoder tests separately require exact serialization of supplied samples. Existing scalar/OCIO, GPU arithmetic, geometry and file/profile gates retain their own boundaries. The opt-in `ocio-reference` feature keeps CPU rendering and composed scalar adapters available only as diagnostic/test oracles; normal editor builds do not link OCIO. [Geometry](geometry.md#process-before-averaging) explains how rotated/reduced outputs reconstruct those pixels.

Exports use immutable settings snapshots, including ordered local processing and the accepted look, so subsequent editor changes cannot change an in-flight file. TIFF identifies its numeric look in metadata; EXR bypasses it and does not claim it was applied. Local stages evaluate at each original source pixel before TIFF rendering or AP1-to-AP0 EXR averaging. Nonempty local snapshots identify the processing contract in TIFF/EXR metadata, even when their records are inactive; the metadata is not a reconstructible recipe or edit history. Editor export validates the complete document against the GPU domain before dispatch; [broader local-grade acceptance](plans/local-adjustments.md) remains open. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. This is not a complete power-loss durability guarantee. Batch export and application export cancellation remain unfinished. [Export performance](plans/export-performance.md) tracks remaining attribution and broader acceptance. Matched reference/GPU measurements and bounded reuse checks establish workload-specific costs, not universal hardware speedups or photographic quality.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- [Presentation](../src/platform/macos.rs): macOS layer tagging.
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, metadata and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer. Numerical reopening and tagged presentation do not establish photographic preference or Lightroom/Resolve interoperability.
