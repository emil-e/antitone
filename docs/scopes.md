# What the image scopes measure

The waveform and vectorscope help judge the **rendered positive**, not the negative's density or the scene-linear working image. Their input is the selected sRGB or Display P3 output, before monitor conversion.

This makes the plots useful for the photograph being graded while keeping them independent of the monitor on which the editor happens to run.

## Read the selected target's codes

Scopes render through the same ACES target as the preview, then use its native RGB primaries and transfer. They do not first convert P3 to sRGB or apply a second display transform. Identical RGB codes in two targets need not represent identical colours.

The waveform plots encoded R/G/B or target-derived encoded luma against horizontal position in the final frame. Its vertical scale is percent of nominal encoded white—not nits, scene stops or broadcast IRE.

The vectorscope derives two chroma coordinates from those same encoded channels and target luma weights. Blue is shown to the right and yellow to the left. Primary/secondary directions come from the target equations, not from the photographic wheel's equally spaced artwork.

Invalid output samples are excluded and visibly counted, not repaired into ordinary bins. That diagnostic concerns the rendered boundary; it does not declare signed AP1 working pixels invalid.

## Measure the whole accepted frame

The plots are labelled **Whole frame · Approximate**. They sample a bounded camera-RGB proxy through the accepted crop/orientation/straightening, rather than processing every source pixel.

Sparse proxy sampling can miss a tiny highlight or colour patch. Cropping cannot create detail absent from that proxy. The result is not guaranteed to match full-resolution TIFF/EXR statistics.

Viewer pan, zoom and window resizing do not change measured coverage. The settled viewer texture only represents a visible region, so it is not a substitute scope input. Source inspection, temporary reference views, unaccepted crop drafts and unavailable/busy image states suspend scopes.

## Magnification changes the plot, not the grade

Vectorscope amplification narrows the displayed chroma range around neutral. The implementation re-bins cached rendered samples; it does not rerender the photograph or enlarge an old plot bitmap. Samples outside the plot remain counted rather than being pinned to its rim.

RGB/CMY rim annotations indicate direction. The reference boxes indicate nominal 100% code colours. The dashed Skin ray is a conventional hue heuristic, not a target every complexion should meet or an automatic correction. Complexion, light and grading can legitimately move skin away from it.

## Keep feedback coherent

Scope work is asynchronous and bounded. While compatible results update, the previous coherent trace can remain labelled Updating. A different source, composition, target or vector scale must not inherit incompatible old bins.

Plot controls do not change photographic processing. Visibility and plot sizing are application preferences; plot selection, magnification and GPU diagnostics are transient.

## Working on this area

[Scopes](../src/scopes.rs) owns target-native measurement and plot conventions; [GPU scopes](../src/gpu_scopes.rs) owns reduction and diagnostics; [editor scopes](../src/editor_scopes.rs) owns availability, scheduling and drawing.

[Scope tests](../tests/scopes.rs) cover measurement and GPU agreement; [geometry tests](../tests/scopes/geometry.rs) cover final-frame mapping. The skin guide's exact convention and drawing constants live in code, not a second prose specification.

Use [scope diagnostics](diagnostics.md#responsive-image-scopes) for explicit performance/window exercises. [Remaining acceptance](plans/completion.md#scopes) includes photographic interpretation and physical use; a synthetic plot test does not establish either.
