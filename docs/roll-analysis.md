# Analyze roll: estimating a starting grade

Analyze turns a roll of camera scans into a shared colour and exposure baseline. It aims to make manual grading start from somewhere useful—not to finish every image or measure the film's physical response.

The distinction from [processing](processing.md) matters: processing transforms pixels using supplied settings; analysis **estimates those settings** from sampled images.

```text
eligible camera samples
  → Dmin and independent-channel Dmax
  → Dmin-to-Dmax density spans; one shared channel balance
  → picture log-chroma; coordinate-median shared Roll WB
  → per-photo luminance medians
  → shared Roll EV from their median; Photo EV zero
```

## 1. Choose what to sample

Analysis retains one regular raster of uncorrected original camera samples per source, currently capped at a 1024-pixel long edge. It stores RGB f32 camera codes and implicit original coordinates, without interpolation or upscaling. Accepted scan correction is evaluated at those coordinates when measurements are requested — never on a displayed preview or embedded camera JPEG.

Film and picture statistics select bounded two-dimensional strata on this source-wide grid, then mask them against reviewed support. Film samples supply the low-density reference; picture samples supply dense tails, WB and exposure statistics. Reviewed regions can move or resize without recapturing either population.

Spatial support excludes the holder and unsuitable material. Statistical anchors and median exposure statistics limit the influence of isolated extremes.

**Analyze** on the Roll tool shelf requests reusable [shared film/picture metadata and automatic sampling windows](spatial-metadata.md), then opens one review for the roll. Either Analyze or Auto-crop can prepare the shared description without accepting the other's edits. The viewer shows the full source in its Roll and Photo orientation, with both opening and picture areas — direct canvas selection, corner resizing and dragging; automatic film exclusions stay visible, and warning icons identify missing areas and the consequence of replacing existing Photo adjustments. Browsing, selection and reordering remain available during review. Reviewed areas follow source IDs, so reordering neither retargets the areas nor changes which photos Analyze fits. Browse to edit individual exceptions, then confirm the complete set with **Analyze roll**; Cancel leaves accepted areas and grading unchanged. Finishing or cancelling keeps the currently displayed photo.

The suggestions assume photographs from one actual roll/stock and a consistent scanning setup. Compatible captures strengthen one conservative holder interior — recurring edge-connected darkness supplies pooled exterior evidence, and an inscribed rectangle avoids rounded corners and terminal intrusions rather than merely intersecting four fitted rails. Opposing measured rails can support a translated photo exception; substantial inward departures stay local, and uncorroborated evidence can abstain and leave coverage bounds for review. Picture evidence remains independent inside that opening: own-source transitions, recurring material appearance and peer layout suggest frame edges and holder/gap exclusions, but weak transitions can supply statistical support without certifying a physical frame or crop. These estimate safe statistical support, not an exact physical aperture or every pixel's material identity.

Authored windows replace corresponding automatic area suggestions without moving shared physical boundaries. Opening edits remain per-photo exceptions. Matching accepted support is reused; incomplete support or older automatic local-only windows use the finished shared suggestions. Missing automatic areas stay visible for manual review rather than triggering another detector.

Selected original sample sites must satisfy film/material/holder support, including its source-scaled inward safety band. Picture statistics additionally obey nominal picture boundaries, even when an authored window extends farther; film-base statistics may include valid film outside the picture. Camera limits add no invented film edge. These sampling safeguards do not trim photographic crops. Previews supply spatial evidence, never the numerical samples. Every ordered photo must supply adequate picture statistics; analysis never silently drops a difficult photo to make the roll succeed.

## 2. Estimate the two anchors

**Dmin** uses the high-transmission film samples pooled across the roll. For each channel, it averages the interpolated 98–99.5% quantile band. A band avoids letting one brightest value determine the reference, while looking toward the least dense sampled film.

**Dmax** uses each channel's 0.5% picture quantile, after equal-count sampling gives photos equal population weight. Each channel is estimated independently: requiring a single RGB pixel to supply all three anchors would couple the result to that object's colour.

The quantiles use finite camera readings, including near-zero values. The resulting anchors must satisfy `Dmin > Dmax > 0` in every channel so inversion can use their ratios. A damaged dense tail or holder contamination can still bias an otherwise valid result.

## 3. Estimate channel balance from the anchors

Each channel’s span is `ln(Dmin / Dmax)`, using the same camera transmissions that anchor inversion. The balance uses only the relative widths of the three spans; it does not move either anchor or require a shared pixel.

Channel gammas are proportional to their spans: a channel that travelled farther gets a larger divisor when inverting. Their geometric mean stays at the selected global gamma. If the spans are unusable or suggest a balance outside the control range, Analyze keeps the incoming balance. Different scenes or capture conditions can still set the two ends of a span; the estimate is not a measured film slope.

## 4. Set shared photographic WB

After setting gamma, Analyze initially estimates WB and exposure from the full eligible picture population. The [photographic graph](../src/photographic/graph/camera_response.rs) uses that provisional grade to build a target-linear sRGB endpoint. Analyze orients and encodes those pixels for CPU segmentation. The segmentation vetoes confident sky, vegetation and earth at original WB sample sites; it does not treat other materials as neutral. Chroma comes from the original camera samples after inversion and that photograph's camera mapping, not from the previews.

Each photo with at least 256 retained sites contributes one negative mean log-chroma vote. The coordinate-wise median of these votes sets the shared Roll WB, limited to the control range. If fewer than half the photos can vote, Analyze keeps the unfiltered grade. If no photo has usable colour measurements in the initial fit, Analyze declines to install a grade.

