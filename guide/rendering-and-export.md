# Rendering and export

**Rendering** decides what the finished photograph looks like; **File → Export…** writes
it to disk. The preview is rendered through the same pipeline as the export — what you
see is what the file contains.

## The Rendering panel

The sections follow processing order:

- **Film texture** — coupled image softness and grain, with separate halation. Presets
  populate starting values; edits retain those actual settings rather than a preset identity.
- **LUT** — **Load LUT…** imports a creative ACEScct/AP1 3D `.cube`, applied before
  Film & paper. Display and camera-conversion LUTs are not supported.
- **Film & paper** — **Negative** selects an optional simulated camera-negative stock;
  None skips synthetic formation, not the Roll panel's inversion of an actual negative.
  **Paper** selects the print medium independently. Paper None is available only without
  a selected negative. From bypass, a colour negative also selects Portra Endura; a B&W
  negative selects Ilford Multigrade RC Deluxe · filter 2. Choose Negative None to use
  paper alone on an already inverted scan or a Digital photograph.
- **Target** — **sRGB** or **Display P3**, both 100-nit. This is the output colour space,
  not the monitor profile; choosing P3 changes the actual rendering and gamut limitation.

LUT and Film & paper are independent: use either, both or neither. A LUT can alter the
working positive before the film/paper response. Switching between colour and B&W asks
before clearing the negative or choosing a matching default paper. Changing a stock
prepares a new treatment in the background; Cancel keeps the accepted choices.

## TIFF or EXR?

The two formats serve different purposes:

| | TIFF | EXR |
| --- | --- | --- |
| What it is | A finished, rendered photograph | The graded image in scene-linear form |
| Colour | sRGB or Display P3, with an ICC profile | Scene-linear AP0, always |
| Negative/Paper/LUT colour treatment and display rendering | Applied | Not applied |
| Use it for | Viewing, printing, delivery, further work in a colour-managed app | Further grading or compositing in HDR/scene-referred tools |
| Precision | 16-bit RGB, lossless Deflate | Half (16-bit) or full float (32-bit) |

Export TIFF when you want the photograph you see in Antitone. Export EXR when another
program will continue the grade — it retains normal source inversion, grading and grain,
but skips the Negative/Paper/LUT colour treatment and display rendering.

## Exporting

**File → Export…** opens the export dialog:

1. **Photos** — **Selected photos** (use the filmstrip selection) or **All photos**.
2. **Destination** — **Choose…** a folder.
3. **Output** — **Format** (TIFF/EXR). TIFF: **Target** (initialized from Rendering, but
   independently choosable per export), 16-bit RGB, lossless Deflate. EXR: **Precision**
   (half or full float).
4. **Files** — filename is the **Original filename**, with an optional **Suffix**; the
   dialog shows an example. For existing files choose **Keep both**, **Skip** or
   **Replace…** (replacement asks for confirmation once).

Click **Export N photo(s)**. Each photograph is rendered and written before the next
begins; a photo that fails does not stop the batch. The summary shows exported, skipped,
failed and remaining counts, with **Show in Finder** to inspect the results.

![The export dialog: photos, destination, format and file options](../assets/docs/export-dialog.png)

Finish any active tool draft (crop, dust, an unapplied adjustment) before exporting —
exports use your applied settings.

## Good to know

- Exports read your **original scans** at full resolution — never the preview — so keep
  the RAW files available.
- Everything is colour-managed end to end: the preview, the TIFF (which carries its ICC
  profile) and the scopes all follow the chosen Target.
- EXR output keeps headroom (values above white and below black survive), so a later
  grade in another tool has room to work. The file declares its own colour space, so
  the receiving application does not have to guess one.
