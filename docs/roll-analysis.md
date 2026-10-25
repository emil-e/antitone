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

The analysis uses original developed camera pixels, with accepted scan correction applied — never a displayed preview or an embedded camera JPEG. Two sample populations have different jobs: the film population supplies the low-density reference; the picture population supplies dense tails, colour trends and exposure statistics. Sampling is bounded so a roll does not require retaining every developed RAW at once, and local capture/development checks reject clipped or precision-floored footprints before retaining their original centre pixels.

Those checks establish numerical eligibility, **not that a pixel is film material** — they cannot reliably distinguish a holder, an exposed edge or damaged film from an ordinary subject.

**Analyze** on the Roll tool shelf requests reusable [shared film/picture metadata](spatial-metadata.md), prepares its separate per-photo sampling areas, then opens one review for the roll. Either Analyze or Auto-crop can prepare the shared description without accepting the other's edits. The viewer shows the uncropped source with both opening and picture areas — direct canvas selection, corner resizing and dragging; automatic film exclusions stay visible, and warning icons identify missing areas and the consequence of replacing existing Photo adjustments. There is no mandatory approval of every photo: browse normally, edit individual exceptions, and confirm the complete set with **Analyze roll**; Cancel restores the original working photo and leaves accepted areas and grading unchanged.

The suggestions assume photographs from one actual roll/stock and a consistent scanning setup. Compatible captures strengthen one conservative holder interior — recurring edge-connected darkness supplies pooled exterior evidence, and an inscribed rectangle avoids rounded corners and terminal intrusions rather than merely intersecting four fitted rails. Opposing measured rails can support a translated photo exception; substantial inward departures stay local, and uncorroborated evidence can abstain and leave coverage bounds for review. Picture evidence remains independent inside that opening: own-source transitions, recurring material appearance and peer layout suggest frame edges and holder/gap exclusions, but weak transitions can supply statistical support without certifying a physical frame or crop. These estimate safe statistical support, not an exact physical aperture or every pixel's material identity.

Authored windows replace corresponding automatic area suggestions without moving shared physical boundaries. Opening edits remain per-photo exceptions. Matching accepted support is reused; incomplete support or older automatic local-only areas triggers fresh sampling preparation.

Before numerical estimation, complete 8×8 footprints on the canonical source grid must satisfy film/material/holder support, including its source-scaled inward safety band. Picture statistics additionally obey nominal picture boundaries, even when an authored window extends farther; film-base statistics may include valid film outside the picture. Camera limits add no invented film edge. These sampling safeguards do not trim photographic crops. Previews supply spatial evidence, never the numerical samples. Every ordered photo must supply adequate picture statistics; analysis never silently drops a difficult photo to make the roll succeed.

## 2. Estimate the two anchors

**Dmin** uses the high-transmission film samples pooled across the roll. For each channel, it averages the interpolated 98–99.5% quantile band. A band avoids letting one brightest value determine the reference, while looking toward the least dense sampled film.

**Dmax** uses each channel's 0.5% picture quantile, after equal-count sampling gives photos equal population weight. Each channel is estimated independently: requiring a single RGB pixel to supply all three anchors would couple the result to that object's colour.

These are useful statistical anchors, not guaranteed observations of clean film base and maximum physical density. A damaged dense tail or holder contamination can still bias them.

## 3. Fit a shared colour trend across tones

A constant WB adjustment cannot fix every shadow-to-highlight colour trend. Channel gamma balance can, because its effect varies with density. The analysis therefore fits curves first and WB afterward.

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

Bradford WB multiplies cone responses. Taking logarithms turns those gains into an additive offset shared by all bands, and subtracting two band coordinates removes that offset. Exposure likewise adds a common log gain and does not change the differences. This lets the fit ask “which curves reduce tonal colour drift?” without simultaneously asking “what should the overall colour balance be?”

### Why average before squaring

Imagine one photograph with warm highlights and cool shadows, and another with the opposite scene colours. Penalizing each photo separately would pressure both toward neutral bands. Averaging their differences first lets opposing scene trends cancel; the fit targets a trend shared by the roll instead.

That is still an assumption. A roll dominated by the same materials or coloured lighting can have a genuine common scene trend; the optimizer cannot prove whether it comes from the film response or the scenes. A bounded search compares valid starting balances and refines them within the existing controls; undefined or out-of-range candidates are rejected, not clamped. Search mechanics and stopping criteria live in [the fitter](../src/roll_analysis_gamma.rs).

## 4. Set shared photographic WB

With the curves fixed, equal-count samples produce one positive mean per photo. Their equally weighted mean supplies a shared Bradford correction toward AP1/D60 neutral.

This is a roll-wide colour prior, not a measured neutral target. Photo WB starts at zero and remains manual: automatically neutralizing each photo would too readily remove blue shade, coloured lighting or a scene's dominant material colour.

## 5. Place each photo's exposure

Exposure is estimated after curve fitting and Roll WB. Two goals are blended equally in stops:

- Put a robust mean luminance near 0.18.
- Put the 99th-percentile luminance near one.

The mean counts values above p99 as p99, reducing the leverage of very bright outliers. **Only the statistic is capped; working pixels are not clipped.**

```text
total_EV = (log2(0.18 / capped_mean) + log2(1 / p99)) / 2
```

These are working-luminance priors, not display white or exposure ground truth. A sky-heavy frame can still leave its foreground too dark; manual exposure remains appropriate.

Roll EV receives the median total EV, and each Photo EV receives its total minus that median. For desired exposures of 1, 2 and 4 stops, that means Roll EV 2 and Photo EV −1, 0 and 2: the sums—and therefore the calculated appearance—are unchanged. This is a useful control decomposition, not another normalization.

## Reviewing gathers evidence, not photographs

Analyze runs in a review session: temporary per-photo support and a separate browsing-return context, with nothing entering the project until acceptance ([architecture](architecture.md#scopes-nested-disposable-ui-state) owns the visit model). A fit can preserve exact fine RGB16 inference rasters in a bounded temporary file while the roll model is fitted, rather than developing those RAWs again; that scratch belongs to one request, closes on success/cancellation/failure, and never enters project persistence. Capacity or scratch-I/O failures fall back to the original reacquisition path with unchanged measurements.

## Install the baseline, not a partial result

Analyze replaces calibration, channel balance, Roll WB/EV and all ordered Photo grading. Photo exposure gets the estimated residual; Photo WB, contrast, pivot, colour preservation, saturation and density return to their actual defaults. Local adjustments and their masks are removed from analyzed photos, including disabled and identity records; their non-reusing ID allocators are retained. The accepted flat-field reference grade is untouched.

Every run reviews sampling areas. Non-default ordered Global Photo settings or any nonempty local collection add an explicit reset warning to that same review, not a second modal; the warning names local-mask removal, and the accepted installation is one undoable history entry. Pending numeric edits commit before preparation, and stale review cannot authorize a changed document or grade. Selected global gamma/response, accepted correction and Roll/Photo orientation and crop are preserved; no review draft is saved.

The complete candidate validates—including every source's resulting GPU domain and the reference-only grade—before one atomic installation of both support and grading. Failure, cancellation or stale completion preserves accepted processing and returns to the original working photo without changing multi-selection. Cancellation is cooperative; a native call or submitted GPU phase may delay acknowledgement.

## Where the estimate can fail

Faint/off-image or scene-confounded boundaries can defeat area suggestions; holder fuzz and narrow rebates remain uncertain at bounded evidence resolution. A damaged frame can still dominate independent Dmax tails, and correlated scene colour can bias shared tonal fitting and WB. These are limitations, not permission for silent source exclusion, repair or hidden clamps: area review and numerical rejection are complementary safeguards, not photographic guarantees.

## Working on this area

- [Sampler](../src/scan.rs) and [Dmin estimator](../src/sampling.rs): eligible original pixels and endpoint statistics.
- [Estimator](../src/roll_analysis.rs) and [curve fitter](../src/roll_analysis_gamma.rs): the numerical proposal.
- [Spatial hypotheses](../src/scan_regions.rs), [staged preparation](../src/editor_analysis_preparation.rs), [source services](../src/editor_analysis_services.rs) and [review controller](../src/editor_analysis_review.rs): source-owned support, bounded scratch/fallback, drafts and review lifecycle.
- [Captured sampling/fitting](../src/editor_analysis_fit.rs) and [workflow](../src/editor_analysis_workflow.rs): awaited source acquisition, bounded statistics, cancellation and direct typed completion.
- [Editor analysis](../src/editor_analysis.rs): adequacy and atomic installation of reviewed areas with grade.
- [Scalar tests](../tests/roll_analysis.rs), [sampling tests](../tests/roll_sampling.rs) and [editor tests](../src/editor_analysis_tests.rs): synthetic recovery, invariance and transaction behavior.

Use [development checks](development.md#roll-analysis) for changes and [diagnostics](diagnostics.md#analyze-roll) for real-RAW exercises.