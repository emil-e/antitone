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

The estimator compares local full-resolution camera statistics with a centre-luminance, AP1-neutral target. It rejects clipped or unusable support and smooths the resulting field. Valid developed camera means may exceed native white after embedded shading: capture/development eligibility, not a post-gain value of 1, determines clipping. Software compatibility cannot establish that lighting, aperture, focus and positioning physically match the roll — the reference must actually represent its scanning setup.

Apply estimates the map from the prepared uncorrected reference and validates the corrected working source before acceptance. Cancel or failure restores the working photo and accepted state; preparation failures use [acknowledged feedback](editor.md#decisions-are-not-operation-failures). The reference session owns its source and task independently of temporary photo visits; reference-only sources stay out of the filmstrip. [Architecture](architecture.md) describes that lifecycle.

## Apply gains before inversion

The map is evaluated at original developed-source pixel centres. It interpolates inside its support and gives identity gains outside it — there is no extrapolated correction over the holder. Correction belongs in camera RGB because it compensates the light measured by the scan; inversion is nonlinear, so a gain applied afterward to the positive would mean something different. Crop, turn and straightening do not relocate the illumination pattern in source coordinates, and the retained camera source is unchanged by illumination gains.

For Sony lens-corrected sources, both reference and scans receive the same embedded shading and geometric development first. The measured map then estimates residual setup illumination rather than applying the embedded correction a second time — making embedded shading useful without requiring a flat reference. Accepted maps require identical lens-development identity as well as dimensions, camera basis and development identity; different per-capture lens maps reject even if their output dimensions match. This baseline estimates a smooth illumination ratio in corrected space; a prewarp illumination hook is separate possible follow-up work. DNG references likewise retain SDK opcode development before residual correction, with matching source interpretation required exactly.

## Correction also changes the precision bound

Amplifying a camera channel amplifies its quantization step. Antitone therefore derives a conservative effective input floor from the largest map gain, including the identity region. The project calibration still stores the native one-code floor; each source separately records its developed floor, including embedded shading, and processing multiplies that source bound by the residual map's largest gain exactly once in a runtime calibration copy. Open does not amplify an already-corrected calibration again.

This is a precision constraint, not a noise measurement or a brightness pedestal. Enabling correction can make an existing calibration or curve unsupported; such a proposal is rejected rather than silently repaired, and Dmin/Dmax are never automatically resampled — after changing correction, re-picking them may be appropriate.

## Save the accepted result

Projects save the map and its development identity, allowing correction to be used or reset even if the reference RAW is missing. Changing input stage clears the accepted correction, because the map belongs to the stage in which it was measured; the dormant scan basis remains. Digital does not apply the map or enforce its compatibility on Digital sources. Changing, enabling or resetting a map preserves composition and inspection mode; removing it restores the developed-source floor, which can still include embedded shading. [Projects](projects.md#compatibility-is-about-the-processing-basis) owns the switching and compatibility boundary.

## Working on this area

[Scan](../src/scan.rs) gathers reference statistics; [flat-field](../src/flat_field.rs) owns estimation, interpolation and precision; [GPU evaluation](../src/gpu_flat_field.rs) shares the source mapping. [Editor correction](../src/editor_flat_field.rs) owns the temporary reference transaction, and [project correction](../src/project_flat_field.rs) owns saved-map validation.

Start with [project tests](../tests/flat_field_project.rs), [export tests](../tests/flat_field_export.rs) and [editor tests](../src/editor_flat_field_tests.rs). [Diagnostics](diagnostics.md#flat-field-correction) provides sample-dependent exercises.