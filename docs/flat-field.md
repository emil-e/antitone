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

The reference workflow opens an uncorrected view, temporarily bypassing photo composition and orientation. An automatic rectangle is a suggestion; the user can edit it to exclude the holder and other unsuitable content. The support must contain the image-centre patch used to establish the target.

The estimator compares local full-resolution camera statistics with a centre-luminance, AP1-neutral target. It rejects clipped or unusable support and smooths the resulting field. Compatibility checks cannot establish that lighting, aperture, focus and positioning physically match the roll; the reference must actually represent its scanning setup.

Apply prepares the corrected candidate before accepting it. Cancel or failure restores the working photo and accepted state. Reference-only sources do not become filmstrip photographs. Rejected support changes leave the area and processing unchanged; reference/preparation failures retain an [acknowledged feedback message](editor.md#decisions-are-not-operation-failures), not a partially accepted map.

## Apply gains before inversion

The map is evaluated at original developed-source pixel centres. It interpolates inside its support and gives identity gains outside it—there is no extrapolated correction over the holder.

Correction belongs in camera RGB because it compensates the light measured by the scan. Inversion is nonlinear, so a gain applied afterward to the positive would mean something different. Crop, turn and straightening do not relocate the illumination pattern in source coordinates.

The retained RGB16 source is unchanged. Measurements, analysis, GPU previews, thumbnails, settled reconstruction and exports evaluate the same accepted map as they process it.

## Correction also changes the precision bound

Amplifying a camera channel amplifies its quantization step. Antitone therefore derives a conservative effective input floor from the largest map gain, including the identity region.

The project still stores the native one-code floor. Processing substitutes the corrected bound into a runtime calibration copy exactly once; it does not save an already-amplified floor and amplify it again on Open. This is a precision constraint, not a noise measurement or a brightness pedestal.

Enabling correction can make an existing calibration or curve unsupported. Such a proposal is rejected rather than silently repaired. Dmin/Dmax are never automatically resampled; after changing correction, re-picking them may be appropriate.

## Save the accepted result

Projects save the map and its development identity, not merely instructions to regenerate it. A missing nonordered reference does not prevent using the saved map. Editing its area does require reopening a compatible reference.

Changing, enabling or clearing a map preserves the working composition and inspection mode. Removing correction restores the native floor. [Projects](projects.md) explains the compatibility boundary.

## Working on this area

[Scan](../src/scan.rs) gathers reference statistics; [flat-field](../src/flat_field.rs) owns estimation, interpolation and precision; [GPU evaluation](../src/gpu_flat_field.rs) shares the source mapping. [Editor correction](../src/editor_flat_field.rs) owns the temporary reference transaction, and [project correction](../src/project_flat_field.rs) owns saved-map validation.

Start with [project tests](../tests/flat_field_project.rs), [export tests](../tests/flat_field_export.rs) and [editor tests](../src/editor_flat_field_tests.rs). [Diagnostics](diagnostics.md#flat-field-correction) provides sample-dependent exercises. Matching software results is not proof that a physical reference matches the scans.
