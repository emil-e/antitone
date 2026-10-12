# Display-referred image scopes

## Purpose and availability

Scopes judge the rendered positive photograph, not scene-linear AP1, film calibration, physical luminance or broadcast compliance. One collapsible slot above Roll/Photo controls switches between waveform and vectorscope, initially vectorscope at 1×. Output stays pinned; showing/switching scopes changes grading scroll space, not the viewer canvas. RGB/luma waveform modes share the same slot height. Vectorscope magnification is 1×/2×/4×/8× with a 1× reset.

Plot height defaults to 85% of sidebar content width. A visible divider below the caption resizes it; double-click resets the default. The chosen height/width ratio is transient and shared across plots, so widening the sidebar also enlarges a manually sized scope. Short windows cap plot height to leave grading scroll space; the plot minimum is 96 points. Shrink/collapse scopes when more grading space is needed.

Scopes are suspended for developed-source view, missing calibration/image, temporary reference/flat-field inspection and busy image/document operations. Source inspection EV never enters scope data. Visibility, plot selection, magnification, GPU resources and diagnostics are transient: no project dirty/schema/export change.

## Colour convention

Use the actual selected 100-nit sRGB or Display P3 ACES output, in its native RGB primaries/D65 white, then its segmented sRGB transfer. No target-to-sRGB or monitor conversion, second DRT, extra gamut clamp or TIFF quantization. The selected target is labelled; equal native RGB codes in different targets are not asserted to be equal colours.

Linear target samples must be finite and within 0…1. Encode using the existing target transfer. Invalid samples are excluded, counted and visibly reported; they are not repaired into ordinary bins. This output boundary does not clamp or validate upstream signed/HDR working data or AP0 EXR.

Let `Kr/Kg/Kb` be the normalized Y row of the selected target's `rgb_to_xyz` matrix. Define encoded-code luma `Y′ = Kr R′ + Kg G′ + Kb B′`. Display P3 uses its own weights, not silently substituted sRGB/Rec.709 coefficients. Vertical waveform labels are percent of nominal encoded white (0…100), not nits, stops or broadcast IRE.

Define target-native chroma:

```text
Cb = (B′ − Y′) / (2*(1−Kb))
Cr = (R′ − Y′) / (2*(1−Kr))
```

Rotate the right/up chroma plane orthonormally so blue lies at 3 o'clock, yellow at 9, and RGB/CMY proceed clockwise. Other primary/secondary directions come from these equations, not equally spaced colour-wheel artwork or photographic-control adapters. SDR RGB cube vertices define the 100% reference markers. The base half-extent is 1.08 times the largest reference chroma length. Rings have chroma radii 0.25/0.5; their drawing and reference boxes scale with amplification. RGB/CMY direction ticks and letters stay on the rim at every gain, with target-derived angles. Rim annotations indicate hue direction only, not a fixed chroma/saturation value. Boxes remain 100% code references and are omitted when outside the plot or overlapping a direction label. There is no broadcast-compliance line.

Amplification narrows the displayed chroma range about neutral without changing grade or measured values. Re-bin cached rendered samples for detail; don't merely magnify a bitmap. Offscreen samples are counted and reported as a percentage of valid samples, not pinned to plot edges. Density normalization includes those offscreen samples. Vector drawing compensates for chroma bin area shrinking by amplification²: fixed log-density mapping uses `(count / all_valid_samples) * 10000 * amplification²`, then `clamp(0.18 * log2(1 + density), 0, 1)`. This prevents zoom from dimming a smooth distribution merely by spreading it over more bins, without auto-normalizing to visible samples or each edit's peak. Default vector intensity is brighter; grade, chroma measurements, counts and waveform mapping are unchanged. Vector/luma traces are bright near-white; graticules are subdued blue-purple, distinct from the traces. RGB overlay retains channel colours; reference markers/labels use corresponding RGB/CMY UI colours.

## Skin-hue reference

The vectorscope includes an amber dashed ray labelled Skin, from neutral toward a conventional positive-I, Q=0 **heuristic**. Its direction is fixed through zoom and projected into the selected target's native chroma basis. It does not certify skin-colour correctness, define a standard Display P3 skin axis, or reproduce an asserted exact Resolve angle. Complexion, illumination, camera response, white balance and grading can all move skin away from it; no detection, selection or correction is performed.

