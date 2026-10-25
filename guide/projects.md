# Projects and saving

Antitone saves what you *decided*, not what you photographed. An `.antitone` project file
is a small JSON document holding your settings — calibration, grades, crops, dust
repairs, look — and **references to your RAW scans**. The scans themselves stay where they
are and are never modified.

## Saving and opening

- **File → Save** (⌘S) / **Save As…** (⇧⌘S) writes the project. Until then the title bar
  shows "Untitled • Modified".
- **File → Open…** reopens a project; **File → New** starts an empty one.
- Closing with unsaved changes asks first — and notes that "your RAW files are not
  changed" whatever you choose.
- Saving over an existing project asks **Replace project** before overwriting.

**Keep your scans.** Because a project references the original RAWs, moving or deleting
them breaks the project's ability to render and export. Keep them where they are, or
move them together with the project.

## What is stored where

| Thing | Where it lives |
| --- | --- |
| Your RAW scans | Their original location, untouched |
| Calibration, grades, crops, dust repairs | The `.antitone` project |
| The creative look | Embedded in the project (saved appearance doesn't depend on the LUT file's path) |
| Exported TIFF/EXR | The folders you exported to |

Dust repairs are part of the project and apply consistently to views, thumbnails and
exports.

## Managing the photos in a project

- **File → Add Scans…** adds more RAW files (references only).
- Select photos in the filmstrip and use the filmstrip's remove action to remove them
  **from the project** ("Remove N photos from project?") — this deletes their project
  settings and dust repairs but never deletes RAW files from disk. An accepted flat-field
  reference stays as a reference-only record until correction is cleared. **Auto-crop
  selected photos** applies auto-crop to a selection.

One project is one roll: scans that share a camera, lens and light. A project keeps its
calibration basis, so scans made under a fundamentally different setup belong in a
separate project.
