# Grading a photograph

The **Photo** tab holds everything that is about one photograph: composition, cleanup and
your photographic grade. Nothing here affects other photos — the shared roll setup lives
on [the Roll panel](roll.md).

The controls are deliberately ordinary photographic ones, applied to the developed
positive in Antitone's colour-managed working space;
[why the grading feels familiar](README.md#why-the-grading-feels-familiar) explains the
model behind that promise.

## The photographic grade

- **Exposure** — in EV. The slider covers ±3 EV; type a value for up to ±16 EV. (After
  Analyze this is the photo's residual exposure on top of the shared Roll EV.)
- **White balance** — **Warmth** and **Tint**, or **Pick neutral…** on a grey/neutral
  area. This is per-photo balance on top of the roll's shared balance.
- **Contrast** — with **Pivot** (where contrast is centred) and **Colour preservation**
  (how much colour is protected from contrast-driven shifts).
- **Saturation** — with **Density**, which darkens as it saturates, like adding dye
  density rather than a pure saturation slider.

Sliders take **Shift for finer steps**. Everything on this tab is undoable (⌘Z).

## Crop & straighten

Activate **Crop & straighten** from the tool shelf:

- **Auto-crop** proposes a crop straight to the film frame — it uses the frame
  measurements retained by Analyze, so run Analyze first for best results.
- **Aspect** — Free, Current, Original, or fixed ratios (1:1, 3:2, 4:3, 5:4, 16:9);
  **Swap portrait / landscape** flips the ratio.
- **Angle** / **Reset straighten** — straighten by eye; Thirds guides help composition.

Rotation and crop are per-photo; scan orientation (for the whole roll) is on the Roll
tool shelf.

## Local adjustments (gradients)

For graduated changes — darkening a sky, lifting a face — use **Add radial** or **Add
linear** from the **Adjustments** drawer. Each gradient is a named adjustment with its own
exposure, white balance, contrast and saturation.

- Draw the gradient on the canvas; drag to move, adjust **Inward margin**, **Strength**
  and **Feather** to taste. **Invert mask** flips which side it affects.
- Weights are continuous: a gradient that half-covers a region with +1 EV applies about
  +0.5 EV there — a true blend, not an on/off mask.
- Gradients are listed by name ("Adjustment name"); rename them as you add more. Deleting
  asks for confirmation.

Global grading runs first, local gradients on top, in the order listed.

## Dust repair

**Dust repair** (Photo tool shelf) is a manual healing brush for dust and small defects —
there is no automatic dust detection. It is provisional, but reliable in what it does:
repairs are saved in the project and applied consistently to the developed view,
thumbnails and exports.

- **Brush size** is in source pixels — zooming in does not change the area you repair.
- Paint over the defect. **Show repair outlines** keeps marks visible; hold **H** for a
  momentary unobstructed preview.
- **Reset dust repairs** removes this photo's repairs and starts over.

Repairs heal the developed image before grading, so your grade applies to the cleaned
photograph.

## View source while grading

The viewer's **View source** toggle flips between your graded positive and the developed
source — useful to check what the scan actually recorded while you grade. (Measuring
Dmin/Dmax first is required before the positive can be shown.) See
[The viewer](viewer.md).
