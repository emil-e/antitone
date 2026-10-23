# Using Antitone

Antitone is an editor for camera-scanned film negatives. You photograph your negatives
with a digital camera; Antitone develops those scans into positive photographs, helps you
establish a shared starting grade for the whole roll, and exports TIFF or EXR files.

This guide teaches the workflow. It is organised by what you want to do:

| I want to… | Read… |
| --- | --- |
| Get from a card of scans to exported photographs | [Getting started](getting-started.md) |
| Set up the scan: orientation, illumination, calibration | [The Roll panel](roll.md) |
| Let Antitone find a starting grade for the roll | [Analyze](analyze.md) |
| Grade, crop and clean up one photograph | [Grading a photograph](grading.md) |
| Choose a look and export TIFF or EXR | [Rendering and export](rendering-and-export.md) |
| Understand saving and project files | [Projects and saving](projects.md) |
| Navigate the viewer, use inspection and scopes | [The viewer](viewer.md) |

## Why the grading feels familiar

Most negative-conversion tools work on an already-developed image: the sliders correct a
picture someone else's software already made. Antitone instead works from the camera's
raw measurements and develops each scan into a proper positive first, in a common,
colour-managed working space — and only then applies your grade. Grading therefore acts
on an image in which ordinary photographic controls mean what they say.

You do not need the theory to use the software. If you want it later:
[how processing works](../docs/processing.md) and
[how Analyze chooses its starting grade](../docs/roll-analysis.md) are written for
contributors and go deeper than this guide.

## The three panels

Antitone organises the work into three tabs, matching the order you use them:

- **Roll** — everything shared by the whole roll: scan orientation, flat-field
  illumination correction, calibration (Dmin/Dmax), the negative response, roll white
  balance and roll exposure. This is also where **Analyze** lives.
- **Photo** — everything about one photograph: crop and straighten, dust repair, and
  your photographic grade (exposure, white balance, contrast, saturation, local
  adjustments).
- **Rendering** — the creative look and the output colour space used for the preview and
  TIFF export.

Changes in Roll affect every photograph in the project; changes in Photo affect only the
one you are working on. Rendering is a project-wide output choice.

For today's input support, platform scope and what is not implemented yet, see
[current state](../README.md#current-state).

![The Antitone window with a roll loaded: filmstrip, viewer and the Roll panel](../assets/docs/window-overview.png)
