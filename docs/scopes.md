# What the image scopes measure

The waveform and vectorscope help judge the **rendered positive**, not the negative's density or the scene-linear working image. Their input is the selected sRGB or Display P3 output, before monitor conversion.

This makes the plots useful for the photograph being graded while keeping them independent of the monitor on which the editor happens to run.

## Read the selected target's codes

Scopes measure the same ACES target as the preview, then use its native RGB primaries and transfer. The positive working-view path consumes the viewer's accepted target-linear raster directly: it does not grade, apply ACES or compose it a second time. Proxy presentation retains its shared-processing path. Neither first converts P3 to sRGB. Identical RGB codes in two targets need not represent identical colours.

The waveform plots encoded R/G/B or target-derived encoded luma against horizontal position in the final frame. Its vertical scale is percent of nominal encoded white—not nits, scene stops or broadcast IRE.

The vectorscope derives two chroma coordinates from those same encoded channels and target luma weights. Blue is shown to the right and yellow to the left. Primary/secondary directions come from the target equations, not from the photographic wheel's equally spaced artwork.

Invalid output samples are excluded and visibly counted, not repaired into ordinary bins. That diagnostic concerns the rendered boundary; it does not declare signed AP1 working pixels invalid.

## Measure the visible image

The plots measure the **visible image, approximately**, in the selected output target. Pan/zoom isolates the region being graded—a face, for example—while Fit covers the accepted composition. Waveform x follows left-to-right position within that visible image region, never the letterbox/canvas. Crop, orientation and straightening use the same affine composition mapping as the viewer.

The positive prototype shares the accepted physical-view raster, including the detail/coarse-underlay composite during navigation and the explicit original-resolution reference when displayed. It never measures only the detailed patch while the viewer shows additional underlay content. Reduction remains independently bounded to a 1024-pixel edge; sparse samples can miss a tiny highlight or colour patch. Unsupported transparent coverage is excluded, not counted as black photograph. Counts are not full-resolution TIFF/EXR statistics. Initial/pending proxies use approximate source-region sampling.

Navigation/resize updates analysis when visible coverage changes; equivalent requests reuse bins. No accepted canvas layout means **Waiting for view**. A completed navigation composite measures its full visible coverage, including the coarse underlay. While an incomplete GPU submission forces the retained accepted mapping to lag navigation, hold compatible traces instead of measuring unsupported replacement coverage. Source inspection, temporary tool/reference-source views, unaccepted crop drafts and unavailable/busy image states suspend scopes. The original-resolution comparison of the same positive is not a source-inspection tool and remains measurable.

## Magnification changes the plot, not the grade

Vectorscope amplification narrows the displayed chroma range around neutral. The implementation re-bins cached rendered samples; it does not rerender the photograph or enlarge an old plot bitmap. Samples outside the plot remain counted rather than being pinned to its rim.

Small coloured reference boxes indicate nominal 100% RGB/CMY code colours; their positions communicate both direction and magnitude. Magnification can move them outside the plot. The unlabelled dashed ray is a conventional skin-tone hue heuristic, not a target every complexion should meet or an automatic correction. Complexion, light and grading can legitimately move skin away from it.

## Keep feedback coherent

Only one reduction/diagnostic readback is outstanding. During grade, pan/zoom or sample-dimension updates, hold the previous coherent trace and its normalization until a compatible replacement is ready. Latest-result status still requires exact processing content and visible-frame identity, so an old completion cannot become current. Compatible holding uses logical source authority, not an immutable rendered-image pointer that changes on every edit. A different source/revision, accepted composition, target or vector scale must not inherit incompatible old bins; suspension clears them. RGB/luma selects cached planes without another reduction. No full-image readback or UI-thread GPU wait is introduced.

The Scopes disclosure collapses the plot and suspends its work. A single header menu selects vectorscope, RGB waveform or luma waveform and, for vectorscope, magnification; its readout shows the active plot and scale. The lower divider resizes the plot, with double-click restoring its default size. Outside-plot counts, invalid samples and errors remain visible in the plot; hover reveals the full message only when it is truncated.

Plot controls do not change photographic processing. Visibility and plot sizing are application preferences; plot selection, magnification and GPU diagnostics are transient.

## Working on this area

[Scopes](../src/scopes.rs) owns target-native measurement and plot conventions; [GPU scopes](../src/gpu_scopes.rs) owns reduction and diagnostics; [editor scopes](../src/editor_scopes.rs) owns availability, scheduling and drawing.

[Scope tests](../tests/scopes.rs) cover measurement and GPU agreement; [geometry tests](../tests/scopes/geometry.rs) cover final-frame mapping. The skin guide's exact convention and drawing constants live in code, not a second prose specification.

Use [authored scope widget checks](diagnostics.md#responsive-image-scopes) for composed controls and completed GPU reductions, not native-window or responsiveness acceptance. [Remaining acceptance](plans/completion.md#scopes) includes photographic interpretation and physical use; a synthetic plot test does not establish either.
