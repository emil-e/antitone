# Working colour, rendering and output

A working positive and a displayed photograph are not the same representation. The working image needs room for grading; the displayed image must fit a particular output. Antitone keeps those responsibilities separate.

```text
signed/HDR AP1 working positive
  ├→ AP1-to-AP0 → scene-linear EXR
  └→ ACES output transform → linear sRGB or Display P3
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

**EXR** retains a scene-linear-style working positive: the accepted grade is converted from signed/HDR AP1 to AP0/D60, without ACES, display transfer, monitor conversion or SDR clipping. Half and full precision are available; half overflow rejects rather than silently clipping. AP0 pixels alone do not certify complete ACES-container compliance.

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. Production export processes bounded original tiles with the precise GPU photographic kernel. TIFF applies pinned GPU ACES per original before exact f64 coverage reconstruction in linear target RGB. EXR reads the kernel's compensated AP1 components, retaining their precision through f64 reconstruction and AP0 conversion rather than rounding through a preview-sized or f32 AP1 intermediate. Transfer/quantization, metadata and file encoding remain CPU responsibilities.

Straightened compositions reuse completed original-tile values in a small bounded job-local cache. The immutable source, correction, accepted grade and output format belong to that job; cached tiles never cross job boundaries. Reuse preserves their precision and leaves reconstruction's coverage weights and reduction order unchanged. Exact compositions avoid this additional retention. This is not a full-source floating-point store or a preview approximation. Coverage planning also shares exactly mapped f64 corners between neighboring cells using two short block-local edge rows; it does not step coordinates incrementally or substitute the preview's f32 affine map. Clipping classifies vertices once and skips only wholly inside/outside planes; mixed-plane intersection arithmetic, vertex order, footprint validation, weights and reduction order remain unchanged. GPU TIFF reads cached origins before misses without changing pixels within a block or the public encoder's row-major callback contract.

For straightened cached TIFF work, an optional job-owned source-demand forecast visits short 1×5 output-block strips and retains values with the nearest predicted future use. Four exact f64 block-edge corners provide conservative source-origin hints, not samples, weights or validation authority. Partial terminal source bins use the same clamped loaded origins as reconstruction. A hint may change eviction or decline retention of a current value; it never suppresses a required load, evaluates a speculative original or changes exact per-pixel reduction.

The immutable flat index is bounded to 1 MiB of explicit vector capacity and 8,192 blocks, built with fixed stack hints and one fallible allocation; allocator bookkeeping is separate. Unavailable, unsafe or over-budget forecasts retain the existing 2×2 order and LRU. The consumer alone owns its cursor, and one captured order drives both producer and encoder. Exact, uncached and EXR paths remain unforecasted. Source cache payload stays 4 MiB; index payload, source-pair temporaries and completed-plan bounds are distinct, not aggregate admission.

Before reconstruction or file encoding, TIFF batches up to two already-needed original-tile misses into one photographic dispatch, ACES render and readback. Each member keeps its validated integer source rectangle; concatenating storage never rescales coordinates, correction or local masks. Cached hits are consumed first, and the same sample slots and per-pixel f64 reduction order are filled afterward. No speculative neighboring original is evaluated. Source/cache/destination units remain128-square; smaller edge units and sources outside the pair-coordinate domain retain serial GPU processing. EXR remains unbatched.

Pair ownership is job-local: at most one extra pair processor is allocated lazily, sharing immutable grade and field resources. Its declared source/working/readback/texture payload is2.25MiB, excluding small constants, bindings and driver accounting. One decoded second original (512KiB) survives only until the next planned miss in the same block; it is separate from the unchanged4-MiB completed-tile cache. Packed host input, the1-MiB decoded pair, splitting/boxing/allocator transients and existing geometry/frame/view ownership are also separate, not an aggregate memory guarantee. Stop checks still surround every requested tile, including cache/pending hits; cancellation may wait for one two-original GPU pass. This is a shared pre-codec rendering optimization, not a TIFF compression change or an implementation of future rendered formats.

TIFF overlaps exact coverage planning with GPU work: one job-local CPU helper prepares the next block while the calling worker alone owns source reads, cache ordering, GPU submissions and encoding. A rendezvous handoff permits at most two completed plans, each admitted against a block-sized payload bound; it does not queue images or retain a full-source float store. Cached-first ordering is resolved when a plan is consumed, preserving the same weights, sample slots and reduction order. The helper is joined before return, including failures; cancellation can wait for the one in-progress bounded plan. Completed-plan accounting excludes construction transients and stack, and is not an aggregate process-memory guarantee. EXR remains serial, and TIFF retains LZW with horizontal prediction.

Quality means preserving that photographic model and reconstruction, avoiding unnecessary precision loss and checking numerical errors independently. It does not require bit identity with scalar/OCIO arithmetic: near quantization thresholds, small implementation differences can change a stored code. Encoder tests separately require exact serialization of supplied samples. Existing scalar/OCIO, GPU arithmetic, geometry and file/profile gates retain their own boundaries. The opt-in `ocio-reference` feature keeps CPU rendering and composed scalar adapters available only as diagnostic/test oracles; normal editor builds do not link OCIO. [Geometry](geometry.md#process-before-averaging) explains how rotated/reduced outputs reconstruct those pixels.

Exports use immutable settings snapshots, including ordered local processing, so subsequent editor changes cannot change an in-flight file. Local stages evaluate at each original source pixel before TIFF rendering or AP1-to-AP0 EXR averaging. Nonempty local snapshots identify the processing contract in TIFF/EXR metadata, even when their records are inactive; the metadata is not a reconstructible recipe or edit history. Normal editor admission validates complete local stacks against the device's supported domain; [photographic and broader-device acceptance](plans/local-adjustments.md) remains open. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. This is not a complete power-loss durability guarantee. Batch export and application export cancellation remain unfinished. [Export performance](plans/export-performance.md) tracks remaining attribution and broader acceptance. Matched reference/GPU measurements and bounded reuse checks establish workload-specific costs, not universal hardware speedups or photographic quality.

## Offline creative-look authoring

The [independent look catalog and authoring tool](../assets/looks/README.md) preserve versioned numerical material definitions and generate checked ACEScct/AP1 creative LUTs. Their fixed wide-reference inverse encodes one output-independent look intended to precede ordinary selected-output ACES. This is separate from the production renderer: no look selection, project persistence, preview/export insertion or EXR policy is implemented here. The current working/output diagram above is unchanged.

The catalog distinguishes selected/accepted content, unconfirmed candidates and shelved definitions. Those statuses do not establish physical material accuracy, final LUT approximation quality or source/asset release clearance. The owning catalog guide describes the source evidence, common evaluator, reproducible payload identities and photo-free checks.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- [Presentation](../src/platform/macos.rs): macOS layer tagging.
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, metadata and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer. Numerical reopening and tagged presentation do not establish photographic preference or Lightroom/Resolve interoperability.
