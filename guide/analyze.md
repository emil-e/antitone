# Analyze

**Analyze** finds a starting grade for the whole roll from the scans themselves: shared
calibration and colour for the roll, plus an exposure for each photograph. It aims to
give you a useful place to start — not to finish every image.

It is on the **Roll** tool shelf.

## What it does

Working from the camera's raw measurements (never the preview you see), Analyze:

1. Measures **Dmin** (film base) and **Dmax** from clear and dense areas.
2. Fits a shared **channel response** so the roll has a consistent tonal colour trend.
3. Sets the roll's shared **white balance**.
4. Estimates an **exposure per photograph**, split into a shared Roll EV and a per-photo
   residual.

That is why your photographs line up with each other immediately afterwards — and why the
remaining work is refinement, not rescue.

## The sampling-area review

After preparing, Analyze opens a review before changing anything. The viewer shows each
photograph's uncropped source with two outlined areas:

- **Opening** — the clear film opening around the frame. Supplies the film-base evidence.
- **Picture** — the image itself. Supplies the dense tones, colour and exposure evidence.

Adjust any area that is wrong: drag its corners to resize, its interior to move; click a
label when the two outlines coincide. Missing-area warnings identify photographs that
need attention. Browse the roll normally — there is no mandatory per-photo approval.
Press **Analyze roll** (Enter) to confirm the complete set, or **Cancel** (Escape), which
restores everything as it was.

> **What confirming replaces.** Analyze installs a fresh baseline: it replaces the roll
> calibration and Photo grading, and **removes local adjustments and their masks** from
> analyzed photographs (a warning in the review counts the affected photos). Orientation
> and crop are preserved. Analyze is not undoable in the ordinary sense — treat it as the
> start of grading, and re-run it only when you are willing to lose manual Photo work.

When it finishes: "Analyzed N frames. Adjust normally; re-pick Dmin if needed."

## After Analyze

- Refine each photograph on the **Photo** tab: exposure, white balance, contrast,
  saturation. Everything per-photo starts from here.
- **Auto-crop** (Photo tab) can use the frame measurements Analyze retained — it
  straightens and crops to the frame it found.
- Re-pick Dmin on the Roll panel if the film base was dirty or unusual.

## When Analyze can mislead

Knowing the failure modes helps more than any setting:

- The suggestions assume **one roll, one stock, one scanning setup**. Mixed setups confuse
  the evidence.
- The evidence is the whole set you loaded. A **partial roll**, or the exposed leader and
  blank frames at the start of a strip, biases Dmin and the shared white balance, and every
  photograph comes out cast. Analyze the roll's exposed frames together.
- Areas are statistical suggestions, not physical measurements of your holder. Holder
  fuzz, narrow rebates or a damaged frame can bias them — that is what the review is for.
- A scene dominated by one colour (a red stage, blue shade) can look like a colour cast
  to the shared fit. Analyze averages trends across the roll; it cannot know what was in
  front of the camera. Correct individually on Photo if the "correction" hurt the mood.
- Exposure aims at a reasonable middle (mean near 0.18, highlights near white). A
  sky-heavy frame can still need manual exposure.