## 5. Set shared roll exposure

Each photo supplies the median of its eligible positive AP1 luminances after curve fitting and Roll WB, without the semantic veto. The median of those frame medians supplies the roll key, giving each photo equal influence regardless of sample count:

```text
roll_key = median(frame_luminance_medians)
Roll_EV = log2(0.18 / roll_key)
```

Both medians are taken in linear luminance; even populations average their two middle values. This reduces the influence of bright and dark tails without clipping working pixels. The 0.18 target is a starting-brightness choice, not a claim that a scene's median surface is a gray card.

Every Photo EV starts at zero. The common exposure preserves frame-to-frame brightness differences, using scene-metered film exposure as a practical prior. Individually normalizing frames can darken foregrounds beneath bright skies or lift dark scenes and their shadow noise. Manual Photo EV remains available for exceptions.

The shared baseline still depends on the roll's scenes and consistent scanning exposure. Predominantly dark or bright rolls can need manual Roll EV adjustment; the statistic cannot infer photographic intent.

## Review uses finished detection

The document-placed [Analyze tool](../src/editor/analysis.rs) owns its unfinished task, source-keyed sampling edits, gestures and temporary view. Its task awaits shared detection and review confirmation before fitting. Confirmation snapshots the accepted document, areas and displayed source; fitting remains provisional until validated installation. Browsing replaces the photo visit while the tool's drafts survive ([architecture](architecture.md#scopes-nested-disposable-ui-state) owns that model). Reusable original sampling inputs belong to the document; active fitting retains its own input snapshot.

Shared preparation captures the camera raster while each original is available, then releases its lease. Document-owned rasters are keyed by source development, independent of reviewed regions, flat-field choices and the optional full-image LRU. Closing the document or removing a source releases them. Payload is at most 12 MiB per source, excluding ownership overhead; reported retained bytes are not process memory.

Before numerical fitting, Analyze acquires every captured photograph's camera raster, reusing matching retained evidence and resolving missing interpretation facts through source acquisition. Each raster retains its photograph's acquired camera-to-AP1 matrix for WB, metering, classifier rendering and final numerical admission; RGB anchors and Roll settings are shared. Fitting owns bounded original-camera rasters rather than full-image leases. The fit reuses the Dmin and Dmax measurements for channel balance, without requesting another picture sample population.

Review needs only geometry and material classification; it does not develop RAWs or run image-based refinement. Within an open document, Auto-crop reuses ready regions without acquiring pixels; Analyze collects missing camera rasters on demand. Reopening starts without regions, and either feature can trigger shared preparation.

## Install the baseline, not a partial result

Analyze replaces calibration, channel balance, Roll WB/EV and all ordered Photo grading. Photo exposure and WB start at zero; contrast, pivot, colour preservation, saturation and density return to their actual defaults. Each analyzed photo's layer stack is replaced with one ordinary global layer at default settings; its non-reusing ID allocator is retained.

Every run reviews sampling areas. Nondefault layer settings or a stack other than the initial global layer add an explicit reset warning to that same review, not a second modal; the warning names layer replacement, and the accepted installation is one undoable history entry. Complete numeric entry to include it in the accepted settings captured at confirmation. Switching tools retires the unfinished analysis. Selected global gamma/response, accepted correction and Roll/Photo orientation and crop are preserved; no review draft is saved.

Fitting validates the complete proposal—including every photo's resulting numerical domain—using each photo's acquired camera matrix and captured correction before one atomic installation of both support and grading. The segmentation branch omits the project’s output looks. Acceptance installs that checked result through a live edit, without repeating rendering admission against viewing-resolved facts. Failure or cancellation preserves accepted processing and multi-selection. Replacing Analyze leaves the displayed photo in place for the next tool. Explicit cancellation waits for worker acknowledgement; tool replacement retires the action immediately while native/GPU work cleans up cooperatively.

## Where the estimate can fail

Faint/off-image or scene-confounded boundaries can defeat area suggestions; holder fuzz and narrow rebates remain uncertain at bounded evidence resolution. A damaged frame can still dominate independent Dmax tails, and correlated scene colour can bias the channel spans and WB. Review the sampling areas and check the resulting grade across the roll before relying on it.

## Working on this area

- [Original camera evidence](../src/scan/analysis/input.rs), [GPU camera sampling](../src/roll_analysis/gpu/sampling.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Anchor span estimator](../src/roll_analysis/anchor_spans.rs) and [roll analysis](../src/roll_analysis.rs): the editor proposal and shared anchor/EV measurements.
- [Spatial hypotheses](../src/scan_regions.rs), [shared preparation](../src/editor/spatial/preparation.rs) and [document retention](../src/editor/document_scope.rs): reusable source-owned support and original sampling inputs.
- [Editor Analyze](../src/editor/analysis.rs): tool interaction, review assembly, awaited preparation, cancellation acknowledgement and atomic installation of reviewed areas with grade.
- [Captured sampling/fitting](../src/editor/analysis/fit.rs): original-site eligibility, GPU measurements and validated numerical proposals.
- [Scalar seed tests](../tests/roll_analysis.rs), [GI tests](../src/roll_analysis/gi/tests.rs), [spatial GPU sampling tests](../src/roll_analysis/gpu/sampling/spatial/tests.rs) and [editor tests](../src/editor/analysis/tests.rs): synthetic recovery, abstention, original-coordinate eligibility and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises.