# Analyze roll: estimating a starting grade

Analyze turns a roll of camera scans into a shared colour and exposure baseline. It aims to make manual grading start from somewhere useful—not to finish every image or measure the film's physical response.

The distinction from [processing](processing.md) matters: processing transforms pixels using supplied settings; analysis **estimates those settings** from sampled images.

```text
eligible camera samples
  → Dmin and independent-channel Dmax
  → sparse-picture seed curves and exposure
  → ordinary GI evidence; one shared channel-curve fit
  → refreshed GI evidence; coordinate-median shared Roll WB
  → per-photo luminance medians
  → shared Roll EV from their median; Photo EV zero
```

## 1. Choose what to sample

The analysis uses original developed camera pixels, with accepted scan correction applied — never a displayed preview or an embedded camera JPEG. Sparse film samples supply the low-density reference; sparse picture samples supply dense tails, a seed grade and exposure statistics. Sampling batches selected 8×8 camera cells, retaining their original coordinates for correction. Local numerical checks reject precision-floored footprints before retaining their original centre pixels.

GI also needs spatial neighbourhoods. After confirmation, fitting acquires each original and captures a regular picture raster, currently limited to a 512-pixel long edge. Each site retains its corrected original camera centre and footprint eligibility; no interpolation reconstructs pixels from the sparse patches. Only bounded camera evidence survives each source acquisition.

Spatial support excludes the holder and unsuitable material; numerical checks establish usable developed values. Statistical anchors and median exposure statistics limit the influence of isolated extremes.

**Analyze** on the Roll tool shelf requests reusable [shared film/picture metadata and automatic sampling windows](spatial-metadata.md), then opens one review for the roll. Either Analyze or Auto-crop can prepare the shared description without accepting the other's edits. The viewer shows the uncropped source with both opening and picture areas — direct canvas selection, corner resizing and dragging; automatic film exclusions stay visible, and warning icons identify missing areas and the consequence of replacing existing Photo adjustments. Selection and browsing retain their ordinary behavior and never narrow this roll-wide review. Browse to edit individual exceptions, then confirm the complete set with **Analyze roll**; Cancel leaves accepted areas and grading unchanged. Finishing or cancelling keeps the currently displayed photo.

The suggestions assume photographs from one actual roll/stock and a consistent scanning setup. Compatible captures strengthen one conservative holder interior — recurring edge-connected darkness supplies pooled exterior evidence, and an inscribed rectangle avoids rounded corners and terminal intrusions rather than merely intersecting four fitted rails. Opposing measured rails can support a translated photo exception; substantial inward departures stay local, and uncorroborated evidence can abstain and leave coverage bounds for review. Picture evidence remains independent inside that opening: own-source transitions, recurring material appearance and peer layout suggest frame edges and holder/gap exclusions, but weak transitions can supply statistical support without certifying a physical frame or crop. These estimate safe statistical support, not an exact physical aperture or every pixel's material identity.

Authored windows replace corresponding automatic area suggestions without moving shared physical boundaries. Opening edits remain per-photo exceptions. Matching accepted support is reused; incomplete support or older automatic local-only windows use the finished shared suggestions. Missing automatic areas stay visible for manual review rather than triggering another detector.

Before numerical estimation, complete 8×8 footprints on the canonical source grid must satisfy film/material/holder support, including its source-scaled inward safety band. Picture statistics additionally obey nominal picture boundaries, even when an authored window extends farther; film-base statistics may include valid film outside the picture. Camera limits add no invented film edge. These sampling safeguards do not trim photographic crops. Previews supply spatial evidence, never the numerical samples. Every ordered photo must supply adequate picture statistics; analysis never silently drops a difficult photo to make the roll succeed.

## 2. Estimate the two anchors

**Dmin** uses the high-transmission film samples pooled across the roll. For each channel, it averages the interpolated 98–99.5% quantile band. A band avoids letting one brightest value determine the reference, while looking toward the least dense sampled film.

**Dmax** uses each channel's 0.5% picture quantile, after equal-count sampling gives photos equal population weight. Each channel is estimated independently: requiring a single RGB pixel to supply all three anchors would couple the result to that object's colour.

These are useful statistical anchors, not guaranteed observations of clean film base and maximum physical density. A damaged dense tail or holder contamination can still bias them.

## 3. Fit a shared colour trend across tones

A constant WB adjustment cannot fix every shadow-to-highlight colour trend. Channel gamma balance can, because its effect varies with density. The analysis therefore fits curves first and WB afterward.

