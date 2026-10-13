# Product and scope

Antitone converts camera-scanned film negatives into grading-friendly positive images using explicit roll calibration, small per-photo adjustments and export for finishing elsewhere. It does not promise exact scene radiometry, dye reconstruction or stock identification, and does not use opaque adaptive per-image inversion.

- macOS/Apple Silicon first; exposed photo input is ARW RAW, with Sony ILCE-1M2 lossless-compressed ARW as the supported camera baseline. Saved input is schema-5 JSON (default `.antitone.json`); developed JPEG/TIFF/EXR import is not implemented. Reuse established RAW development rather than camera-specific hacks. No minimum chip/RAM requirement is established. Resource policy follows observed needs, not an assumed hardware baseline.
- A roll is an explicitly selected ordered set of external, untouched photographs, not a watched folder/catalog. Shared roll edits and individual photo edits are separate scopes.
- Native-camera inversion/calibration with explicit camera-model/matrix identity; photographic working space AP1/D60; signed/HDR AP0 EXR and matching sRGB/Display P3 rendered TIFF. [Processing](processing-model.md) and [output](output.md) define exact behavior.
- Four full-resolution single-patch corrections: Dmin, Dmax, roll gamma neutral and photo WB neutral. Invalid or cancelled picks leave processing state unchanged. Stored reference values are independent of photo availability.
- Developed-source inspection is an explicit bypass; inspection convenience must not silently change development, calibration or exports.
- Interactive GPU processing is required. Responsive approximate previews are followed by full-source-derived settled regional previews; never use proxy pixels for measurement or export.
- UI controls and neutral fitting share actual state, with independent true-default resets, precise numeric access and no hidden strength/baseline or Kelvin claim. [Control contract](editor-controls.md).
- Explicit Save/reopen and dirty-document protection are implemented. Recovery, relinking/source-change UI and undo remain required work. [Project state](project-state.md).

## Boundaries and future goals

[PLAN.md](../../PLAN.md) is the only current priority/status list. Navigation/source inspection, bounded thumbnails and responsive display-referred proxy scopes are implemented. Native-camera inversion and accepted composition are integrated. Photo contrast/pivot and saturation are implemented with explicit signed-AP1 operators and identity defaults; [processing](processing-model.md#photo-contrastpivot-and-saturation) defines them. Remaining goals include interaction/photographic acceptance and assisted roll analysis; additional tools are not specified merely because they appear in feedback. Consult the user about each existing viewer-intrusion case before choosing a layout replacement.

Roll-level flat-field correction is implemented; [processing](processing-model.md#optional-roll-level-flat-field-correction) and [controls](editor-controls.md#flat-field-workflow) define its behavior. Custom demosaicing, dust removal, catalogs, arbitrary node graphs, HDR display output and later-platform support are not current implementation scope. Do not change ACES mathematics or introduce a pedestal/clamp as an implicit remedy.

Samples are read-only/local; Lightroom/Resolve automation is outside the supported workflow. Native-dependency packaging/licensing and physical/device acceptance are required before distribution; the current checkout is not release-ready.
