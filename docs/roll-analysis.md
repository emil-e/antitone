# Analyze roll: estimating a starting grade

Analyze turns a roll of camera scans into a shared colour baseline and individually exposed photographs. It aims to make manual grading start from somewhere useful—not to finish every image or measure the film's physical response.

The distinction from [processing](processing.md) matters: processing transforms pixels using supplied settings; analysis **estimates those settings** from sampled images.

```text
eligible camera samples
  → Dmin and independent-channel Dmax
  → shared channel curves, fitted independently of photographic WB
  → shared Roll WB
  → individual total exposures
  → Roll EV median + Photo EV residuals
```

## 1. Choose what to sample

The analysis uses original developed camera pixels, with accepted scan correction applied. It does not measure a displayed preview or an embedded camera JPEG.

Two sample populations have different jobs. The film population supplies the low-density reference; the picture population supplies dense tails, colour trends and exposure statistics. Sampling is bounded so a roll does not require retaining every developed RAW at once. Local capture/development checks reject clipped or precision-floored footprints before retaining their original centre pixels.

Those checks establish numerical eligibility, **not that a pixel is film material**. They cannot reliably distinguish a holder, an exposed edge or damaged film from an ordinary subject.

The editable **picture area** is shared statistics geometry, not output crop or straightening. Explicit geometry wins. Otherwise, accepted correction support or a compatible roll source's suggested illumination opening supplies a starting area. If no suggestion exists, Analyze asks for an explicit area rather than silently measuring the whole camera image. Film sampling uses the shared support, or the active area when explicit picture geometry is available without support. Illumination proposals are requested lazily on the worker, not during ordinary photo selection. Analyze resolves the unchanged loaded source's local hint first when it has not been requested, then falls back to ordered-roll search. Reset picture area deliberately bypasses that local hint and retains ordered-roll search. Picture-area editing requests a suggestion only when accepted geometry/correction support is unavailable and local support has not yet been queried; the delayed result is transient until Apply and cannot overwrite an already drawn rectangle.

Every ordered photo must supply adequate picture statistics. An individual photo may supply no usable film samples if the pooled film population is adequate. Analysis never silently drops a difficult photo to make the roll succeed.

## 2. Estimate the two anchors

**Dmin** uses the high-transmission film samples pooled across the roll. For each channel, it averages the interpolated 98–99.5% quantile band. A band avoids letting one brightest value determine the reference, while looking toward the least dense sampled film.

**Dmax** uses each channel's 0.5% picture quantile, after equal-count sampling gives photos equal population weight. Each channel is estimated independently. Requiring a single RGB pixel to supply all three anchors would couple the result to that object's colour.

These are useful statistical anchors, not guaranteed observations of clean film base and maximum physical density. A damaged dense tail or holder contamination can still bias them.

## 3. Fit a shared colour trend across tones

A constant WB adjustment cannot fix every shadow-to-highlight colour trend. Channel gamma balance can change that trend because its effect varies with density. The analysis therefore fits curves first and WB afterward.

Each photo is divided into shadow, middle and highlight bands using its positive luminance ranks with zero gamma balance. These memberships stay fixed during fitting, so a candidate cannot improve its score simply by moving samples into different bands.

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

Bradford WB multiplies cone responses. Taking logarithms turns those gains into an additive offset shared by all bands. Subtracting two band coordinates removes that offset. Exposure likewise adds a common log gain and does not change the differences.

This lets the fit ask “which curves reduce tonal colour drift?” without simultaneously asking “what should the overall colour balance be?”

### Why average before squaring

Imagine one photograph with warm highlights and cool shadows, and another with the opposite scene colours. Penalizing each photo separately would pressure both toward neutral bands. Averaging their differences first lets opposing scene trends cancel; the fit targets a trend shared by the roll instead.

That is still an assumption. A roll dominated by the same materials or coloured lighting can have a genuine common scene trend. The optimizer cannot prove whether it comes from the film response or the scenes.

A bounded search compares valid starting balances and refines them within the existing controls. Undefined or out-of-range candidates are rejected, not clamped. Search mechanics and stopping criteria live in [the fitter](../src/roll_analysis_gamma.rs).

## 4. Set shared photographic WB

With the curves fixed, equal-count samples produce one positive mean per photo. Their equally weighted mean supplies a shared Bradford correction toward AP1/D60 neutral.

This is a roll-wide colour prior, not a measured neutral target. Photo WB starts at zero and remains manual. Automatically neutralizing each photo would too readily remove blue shade, coloured lighting or a scene's dominant material colour.

## 5. Place each photo's exposure

Exposure is estimated after curve fitting and Roll WB. Two goals are blended equally in stops:

- Put a robust mean luminance near 0.18.
- Put the 99th-percentile luminance near one.

The mean counts values above p99 as p99, reducing the leverage of very bright outliers. **Only the statistic is capped; working pixels are not clipped.**

```text
total_EV = (log2(0.18 / capped_mean) + log2(1 / p99)) / 2
```

These are working-luminance priors, not display white or exposure ground truth. A sky-heavy frame can still leave its foreground too dark; manual exposure remains appropriate.

Roll EV receives the median total EV. Each Photo EV receives its total minus that median. For desired exposures of 1, 2 and 4 stops, that means Roll EV 2 and Photo EV −1, 0 and 2. The sums—and therefore the calculated appearance—are unchanged. This is a useful control decomposition, not another normalization.

## Install the baseline, not a partial result

Analyze replaces calibration, channel balance, Roll WB/EV and all ordered Photo grading. Photo exposure gets the estimated residual; Photo WB, contrast, pivot, colour preservation, saturation and density return to their actual defaults. Retired/nonordered source grades are untouched.

Non-default ordered Photo settings trigger a reset confirmation. Default Photos analyze directly. Pending numeric edits commit before the snapshot; stale confirmation cannot authorize a changed document or grade. Authored zero is not treated as “untouched.”

Selected global gamma/response, accepted correction, statistics geometry and Roll/Photo orientation and crop are preserved. The used picture area is saved with the installed baseline. Manual edits, picks and area changes never trigger another analysis automatically.

The complete candidate validates, including every Photo's GPU domain, before one atomic installation. Failure, cancellation or stale completion preserves the accepted grade and settled pixels. Cancellation is cooperative; a native call or submitted GPU phase may delay acknowledgement.

## Where the estimate can fail

Opening suggestions can depend on source order, and a damaged frame can dominate independent Dmax tails. Shared tonal fitting and WB can be biased by correlated scene colour. These are limitations of the estimator, not permission for silent source exclusion, repair or hidden clamps.

The [acceptance plan](plans/roll-assistance.md) tracks remaining photographic and physical-use checks.

## Working on this area

- [Sampler](../src/scan.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Estimator](../src/roll_analysis.rs) and [curve fitter](../src/roll_analysis_gamma.rs): the numerical proposal.
- [Editor analysis](../src/editor_analysis.rs): geometry, adequacy, cancellation, confirmation and atomic installation.
- [Scalar tests](../tests/roll_analysis.rs), [sampling tests](../tests/roll_sampling.rs) and [editor tests](../src/editor_analysis_tests.rs): synthetic recovery, invariance and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises. Passing those checks does not establish calibrated colour or universal photographic success.