Declared convention: use the legacy encoded-RGB [YIQ coefficient set](https://en.wikipedia.org/wiki/YIQ): `y = (0.299, 0.587, 0.114)`, `q = (0.2115, −0.5227, 0.3112)`, `i = (0.5959, −0.2746, −0.3213)`. The seed direction is `d = y × q`, which has zero Y/Q and positive I. Apply these coefficients deliberately to **native encoded RGB code directions**, then draw `normalize(Basis::vector(d))`. This is not an NTSC primaries/transfer conversion or a colourimetric sRGB↔P3 reference; it does not introduce a second measurement transform. Neutral offsets and positive magnitude scaling preserve the ray. Target changes rebuild the projected direction; pixel statistics/processing are unchanged.

[Resolve's Colorist Guide](https://documents.blackmagicdesign.com/UserManuals/DaVinci-Resolve-18-Colorist-Guide.pdf) describes a skin-tone indicator, but the inspected wording does not establish a universal numerical angle. [Display P3's definition](https://registry.color.org/rgb-registry/displayp3) establishes RGB primaries/D65/transfer, not a skin-hue axis. The plot tooltip distinguishes the heuristic and direction annotations from measurements/100% references.

## Spatial coverage and approximation

The implemented input is the existing maximum-dimension-1024 nearest-centre whole-active-area RGB16 proxy, processed/rendered with actual positive parameters and accepted source-coordinate flat-field correction. Labels say **Whole frame · Approximate**, with sample dimensions. Sparse samples can miss small highlights/colours; scopes are not full-source statistics and never supply picks or exports.

Waveform x is normalized **frame-raster** x at pixel centres, never canvas x or visible-region x. Accumulate all raster y, with 256 horizontal columns × 256 level bins in four R/G/B/Y′ planes. Vectorscope uses a 256×256 density raster. Endpoints map into valid edge bins. Waveform density is normalized per contributing column; vector density is normalized across all valid image samples and accounts for chroma bin area in drawing. Fixed logarithmic drawing preserves relative concentration without per-edit axis auto-scaling.

This branch currently has identity geometry: active source content and final frame coincide. Existing settled viewer images cover only a regional `ViewRequest` and are **not scope inputs**. Viewer pan/zoom/Fit/resize/DPI changes leave scope coverage and values unchanged.

The concurrent `plan/flip-crop-rotate` worktree owns source → oriented-source → final-frame → raster/canvas mappings. On rebase, adapt the editor's `scope_input` boundary to the exported composition through its canonical composition/raster/coverage utilities, include accepted geometry in identity, and handle crop drafts explicitly. Preserve original source centres for flat-field processing. A source bounding box cannot substitute for rotated frame coverage. Combined geometry acceptance has not been performed here.

## GPU lifecycle and resources

`GpuImage::prepare` is shared with the viewer: either callback can prepare the whole proxy first without duplicate processing/ACES. Scope identity uses image ownership, source/correction revision, exact processing parameters, target and bin/amplification convention. GUI geometry and trace drawing do not invalidate processing. RGB/luma switching selects cached planes and performs no new reduction/readback.

Only one reduction/16-byte asynchronous diagnostic readback is outstanding. Rapid edits and hide/source transitions do not bypass that bound. The live UI supplies the latest request after completion; obsolete diagnostics never reinstall. While grading edits await a reduction, retain the previous coherent trace labelled Updating. This reuse requires identical image/source revision, target and accumulation/amplification convention; RGB/luma can select the same cached planes. Never pair old bins with a new target/vector scale or another source. Hide/suspension clears the cache, and incompatible presentation changes omit the old trace until matching bins exist. There is no full-image readback, CPU interactive fallback or UI-thread GPU wait. Nonblocking polling/repaint stops when diagnostics finish; unchanged idle repaints do no scope compute.

Retained scope pixel storage is 1,048,576 bytes of bins plus 1,040 bytes of totals, shared between plot modes, independent of photograph/window dimensions. One 16-byte staging buffer and small uniforms/bindings are additional; driver/in-flight rendering overhead is not an application-wide budget claim. No additional RAW or wide retained source/working image is introduced. Source raster dimensions/counts are checked against zero/u32 overflow.

Full-source accumulation, broader hardware/monitor validation and geometry integration remain separate work. [Plan, checks and measured limits](../plans/vectorscope-and-waveform.md).
