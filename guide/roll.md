# The Roll panel

Everything on the **Roll** tab is shared by the whole roll. Changes here affect every
photograph in the project — which is what makes a roll's photographs match each other.

The panel is best worked through top to bottom.

## Scan orientation

The tool shelf's turn and flip buttons rotate and mirror every scan — use them until
photographs are upright. **Reset scan orientation** returns to the file's own orientation.

Orientation is part of the scan setup, so it lives here rather than on Photo. (Each photo
can additionally be rotated for composition; see [Grading a photograph](grading.md).)

## Flat-field correction

Camera scanning rarely gives perfectly even illumination: the light source or lens may
darken the corners. **Flat-field correction** divides out that unevenness before anything
else happens.

To set it up, you need one extra frame from the same scanning setup: photograph the light
source itself — same lens, focus and aperture, **without film** in the holder. Choose
**Set up…** and select that "bare light" RAW. When editing the reference area, "select the
illuminated area, excluding the holder and including the image centre".

If your setup is even enough without it, leave this disabled. Sony scans may already carry
embedded lens correction (vignetting and distortion), which can make a flat field
unnecessary.

## Calibration: Dmin and Dmax

Calibration tells Antitone what "clear film" and "dense film" look like in *your* scans.
The names come from film density, but what you pick are measurements in the image:

- **Dmin (film base)** — a low-density part of the negative, usually the clear film base
  between or around frames. It becomes the bright end of the positive.
- **Dmax** — a genuinely dense part of the negative (deep shadow on the positive side of
  the film). It becomes the dark end.

Use **Pick Dmin (film base)** and **Pick Dmax**, then click a suitable area in the image.
The values are measured per colour channel, and the two picks need not be the same kind
of material.

If you used Analyze, these are measured for you; the status line shows "Dmin + Dmax
measured". Re-pick Dmin by hand if the film base is dirty or unusual — Analyze's status
message says as much.

## Gamma & balance

**Gamma** sets the overall contrast of the negative-to-positive development. New grades
start at 0.60, a sensible prior for most stocks; adjust to taste.

**Balance** (the gamma balance wheel) is *not* white balance. It changes the relative
response of the three colour channels across tones, so it can fix a colour cast that
differs between shadows and highlights — a negative that goes warm in the highlights and
cool in the shadows, for example. The eyedropper in the section header fits it from a
neutral area.

## Toe & shoulder

**Toe** and **Shoulder** shape the ends of the response: how quickly the deepest shadows
and brightest highlights roll off; 0 turns the shoulder off. If the project shows "Classic
curve", the roll is using a fixed stock-style response; **Use adjustable response** swaps
to the editable curve.

## White balance

This is ordinary photographic white balance, applied to the developed positive and shared
by the roll: **Warmth** and **Tint**, the **White balance wheel** beside them, or the
eyedropper in the section header on a grey/neutral area of the scene.

(Each photograph can then fine-tune balance individually on the Photo tab — Roll WB is
the shared baseline.)

## Exposure

Roll **Exposure**, in EV, shifts the whole roll. Analyze splits each photograph's
estimated exposure into a shared Roll part and a per-photo residual on the Photo tab, so
this is usually where you set the roll's overall brightness after analysis.
