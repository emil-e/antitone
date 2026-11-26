# Correcting scan illumination

A photograph of a negative contains both the film image and the scanning setup's illumination. Uneven backlighting, lens shading or colour variation across the field can therefore look like part of the photograph.

A **flat-field reference** is a film-free photograph of the light source made with the same scanning setup. Antitone estimates a smooth camera-RGB gain map from it, so later scans receive the corresponding correction before negative inversion.

```text
bare-light reference → accepted gain map
                              ↓
negative scan → camera-RGB correction → inversion → positive grade
```

This is scan correction, not film-base calibration, dust removal or a colour grade painted over the positive.

## Choose the illuminated support

Use **Choose reference…** in Roll's **Flat-field correction** section to select a reference and adjust the illuminated area, then Apply or Cancel. Once accepted, the section shows the reference filename and an enable control. Its Reset removes correction so setup can be run again.

Setup uses untransformed source with automatic embedded lens development. Adjust the suggested rectangle to exclude the holder and other unsuitable content while keeping the image centre inside it.

The estimator fits a smooth illumination surface per camera channel and corrects the field to match the centre. Each channel has unity gain at the centre, preserving its brightness and colour. Capture the reference without clipping: saturation erases the illumination differences needed for correction. For useful correction, match the reference's lighting, aperture, focus and positioning to the photographs being corrected. Developed values above one after embedded shading are valid headroom, not evidence of capture clipping.

The [reference tool](../src/editor/flat.rs) owns preparation, selection and viewing; Cancel or replacement discards it. Apply captures the prepared uncorrected source and rectangle, then closes the reference view immediately. A [correction task](../src/editor/flat.rs) owns a document edit while it estimates the map and prepares its GPU copy before accepting the values and recording history. Map preparation depends on the map and GPU limits. Photographic consumers check their captured source identity, geometry and numerical execution when using it. Enable, disable and Reset use the same acceptance path. Selected-photo loading and reusable camera inputs remain with their existing visit. Failure preserves the previous correction and working photo, with [acknowledged feedback](editor.md#dialogs-return-answers-to-their-workflows). The reference filename is descriptive provenance.

## Apply gains before inversion

The map is evaluated at original developed-source pixel centres. It interpolates inside its support and gives identity gains outside it — there is no extrapolated correction over the holder. Correction belongs in camera RGB because it compensates the light measured by the scan; inversion is nonlinear, so a gain applied afterward to the positive would mean something different. Crop, turn and straightening do not relocate the illumination pattern in source coordinates, and the retained camera source is unchanged by illumination gains.

Sony embedded shading/geometric development and DNG SDK opcodes run before measured correction. The map therefore estimates residual setup illumination in developed-source coordinates. Reopening reuses the accepted numeric map with the current decoder; lens/opcode fingerprints do not gate its use. Capture the reference with matching optics and illumination, and remeasure when the physical setup changes.

## Correction and measurements

Changing correction changes the camera light measured by the pickers, but does not rewrite accepted Dmin and Dmax. Re-pick the anchors if the new correction changes the light at their reference locations.

## Save the accepted result

The project's optional `flat_field` saves the numeric map, enable state and reference filename as descriptive provenance. Choose and apply the reference again to re-estimate the saved map.

Correction owns shared GPU preparation of the accepted numeric map. Processing requests retain the correction current when captured and await its working copy when needed; later edits or document replacement leave their inputs unchanged. Open and Undo/Redo restore accepted values independently of preparation. Failures appear here and the viewer falls back to source inspection; Save retains the enabled map.

Importing or removing the reference as a photo is independent of correction. Input-stage changes clear accepted correction after any required confirmation; Digital does not apply it. Changing, enabling or resetting correction preserves composition and inspection mode. [Projects](projects.md#source-interpretation-and-identity) owns stage switching and source identity.

## Working on this area

[Reference measurement](../src/flat_field/reference.rs) reads uncorrected developed pixels for support and gain estimation. The [flat-field model](../src/flat_field.rs) fits and interpolates the gain map; the [correction stage](../src/photographic/stages/flat_field.rs) owns the prepared gain map and its application to camera light. Correction and grading share [source-coordinate mapping](../src/acquisition/camera/coordinates.rs). [Editor correction](../src/editor/flat.rs) connects reference interaction to document acceptance; [project correction](../src/project/flat.rs) owns saved-map validation.

Start with [project tests](../tests/flat_field_project.rs), [export tests](../tests/flat_field_export.rs) and [editor tests](../src/editor/flat/tests.rs). [Diagnostics](diagnostics.md#flat-field-correction) provides sample-dependent exercises.