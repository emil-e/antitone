# Analyze roll: estimating a starting grade

Analyze turns a roll of camera scans into a shared colour and exposure baseline. It aims to make manual grading start from somewhere useful—not to finish every image or measure the film's physical response.

The distinction from [processing](processing.md) matters: processing transforms pixels using supplied settings; analysis **estimates those settings** from sampled images.

```text
eligible camera samples
  → Dmin and independent-channel Dmax
  → shared channel curves, fitted independently of photographic WB
  → shared Roll WB
  → per-photo exposure estimates
  → shared Roll EV median; Photo EV zero
```

## 1. Choose what to sample

The analysis uses original developed camera pixels, with accepted scan correction applied — never a displayed preview or an embedded camera JPEG. Two sample populations have different jobs: the film population supplies the low-density reference; the picture population supplies dense tails, colour trends and exposure statistics. Sampling batches only selected 8×8 camera cells, retaining their original coordinates for correction. Local numerical checks reject precision-floored footprints before retaining their original centre pixels; fitting retains samples rather than every developed RAW.

Spatial support excludes the holder and unsuitable material; numerical checks establish usable developed values. Statistical anchors and capped exposure means limit the influence of isolated extremes.

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

Each photo is divided into shadow, middle and highlight bands using its positive luminance ranks with zero gamma balance. These memberships stay fixed during fitting, so a candidate cannot improve its score simply by moving samples into different bands.

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

With the curves fixed, equal-count samples produce one positive mean per photo. Their equally weighted mean supplies a shared Bradford correction toward AP1/D60 neutral.

This is a roll-wide colour prior, not a measured neutral target. Photo WB starts at zero and remains manual: automatically neutralizing each photo would too readily remove blue shade, coloured lighting or a scene's dominant material colour.

## 5. Set shared roll exposure

Each photo supplies an exposure estimate after curve fitting and Roll WB. Two goals are blended equally in stops:

- Put a robust mean luminance near 0.18.
- Put the 99th-percentile luminance near one.

The mean counts values above p99 as p99, reducing the leverage of very bright outliers. **Only the statistic is capped; working pixels are not clipped.**

```text
estimated_EV = (log2(0.18 / capped_mean) + log2(1 / p99)) / 2
```

Roll EV receives the median of those estimates; every Photo EV starts at zero. The common exposure preserves frame-to-frame brightness differences, using scene-metered film exposure as a practical prior. Individually normalizing frames can darken foregrounds beneath bright skies or lift dark scenes and their shadow noise. Manual Photo EV remains available for exceptions.

The targets are working-luminance priors, not display white or exposure ground truth. The shared baseline still depends on the roll's scenes and consistent scanning exposure.

## Review uses finished detection

The document-placed [Analyze tool](../src/editor/analysis.rs) owns its unfinished task, source-keyed sampling edits, gestures and temporary view. Its task awaits shared detection and review confirmation before fitting. Confirmation snapshots the areas and displayed source; fitting remains provisional until validated installation. Browsing replaces the photo visit while the tool's drafts survive ([architecture](architecture.md#scopes-nested-disposable-ui-state) owns that model). Reusable original sampling inputs belong to the document; active fitting retains its own input snapshot.

While each original is available, shared preparation copies the current canonical grid's 8×8 camera-code patches and native-floor decisions. These document-owned working inputs retain original coordinates and precision, independently of saved regions and the full-image LRU; they hold no native image leases. Reviewed windows, material support and the current flat-field correction select and evaluate those patches after confirmation. Closing the document or removing a source releases its working inputs. Their reported bytes count owned patch payloads, not total process memory.

Review needs only geometry and material classification; it does not develop RAWs or run image-based refinement. Opening a project with finished regions collects missing sampling inputs when Analyze needs them, without redetecting areas; Auto-crop can use those regions without collecting pixels. Legacy requests without reviewed regions, and sources without retained inputs, use direct sampling.

## Install the baseline, not a partial result

Analyze replaces calibration, channel balance, Roll WB/EV and all ordered Photo grading. Photo exposure and WB start at zero; contrast, pivot, colour preservation, saturation and density return to their actual defaults. Each analyzed photo's layer stack is replaced with one ordinary global layer at default settings; its non-reusing ID allocator is retained. The accepted flat-field reference grade is untouched.

Every run reviews sampling areas. Nondefault layer settings or a stack other than the initial global layer add an explicit reset warning to that same review, not a second modal; the warning names layer replacement, and the accepted installation is one undoable history entry. Pending numeric edits commit before preparation; switching tools retires the unfinished analysis. Selected global gamma/response, accepted correction and Roll/Photo orientation and crop are preserved; no review draft is saved.

The complete candidate validates—including every source's resulting GPU domain and the reference-only grade—before one atomic installation of both support and grading. Failure or cancellation preserves accepted processing and multi-selection. Replacing Analyze leaves the displayed photo in place for the next tool. Explicit cancellation waits for worker acknowledgement; tool replacement retires the action immediately while native/GPU work cleans up cooperatively.

## Where the estimate can fail

Faint/off-image or scene-confounded boundaries can defeat area suggestions; holder fuzz and narrow rebates remain uncertain at bounded evidence resolution. A damaged frame can still dominate independent Dmax tails, and correlated scene colour can bias shared tonal fitting and WB. These are limitations, not permission for silent source exclusion, repair or hidden clamps: area review and numerical rejection are complementary safeguards, not photographic guarantees.

## Working on this area

- [Footprint selection](../src/scan/analysis.rs), [GPU camera sampling](../src/gpu/analysis/sampling.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Estimator](../src/roll_analysis.rs) and [curve fitter](../src/roll_analysis/gamma.rs): the numerical proposal.
- [Spatial hypotheses](../src/scan_regions.rs), [shared preparation](../src/editor/spatial/preparation.rs) and [document retention](../src/editor/document_scope.rs): reusable source-owned support and original sampling inputs.
- [Editor Analyze](../src/editor/analysis.rs): tool interaction, review assembly, awaited preparation, cancellation acknowledgement and atomic installation of reviewed areas with grade.
- [Captured sampling/fitting](../src/editor/analysis/fit.rs): footprint eligibility, GPU measurements and validated numerical proposals.
- [Scalar tests](../tests/roll_analysis.rs), [sampling tests](../tests/roll_sampling.rs) and [editor tests](../src/editor/analysis/tests.rs): synthetic recovery, invariance and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises.