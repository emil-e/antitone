# What the image scopes measure

The waveform and vectorscope help judge the **rendered positive**, not the negative's density or the scene-linear working image. Their input is the selected sRGB or Display P3 output, before monitor conversion. This makes the plots useful for the photograph being graded while keeping them independent of the monitor on which the editor happens to run.

## Read the selected target's codes

Scopes measure the same ACES target as the preview, then use its native RGB primaries and transfer. The positive working-view path consumes the viewer's accepted target-linear raster directly: it does not grade, apply ACES or compose it a second time. Neither first converts P3 to sRGB — identical RGB codes in two targets need not represent identical colours.

The waveform plots encoded R/G/B or target-derived encoded luma against horizontal position in the final frame. Its vertical scale is percent of nominal encoded white — not nits, scene stops or broadcast IRE. The vectorscope derives two chroma coordinates from those same encoded channels and target luma weights; blue is shown to the right and yellow to the left. Primary/secondary directions come from the target equations, not from the photographic wheel's equally spaced artwork.

Invalid output samples are excluded and visibly counted, not repaired into ordinary bins. That diagnostic concerns the rendered boundary; it does not declare signed AP1 working pixels invalid.

## Measure the visible image

The plots measure the **visible image, approximately**, in the selected output target. Pan/zoom isolates the region being graded — a face, for example — while Fit covers the accepted composition. Waveform x follows left-to-right position within that visible region, never the letterbox or canvas. Crop, orientation and straightening use the same affine composition mapping as the viewer.

A roll/selection Auto-crop draft deliberately inspects the full oriented scan, including surrounding film — but not proposed crop pixels or outline/shading artwork. Apply or Cancel restores normal accepted-composition measurements. Ordinary manual Crop drafts instead suspend the scopes; neither workflow changes Analyze's sampling populations. The positive prototype shares the accepted physical-view raster, including the coarse underlay during navigation, and never measures only the detailed patch while the viewer shows additional content.

Reduction remains independently bounded to a 1024-pixel edge, so sparse samples can miss a tiny highlight or colour patch. Unsupported transparent coverage is excluded, not counted as black. Navigation/resize updates analysis when visible coverage changes; without an accepted canvas layout the plots show **Waiting for view**. During grade, pan/zoom or resize, hold the previous coherent trace until a compatible replacement is ready. Source inspection, temporary tool/reference views and unavailable/busy image states suspend scopes.

## Magnification changes the plot, not the grade

Vectorscope amplification narrows the displayed chroma range around neutral. The implementation re-bins cached rendered samples; it does not rerender the photograph or enlarge an old plot bitmap. Samples outside the plot remain counted rather than being pinned to its rim.

Small coloured reference boxes indicate nominal 100% RGB/CMY code colours; magnification can move them outside the plot. The unlabelled dashed ray is a conventional skin-tone hue heuristic, not a target every complexion should meet or an automatic correction. Complexion, light and grading can legitimately move skin away from it.

## Keep feedback coherent

Only one reduction/diagnostic readback is outstanding. Latest-result status requires exact processing content and visible-frame identity, so an old completion cannot become current: a different source/revision, accepted composition, target or vector scale must not inherit incompatible old bins; suspension clears them. Compatible holding uses logical source authority, not an immutable rendered-image pointer that changes on every edit.

The Scopes disclosure collapses the plot and suspends its work. A single header menu selects the plot type and vectorscope magnification. Plot controls do not change photographic processing: visibility and plot sizing are application preferences, while selection, magnification and GPU diagnostics are transient.

## Working on this area

[Scopes](../src/scopes.rs) owns target-native measurement and plot conventions; [GPU scopes](../src/gpu_scopes.rs) owns reduction and diagnostics; [editor scopes](../src/editor/scopes.rs) owns availability, scheduling and drawing.

Use [measurement/GPU tests](../tests/scopes.rs) and [final-frame geometry tests](../tests/scopes/geometry.rs) for their respective boundaries, and [authored scope widget checks](diagnostics.md#responsive-image-scopes) for composed controls.