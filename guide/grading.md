# Grading a photograph

The **Photo** tab holds everything that is about one photograph: composition, cleanup and
your photographic grade. Nothing here affects other photos — the shared roll setup lives
on [the Roll panel](roll.md).

The controls are deliberately ordinary photographic ones, applied to the developed
positive in Antitone's colour-managed working space;
[why the grading feels familiar](README.md#why-the-grading-feels-familiar) explains the
model behind that promise.

## The photographic grade

These controls edit the selected adjustment layer. The **Layers** header names the
target even when collapsed; every new photo starts with an ordinary **Global** layer.

- **Exposure** — in EV. The slider covers ±3 EV; type a value for up to ±16 EV. (After
  Analyze this is the photo's residual exposure on top of the shared Roll EV.)
- **White balance** — **Warmth** and **Tint**, the **White balance wheel** beside them, or
  the eyedropper in the section header on a grey/neutral area. This is per-photo balance
  on top of the roll's shared balance.
- **Contrast** — with **Pivot** (where contrast is centred) and **Preserve** (how much
  colour is protected from contrast-driven shifts).
- **Saturation** — with **Density**, which darkens as it saturates, like adding dye
  density rather than a pure saturation slider.

Hold **Shift** while dragging a slider, its value or the balance wheel for finer motion, and
with the arrow keys for finer steps; a track's precision drag starts only when Shift is held
as you press. Double-click or Option-click a control's name to reset it; the section reset
clears the whole group. Everything on this tab is undoable (⌘Z).

## Crop & straighten

Activate **Crop & straighten** from the tool shelf:

- **Auto-crop** proposes a crop straight to the film frame. It measures the frame
  itself, so it works without Analyze; accepted Analyze areas are reused when they exist.
- **Aspect** — Free, Current, Original, or fixed ratios (1:1, 3:2, 4:3, 5:4, 16:9);
  **Swap portrait / landscape** flips the ratio.
- **Angle** / **Reset straighten** — straighten by eye; Thirds guides help composition.

Rotation and crop are per-photo; scan orientation (for the whole roll) is on the Roll
tool shelf.

## Adjustment layers

Use **Add global**, **Add linear** or **Add radial** in the **Layers** section. Every
layer has the same exposure, white balance, contrast and saturation controls; only its
coverage differs.

- Layers process **bottom to top**. Drag rows to reorder; new layers appear above the
  selection. Put a global contrast layer above a sky gradient to soften the balanced
  photograph, rather than changing what the gradient receives.
- Draw gradients on the canvas and refine their handles. **Strength** attenuates the
  adjustment, **Invert mask** swaps coverage, and radial **Feather** changes the fade.
  Half coverage of +1 EV means +0.5 EV, not half a rendered-pixel blend.
- Select a row to target the shared controls, double-click to rename, or press
  Delete/Backspace to remove it with confirmation. The initial global layer can also be
  moved, disabled or deleted.

Collapse **Layers** for a clean view. Mask coverage and handles also hide while you
adjust values, then return. Drawing or moving a gradient keeps its guides visible.

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
