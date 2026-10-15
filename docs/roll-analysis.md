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

**Analyze roll** first prepares per-photo sampling areas, then opens one review for the roll. The normal inspector stays in place, with review controls and any Photo-reset warning revealed in its **Roll analysis** section. Other photographic controls are read-only during review. The uncropped source shows the opening by default; switch to **Picture statistics** to inspect the photographic domain and automatic film exclusions. Browse normally in the filmstrip or with arrow keys, and edit individual exceptions. There is no mandatory approval of every photo. The holder model is shared where the captures support it; picture geometry and authored exceptions remain per-photo. **Analyze roll** in the viewer confirms the complete set; Cancel restores the original working photo and leaves accepted areas and grading unchanged.

These suggestions assume photographs from one actual roll/stock and a consistent scanning setup. Compatible captures strengthen one conservative holder interior: recurring edge-connected darkness supplies pooled exterior evidence, and an inscribed rectangle avoids rounded corners and terminal intrusions rather than merely intersecting four fitted rails. Fine measurements tighten the common interior before it is frozen. Opposing measured rails can support a translated photo exception; substantial inward departures stay local. Uncorroborated or incompatible evidence can still abstain and leave coverage bounds for review. This estimates safe statistical support, not an exact physical aperture or every original pixel's material identity.

Picture evidence remains independent inside that opening: own-source transitions, recurring material appearance and peer layout suggest frame edges and holder/gap exclusions. Weak transitions and source/opening coverage bounds can supply statistical support without certifying a physical frame or crop. **Inspect** and **Need area** filmstrip badges direct attention, not guarantee that other suggestions are correct. Faint and dense interiors remain eligible by location, not by a provisional density threshold.

Accepted shared support and authored exceptions are reused; incomplete support or older automatic local-only areas triggers fresh roll preparation. Authored edits replace the corresponding automatic constraints: editing opening replaces the automatic film mask and refits automatic picture coverage against retained independent frame evidence. It does not move measured picture edges or overwrite an authored picture area. Opening edits are per-photo exceptions, not a silent roll-wide overwrite. Editing picture replaces its half-planes and retained automatic frame evidence. Older shared explicit picture geometry remains authoritative and is transferred into the review; accepted correction is retained independently, with its support available as an opening fallback. Missing picture support requires an explicit edit, never a silent whole-camera fallback. **Reset sampling areas** discards accepted support without changing the grade, so the next Analyze prepares new suggestions.

Support is frozen before estimation. A canonical grid spans each active source; complete 8×8 footprints must belong to film support, and picture footprints must additionally satisfy picture geometry. Only then do the unchanged original capture/development, corrected-floor and finite-value checks run. Previews supply spatial evidence, never the numerical Analyze samples. Confirmation approves using areas—not the identity or numerical validity of every pixel. Numerically valid wrong-material contamination can still bias a result.

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

Every run reviews sampling areas. Non-default ordered Photo settings add the reset warning to that same review, not a second modal. Pending numeric edits commit before preparation; stale review cannot authorize a changed document or grade. Authored zero is not treated as “untouched.”

Selected global gamma/response, accepted correction and Roll/Photo orientation and crop are preserved. Reviewed per-photo support is saved with the installed baseline; no review draft is saved. Manual edits, picks and area changes never trigger another analysis automatically.

The complete candidate validates, including every Photo's GPU domain, before one atomic installation of both support and grading. Failure, cancellation or stale completion preserves accepted processing and returns to the original working photo without changing multi-selection. Source file revisions are checked through promotion. Cancellation is cooperative; a native call or submitted GPU phase may delay acknowledgement.

## Where the estimate can fail

Faint/off-image or scene-confounded boundaries can defeat area suggestions; holder fuzz and narrow rebates remain uncertain at bounded evidence resolution. Preparation uses stable source-locator order, not the currently browsed photo, and does not use a preferred grade to select geometry. A damaged frame can still dominate independent Dmax tails, and correlated scene colour can bias shared tonal fitting and WB. These are limitations, not permission for silent source exclusion, repair or hidden clamps. Area review and numerical rejection are complementary safeguards, not photographic guarantees.

The [acceptance plan](plans/roll-assistance.md) tracks remaining photographic and physical-use checks.

## Working on this area

- [Sampler](../src/scan.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Estimator](../src/roll_analysis.rs) and [curve fitter](../src/roll_analysis_gamma.rs): the numerical proposal.
- [Spatial hypotheses](../src/scan_regions.rs) and [review controller](../src/editor_analysis_review.rs): source-owned support, drafts and review lifecycle.
- [Editor analysis](../src/editor_analysis.rs): adequacy, cancellation and atomic installation.
- [Scalar tests](../tests/roll_analysis.rs), [sampling tests](../tests/roll_sampling.rs) and [editor tests](../src/editor_analysis_tests.rs): synthetic recovery, invariance and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises. Passing those checks does not establish calibrated colour or universal photographic success.
