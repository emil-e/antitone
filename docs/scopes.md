# What the image scopes measure

The waveform and vectorscope help judge the **displayed image**, including source inspection and temporary tool views. They measure the same ACES output as the canvas, before OS monitor conversion.

## Read the preview target

The waveform plots R/G/B or their weighted sum against horizontal position. For sRGB and Display P3, it plots transfer-encoded values and luma as a percentage of nominal encoded white. For Rec.2020 HDR, it plots linear channels and luminance in nits, up to the rendering target's peak.

The vectorscope derives two chroma coordinates from the same channels and target weights; blue is shown to the right and yellow to the left. Primary and secondary directions come from the target equations.

## Measure the visible image

The plots measure the **visible image, approximately**, in the selected output target. Pan/zoom isolates the visible region — a face, for example — while Fit covers the displayed composition. Waveform x follows left-to-right position within that visible region, never the letterbox or canvas. Crop, orientation and straightening use the same affine composition mapping as the viewer.

Scopes follow the image beneath tool artwork. Auto-crop shows the full oriented scan; manual Crop can show source content beyond its draft frame. Measurement pickers leave the displayed image unchanged; reference views supply their own source image. Navigation composites include the same coarse underlay and detailed patch as the canvas. These viewing choices leave Analyze's sampling populations unchanged.

Reduction remains independently bounded to a 1024-pixel edge, so sparse samples can miss a tiny highlight or colour patch. The plots omit uncovered and unusable output samples without treating them as black. Navigation and resizing update the analysis when visible coverage changes. During grading or navigation, the previous coherent trace holds until a compatible replacement is ready. Without a displayed image, the plot stays blank and scope work is suspended.

## Magnification changes the plot, not the grade

The vectorscope's zoom buttons and vertical wheel input step from 1× to 8×. Zoom narrows the chroma range around neutral by re-binning cached rendered samples. Samples beyond the enlarged plot's range are omitted from the trace but still contribute to its density normalization.

Small coloured boxes mark full-scale RGB/CMY colours; magnification can move them outside the plot. The dashed ray on SDR plots is a conventional skin-tone hue heuristic. Complexion, light and grading can legitimately move skin away from it.

## Keep feedback coherent

Only one reduction is outstanding. Completion requests a repaint so scopes can measure the latest view. Bins and density totals stay on the GPU. Current results require an exact image and mapping match; older traces may hold across grade or navigation changes for the same source, composition and plot settings. Hiding scopes retires the trace without queueing more work.

The Scopes disclosure collapses the plot and suspends its work. The header switches between the vectorscope and waveform; the waveform's settings choose RGB or luma. Visibility and plot size are application preferences; plot selection, waveform mode and zoom are transient.

## Working on this area

[Scopes](../src/scopes.rs) owns target-native measurement and plot conventions; [GPU scopes](../src/gpu_scopes.rs) owns reduction and completion; [editor scopes](../src/editor/scopes.rs) owns availability, scheduling and drawing.

Use [measurement/GPU tests](../tests/scopes.rs) and [final-frame geometry tests](../tests/scopes/geometry.rs) for their respective boundaries, and [authored scope widget checks](diagnostics.md#responsive-image-scopes) for composed controls.