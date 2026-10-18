# Analyze roll: photographic acceptance

## Scope

The initializer and its first sampling-area review are accepted for current use. Evaluate them as a useful shared-colour and individual-exposure starting point, followed by ordinary manual grading. It is not an automatically finished or physically calibrated grade. Preserve legitimate blue shade/water and coloured lighting.

The [Analyze guide](../roll-analysis.md) explains sampling-area review, the estimator and atomic installation; [projects](../projects.md) explains saved state. Commands and maintained diagnostic recipes belong in [diagnostics](../diagnostics.md#analyze-roll).

## Unfinished acceptance

- Exercise the single roll-level review with real photographs and physical controls: source navigation, compact overview, opening versus picture support, per-photo exception edits, numeric focus, reset warning and cancellation during preparation/estimation. Automated normal/narrow and native-worker checks do not establish physical interaction or photographic appearance.
- Inspect the shared conservative holder interior against the frozen local-detector baseline across corrected and uncorrected scan sets, including faint underexposed content, dense film, narrow rebates/fuzz, shifted holders, cuts, perforations and scene-confounded transitions. Keep difficult photos in the complete roll. Assess wanted-content retention and unwanted-material admission separately from proposal availability.
- Assess noisy shadows, coloured lighting and sky-dominated scenes after Analyze and ordinary manual finishing. Do not select geometry or tune the estimator to reproduce preferred example grades.
- Reopen TIFF/AP0 EXR and saved projects with corrected and uncorrected sources; assess photographic appearance separately from numerical/state roundtrips.
- Calibrated monitor/colour, physical gestures, actual macOS 14 and broader cameras/devices remain separate acceptance.

## Deferred sampling and endpoint work

Keep these separate from completion of the accepted review; no implementation has been selected:

- Make existing rejection counts and endpoint-contributor diagnostics more accessible; consider spatial overlays showing contributing material.
- Evaluate retaining all eligible Dmax samples with explicit weighting instead of equal-count thinning. Weighting does not solve contamination; genuinely dense photographs must still influence the roll endpoint.
- Measure legitimate signal lost to the conservative 8×8 neighbourhood veto before changing it. Preserve capture/development/floor safety invariants.
- Investigate damaged-film tails, including DSC04492. Better geometry can exclude non-film material but cannot establish that film is undamaged. Diagnostic exclusion establishes influence, not permission to omit or repair the source.

[Remaining region consumers](region-detection.md) own bare-light correction support and future crop acceptance.

## Concrete limits

- Opening and picture rectangles are statistical hypotheses. Missing sides may use source/opening coverage constraints; coarse material evidence and finer terminal runs do not establish original-pixel segmentation, physical aperture accuracy or crop correctness.
- [Missing-area warning indicators](../roll-analysis.md#1-choose-what-to-sample) direct attention; their absence does not certify the suggestions. Confirming areas does not certify material identity or pixel eligibility. Numerically valid contamination and damaged dense film can still influence independent Dmax tails.
- Tonal-drift fitting can be biased by systematic scene/material/illumination correlations. Shared WB and exposure remain statistical priors; Photo WB is manual rather than individual gray-world fitting.
- Native/GPU phases may delay cancellation acknowledgement. Numerical and headless checks do not establish physical or photographic acceptance or physical flat-field-reference compatibility.

## Non-goals and next action

Do not add source omissions/repair, clamps, pedestals, automatic per-photo WB, a CPU interactive fallback or weakened numerical gates. The implemented spatial baseline is bounded support under the declared-negative roll contract, not a general material classifier or automatic crop/straightener. Further detector sophistication needs a demonstrated real-use failure, not continued tuning to the same corpus. Generated reports and photographs stay private/ignored; do not restore archives or execution records.

Next: reproduce the reported DSC04240 opening failure with the complete DSC04234–DSC04245 roll (not in the current private corpus), then exercise review and manual grading with physical controls, then scope corrected cross-roll/export photographic acceptance. Keep area confirmation and the unchanged independent numerical rejections complementary; neither replaces the other.
