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

Generated GPU algorithms and tables implement the same rendering as the CPU reference/export path. The shared renderer adapts resource layout without rewriting the transform; its immutable resources serve viewer, thumbnails, scopes and refinement. This is why changes must be checked across both paths and their composed consumers rather than adjusted in a shader alone. [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md) own generation and provenance.

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

Both outputs use accepted composition and full-resolution source pixels, never a preview buffer or tool draft. [Geometry](geometry.md#process-before-averaging) explains how rotated/reduced outputs reconstruct those pixels.

Exports use immutable settings snapshots, including ordered local processing, so subsequent editor changes cannot change an in-flight file. Local stages evaluate at each original source pixel before TIFF rendering or AP1-to-AP0 EXR averaging. Nonempty local snapshots identify the processing contract in TIFF/EXR metadata, even when their records are inactive; the metadata is not a reconstructible recipe or edit history. Normal editor local export remains blocked by [feature admission](plans/local-adjustments.md), while library export tests exercise it. Same-directory temporary writes protect existing destinations, and replacement needs explicit confirmation. This is not a complete power-loss durability guarantee. Batch export and application export cancellation remain unfinished.

## Working on this area

- [Rendering](../src/rendering.rs) and [native transform definition](../native/rendering_transform.h): shared OCIO configuration.
- [Presentation](../src/platform/macos.rs): macOS layer tagging.
- [Export](../src/export.rs) and [profiles](../src/profiles.rs): file encoding, metadata and destination safety.
- [Rendering tests](../tests/rendering.rs), [export tests](../tests/export.rs) and [geometry exports](../tests/geometry_export.rs): reference agreement and numerical reopening.

Use the [rendering and output diagnostics](diagnostics.md#rendering-gamut-and-icc) for target/ICC checks, and [presentation diagnostics](diagnostics.md#macos-presentation) for the native layer. Numerical reopening and tagged presentation do not establish photographic preference or Lightroom/Resolve interoperability.
