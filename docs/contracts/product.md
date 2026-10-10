# Product and scope

Antitone converts camera-scanned film negatives into grading-friendly positive images using explicit roll calibration, small per-photo adjustments and export for finishing elsewhere. It does not promise exact scene radiometry, dye reconstruction or stock identification, and does not use opaque adaptive per-image inversion.

- macOS/Apple Silicon first; initial camera is Sony ILCE-1M2 lossless-compressed ARW. Reuse established RAW development rather than camera-specific hacks. No minimum chip/RAM requirement is established; M1 Pro/16 GB was a provisional example, not an acceptance gate.
- A roll is an explicitly selected ordered set of external, untouched photographs, not a watched folder/catalog. Shared roll edits and individual photo edits are separate scopes.
- Initial inversion/calibration basis AP0/D60; photographic working space AP1/D60; signed/HDR AP0 EXR and matching sRGB/Display P3 rendered TIFF. [Processing](processing-model.md) and [output](output.md) define exact behavior.
- Four full-resolution single-patch corrections: Dmin, Dmax, roll gamma neutral and photo WB neutral. Invalid or cancelled picks leave processing state unchanged. Stored reference values are independent of photo availability.
- Developed-source inspection is an explicit bypass; inspection convenience must not silently change development, calibration or exports.
- Interactive GPU processing is required. Responsive approximate previews are followed by full-source-derived settled regional previews; never use proxy pixels for measurement or export.
- UI controls and neutral fitting share actual state, with independent true-default resets, precise numeric access and no hidden strength/baseline or Kelvin claim. [Control contract](editor-controls.md).
- Explicit Save/reopen and dirty-document protection are implemented. Recovery, relinking/source-change UI and undo remain required work. [Project state](project-state.md).

## Boundaries and future goals

[PLAN.md](../../PLAN.md) is the only current priority/status list. Recorded goals include natural Mac interaction, thumbnails/scopes, geometry, inversion bases, assisted roll analysis and contrast/pivot. Their numerical/UX design is not settled merely because they appear in feedback. Consult the user about each existing viewer-intrusion case before choosing a layout replacement.

Flat-field correction remains separate deferred work. Custom demosaicing, dust removal, catalogs, arbitrary node graphs, HDR display output and later-platform support are not current implementation scope. Do not change ACES mathematics or introduce a pedestal/clamp as an implicit remedy.

Samples are read-only/local. No Lightroom/Resolve automation, global package installation, commits or pushes without separate authorization. Native-dependency packaging/licensing and physical/device acceptance are required before distribution; the current checkout is not release-ready.
