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

Use **Set up…** in Roll's **Flat-field correction** section to select a reference and adjust the illuminated area, then Apply or Cancel. Once accepted, the section shows the reference filename and an enable control. Its Reset removes correction so setup can be run again.

Setup uses untransformed source with automatic embedded lens development. Adjust the suggested rectangle to exclude the holder and other unsuitable content, while retaining the image-centre patch used to establish the target.

The estimator compares local full-resolution camera statistics with a centre-luminance, AP1-neutral target and smooths the resulting field, validating positive finite means and representable gains. Capture the reference without clipping: saturation erases the illumination differences needed for correction. Lighting, aperture, focus and positioning must match the scanning setup. Developed values above one after embedded shading are valid headroom, not evidence of capture clipping.

The [reference tool](../src/editor/flat.rs) owns preparation, selection and viewing; Cancel or replacement discards it. Apply captures the prepared uncorrected source and rectangle, then closes the reference view immediately. A [correction task](../src/editor/flat.rs) owns a document edit while it estimates the map, validates recorded source domains and prepares the GPU correction before publishing document metadata, correction and history together. Enable, disable and Reset use the same acceptance path. Selected-photo loading and reusable camera inputs remain with their existing visit. Failure preserves the previous correction and working photo, with [acknowledged feedback](editor.md#dialogs-return-answers-to-their-workflows). A newly registered reference stays out of the filmstrip as a reference-only record.

## Apply gains before inversion

The map is evaluated at original developed-source pixel centres. It interpolates inside its support and gives identity gains outside it — there is no extrapolated correction over the holder. Correction belongs in camera RGB because it compensates the light measured by the scan; inversion is nonlinear, so a gain applied afterward to the positive would mean something different. Crop, turn and straightening do not relocate the illumination pattern in source coordinates, and the retained camera source is unchanged by illumination gains.

Sony embedded shading/geometric development and DNG SDK opcodes run before measured correction. The map therefore estimates residual setup illumination in developed-source coordinates. Reopening reuses the accepted numeric map with the current decoder; lens/opcode fingerprints do not gate its use. Capture the reference with matching optics and illumination, and remeasure when the physical setup changes.

## Correction and measurements

Amplification increases quantization error along with the signal. Measurement uses the source precision estimate and largest map gain to avoid poorly resolved evidence. Calibration remains the measured Dmin/Dmax RGB values; correction does not rewrite them or set the inversion's numerical domain. After changing correction, re-picking the anchors may be appropriate.

## Save the accepted result

Projects save the map, enable state and reference source ID, allowing correction to be used or reset even if the reference RAW is missing. Open prepares its runtime resource independently of metadata acceptance. If preparation fails, this section reports the error and the viewer falls back to source inspection; Save retains the enabled map. Measurements and Export capture its preparation result with their inputs, await it when acquiring those inputs, and receive the same resource or preparation failure shown here. The reference source record is retained only while the map is accepted; clearing or replacing correction removes it unless the same source remains an ordered photo. Changing input stage clears the accepted correction and its reference record, because the map belongs to the stage in which it was measured; the dormant scan basis remains. Digital does not apply the map. Changing, enabling or resetting it preserves composition and inspection mode. [Projects](projects.md#compatibility-is-about-the-processing-basis) owns the switching and compatibility boundary.

## Working on this area

[Scan](../src/scan.rs) gathers reference statistics; [flat-field](../src/flat_field.rs) owns estimation, interpolation and precision; [GPU evaluation](../src/gpu/flat.rs) shares the source mapping. [Editor correction](../src/editor/flat.rs) connects reference interaction to document acceptance, and [project correction](../src/project/flat.rs) owns saved-map validation.

Start with [project tests](../tests/flat_field_project.rs), [export tests](../tests/flat_field_export.rs) and [editor tests](../src/editor/flat/tests.rs). [Diagnostics](diagnostics.md#flat-field-correction) provides sample-dependent exercises.