The sparse picture population first supplies seed curves and exposure. At those curves, ordinary Grayness Index (GI) ranks local channel-proportion stability in native positives, without applying photographic WB. The current selector follows the Gaussian-gradient executable reference for [Qian et al.'s GI](https://openaccess.thecvf.com/content_CVPR_2019/html/Qian_On_Finding_Gray_Pixels_CVPR_2019_paper.html): 7×7 smoothing, complete valid neighbourhoods, dark/flat rejection and the lowest 0.1% score quota, including threshold ties. Seed EV supplies its brightness gain and stays fixed through selection and the subsequent WB refresh.

Selected original camera vectors supply one further gamma fit. Each photo's selected population is divided into shadow, middle and highlight bands using its positive luminance ranks with zero gamma balance. These memberships stay fixed during fitting, so a candidate cannot improve its score simply by moving samples into different bands. Every photo must supply complete gamma evidence; fitting does not iterate GI selection and curves.

The [production forward model](../src/gpu/analysis.rs) evaluates shared photographic GPU components for every candidate, not just the initial samples. Captured camera vectors use an index grid after original-footprint eligibility; they need no image reconstruction. CPU ranking, reduction and search consume these f32 responses, while the public scalar estimator remains an independent f64 reference.

For each candidate:

1. Invert each sample, then average the positives within each band.
2. Express the band's mean in log-opponent Bradford coordinates.
3. Compare shadow and highlight colour with the middle band.
4. Average those differences across photos, then penalize the remaining shared drift.

If `z` denotes those two colour coordinates, the objective is:

```text
J = |mean_photos(z_shadow − z_middle)|²
  + |mean_photos(z_highlight − z_middle)|²
```

### Why photographic WB cancels

Bradford WB multiplies cone responses. Taking logarithms turns those gains into an additive offset shared by all bands, and subtracting two band coordinates removes that offset. Exposure likewise adds a common log gain and does not change the differences. This lets the fit ask “which curves reduce tonal colour drift?” without simultaneously asking “what should the overall colour balance be?”

### Why average before squaring

Imagine one photograph with warm highlights and cool shadows, and another with the opposite scene colours. Penalizing each photo separately would pressure both toward neutral bands. Averaging their differences first lets opposing scene trends cancel; the fit targets a trend shared by the roll instead.

That is still an assumption. A roll dominated by the same materials or coloured lighting can have a genuine common scene trend; the optimizer cannot prove whether it comes from the film response or the scenes. A bounded search compares valid starting balances and refines them within the existing controls; undefined or out-of-range candidates are rejected, not clamped. Search mechanics and stopping criteria live in [the fitter](../src/roll_analysis/gamma.rs).

## 4. Set shared photographic WB

With the fitted curves fixed, fresh ordinary GI selection supplies one native-positive mean per photo. Camera-to-AP1 conversion and Bradford neutralization yield a candidate WB. The shared Roll WB is the coordinate-wise median of admitted candidates, averaging the middle pair for even counts. A photo whose GI evidence or WB correction is unusable abstains from this aggregation, not from gamma fitting or the resulting grade. Analyze reports partial WB participation; rejection reasons are logged by source. No admitted candidate means failure rather than a substitute grade.

GI evidence is not a measured neutral target: coloured material and coloured illumination can also have stable channel proportions. Roll aggregation limits some frame-specific extremes, not shared scene bias. Photo WB starts at zero and remains manual; automatically applying individual estimates would too readily remove blue shade, coloured lighting or a scene's dominant material colour.

## 5. Set shared roll exposure

Each photo supplies the median of its eligible positive AP1 luminances after curve fitting and Roll WB. The median of those frame medians supplies the roll key, giving each photo equal influence regardless of sample count:

```text
roll_key = median(frame_luminance_medians)
Roll_EV = log2(0.18 / roll_key)
```

Both medians are taken in linear luminance; even populations average their two middle values. This reduces the influence of bright and dark tails without clipping working pixels. The 0.18 target is a starting-brightness choice, not a claim that a scene's median surface is a gray card.

Every Photo EV starts at zero. The common exposure preserves frame-to-frame brightness differences, using scene-metered film exposure as a practical prior. Individually normalizing frames can darken foregrounds beneath bright skies or lift dark scenes and their shadow noise. Manual Photo EV remains available for exceptions.

The shared baseline still depends on the roll's scenes and consistent scanning exposure. Predominantly dark or bright rolls can need manual Roll EV adjustment; the statistic cannot infer photographic intent.

## Review uses finished detection

The document-placed [Analyze tool](../src/editor/analysis.rs) owns its unfinished task, source-keyed sampling edits, gestures and temporary view. Its task awaits shared detection and review confirmation before fitting. Confirmation snapshots the areas and displayed source; fitting remains provisional until validated installation. Browsing replaces the photo visit while the tool's drafts survive ([architecture](architecture.md#scopes-nested-disposable-ui-state) owns that model). Reusable original sampling inputs belong to the document; active fitting retains its own input snapshot.

While each original is available, shared preparation copies the current canonical grid's 8×8 camera-code patches and native-floor decisions. These document-owned working inputs retain original coordinates and precision, independently of saved regions and the full-image LRU; they hold no native image leases. Reviewed windows, material support and the current flat-field correction select and evaluate those patches after confirmation. Closing the document or removing a source releases its working inputs. Their reported bytes count owned patch payloads, not total process memory. Those sparse patches cannot supply GI's regular raster: fitting still acquires originals after confirmation, even when it reuses patches for anchor and exposure measurements. This adds a source pass when the full-image cache cannot satisfy acquisition.

Review needs only geometry and material classification; it does not develop RAWs or run image-based refinement. Opening a project with finished regions collects missing sampling inputs when Analyze needs them, without redetecting areas; Auto-crop can use those regions without collecting pixels. Legacy requests without reviewed regions, and sources without retained inputs, use direct sampling.

## Install the baseline, not a partial result

Analyze replaces calibration, channel balance, Roll WB/EV and all ordered Photo grading. Photo exposure and WB start at zero; contrast, pivot, colour preservation, saturation and density return to their actual defaults. Each analyzed photo's layer stack is replaced with one ordinary global layer at default settings; its non-reusing ID allocator is retained. The accepted flat-field reference grade is untouched.

Every run reviews sampling areas. Nondefault layer settings or a stack other than the initial global layer add an explicit reset warning to that same review, not a second modal; the warning names layer replacement, and the accepted installation is one undoable history entry. Opening Analyze retires unfinished control input; complete numeric entry first to include it. Switching tools retires the unfinished analysis. Selected global gamma/response, accepted correction and Roll/Photo orientation and crop are preserved; no review draft is saved.

The complete candidate validates—including every source's resulting GPU domain and the reference-only grade—before one atomic installation of both support and grading. Failure or cancellation preserves accepted processing and multi-selection. Replacing Analyze leaves the displayed photo in place for the next tool. Explicit cancellation waits for worker acknowledgement; tool replacement retires the action immediately while native/GPU work cleans up cooperatively.

## Where the estimate can fail

Faint/off-image or scene-confounded boundaries can defeat area suggestions; holder fuzz and narrow rebates remain uncertain at bounded evidence resolution. A damaged frame can still dominate independent Dmax tails, and correlated scene colour can bias shared tonal fitting and WB. These are limitations, not permission for silent source exclusion, repair or hidden clamps: area review and numerical rejection are complementary safeguards, not photographic guarantees.

## Working on this area

- [Footprint selection](../src/scan/analysis.rs), [GPU camera sampling](../src/gpu/analysis/sampling.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Point-only seed](../src/roll_analysis.rs), [spatial GI baseline](../src/roll_analysis/gi.rs), [selector](../src/roll_analysis/grayness.rs) and [curve fitter](../src/roll_analysis/gamma.rs): the numerical proposal.
- [Spatial hypotheses](../src/scan_regions.rs), [shared preparation](../src/editor/spatial/preparation.rs) and [document retention](../src/editor/document_scope.rs): reusable source-owned support and original sampling inputs.
- [Editor Analyze](../src/editor/analysis.rs): tool interaction, review assembly, awaited preparation, cancellation acknowledgement and atomic installation of reviewed areas with grade.
- [Captured sampling/fitting](../src/editor/analysis/fit.rs): footprint eligibility, GPU measurements and validated numerical proposals.
- [Scalar seed tests](../tests/roll_analysis.rs), [GI tests](../src/roll_analysis/gi/tests.rs), [spatial GPU sampling tests](../src/gpu/analysis/sampling/spatial/tests.rs) and [editor tests](../src/editor/analysis/tests.rs): synthetic recovery, abstention, original-coordinate eligibility and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises.