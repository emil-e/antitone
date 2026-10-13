# Analyze roll: photographic acceptance

## Scope

Evaluate Analyze as a useful shared-colour and individual-exposure starting point, followed by ordinary manual grading. It is not an automatically finished or physically calibrated grade. Preserve legitimate blue shade/water and coloured lighting.

Implemented behavior belongs in the [processing](../contracts/processing-model.md), [controls](../contracts/editor-controls.md#analyze-roll) and [persistence](../contracts/project-state.md#photographic-roll-wb) contracts. Verification commands and maintained diagnostics belong in the [probe guide](../build-and-probes.md#analyze-roll).

## Unfinished acceptance

- Inspect the current initializer across corrected and uncorrected scan sets, including noisy shadows, coloured lighting and sky-dominated scenes. Evaluate the need for manual finishing without treating a statistical prior as measured film/process truth.
- Exercise conditional confirmation, numeric focus loss, cancellation, manual Roll/Photo WB and settled preview retention with physical controls. Scripted corrected/uncorrected area/Analyze checks submit visible frames and settle both targets, but do not establish physical interaction or photographic appearance.
- Reopen TIFF/AP0 EXR and saved projects with corrected and uncorrected sources; assess photographic appearance separately from numerical/state roundtrips.
- Calibrated monitor/colour, physical gestures, actual macOS 14 and broader cameras/devices remain separate acceptance.

## Concrete limits

- Automatic support is order-dependent: DSC01631 first can select a partial opening, while DSC01636 first finds the full opening and materially changes the grade. Explicit geometry wins; there is no whole-camera fallback. A general support fix is not selected.
- Damaged DSC04492 dominates independent Dmax tails and biases its scan set. Diagnostic exclusion establishes causality, not permission to omit or repair the source.
- Tonal-drift fitting can be biased by systematic scene/material/illumination correlations. Shared WB and exposure remain statistical priors; Photo WB is manual rather than individual gray-world fitting.
- Native/GPU phases may delay cancellation acknowledgement. Numerical and headless checks do not establish physical or photographic acceptance.

## Non-goals and next action

Do not add material masks, source omissions/repair, clamps, pedestals, automatic per-photo WB, a CPU interactive fallback or weakened numerical gates. Broad mask research needs a concrete failure and separately agreed scope. Generated reports and photographs stay private/ignored; do not restore archives or execution records.

Next: exercise conditional confirmation and manual grading with physical controls, then scope corrected cross-roll/export photographic acceptance. Address support ambiguity and damaged-tail handling separately, without silently excluding sources.
