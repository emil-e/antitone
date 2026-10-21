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

The reference workflow bypasses the accepted illumination map, photo composition and orientation, but retains automatic embedded lens development. An automatic rectangle is a suggestion; the user can edit it to exclude the holder and other unsuitable content. The support must contain the image-centre patch used to establish the target.

The estimator compares local full-resolution camera statistics with a centre-luminance, AP1-neutral target. It rejects clipped or unusable support and smooths the resulting field. Valid developed camera means may exceed native white after embedded shading; capture/development eligibility, not a post-gain value of 1, determines clipping. Compatibility checks cannot establish that lighting, aperture, focus and positioning physically match the roll; the reference must actually represent its scanning setup.

Apply prepares the corrected candidate before accepting it. Cancel or failure restores the working photo and accepted state. The reference session owns its captured uncorrected source/request, asynchronous task and saved working context independently of temporary photo visits. Estimation consumes the prepared reference without reopening it; corrected working-source preparation and device/compatibility admission finish before atomic acceptance. Proposals and completion wake outside painting. Retiring a session cancels its continuation, while running native/CPU work retains shared admission and resources until it returns. Reference-only sources do not become filmstrip photographs. Rejected support changes leave the area and processing unchanged; reference/preparation failures retain an [acknowledged feedback message](editor.md#decisions-are-not-operation-failures), not a partially accepted map.

## Apply gains before inversion

The map is evaluated at original developed-source pixel centres. It interpolates inside its support and gives identity gains outside it—there is no extrapolated correction over the holder.

Correction belongs in camera RGB because it compensates the light measured by the scan. Inversion is nonlinear, so a gain applied afterward to the positive would mean something different. Crop, turn and straightening do not relocate the illumination pattern in source coordinates.

The retained camera source is unchanged by illumination gains. Measurements, analysis, GPU previews, thumbnails, settled reconstruction and exports evaluate the same accepted map as they process it.

For Sony lens-corrected sources, both reference and scans receive the same embedded shading and geometric development first. The measured map then estimates residual setup illumination, rather than applying the embedded correction a second time. This preserves the reference workflow while making embedded shading useful without a flat reference. Accepted maps require identical lens-development identity as well as dimensions, camera basis and WB; different per-capture lens maps reject even if their output dimensions match. This baseline estimates a smooth illumination ratio in corrected space. It is not mathematically identical to multiplying gains before reconstruction, and software compatibility cannot establish physical reference suitability. A prewarp illumination hook is separate possible follow-up work.

DNG references likewise retain SDK opcode development before residual illumination correction. Matching geometry, WB and precision alone is insufficient: the source's DNG metadata/opcode interpretation must match the accepted reference exactly. Neither reference preparation nor later processing executes those opcodes a second time.

## Correction also changes the precision bound

Amplifying a camera channel amplifies its quantization step. Antitone therefore derives a conservative effective input floor from the largest map gain, including the identity region.

The project calibration still stores the native one-code floor; each source separately records its developed floor, including embedded shading. Processing multiplies that source bound by the residual map's largest gain in a runtime calibration copy exactly once. Open does not amplify an already-corrected calibration again. This is a precision constraint, not a noise measurement or a brightness pedestal.

Enabling correction can make an existing calibration or curve unsupported. Such a proposal is rejected rather than silently repaired. Dmin/Dmax are never automatically resampled; after changing correction, re-picking them may be appropriate.

## Save the accepted result

Projects save the map and its development identity, not merely instructions to regenerate it. A missing nonordered reference does not prevent using the saved map. Editing its area does require reopening a compatible reference.

Changing, enabling or clearing a map preserves the working composition and inspection mode. Removing the measured map restores the developed-source floor, which can still include embedded shading. [Projects](projects.md) explains the compatibility boundary.

## Working on this area

[Scan](../src/scan.rs) gathers reference statistics; [flat-field](../src/flat_field.rs) owns estimation, interpolation and precision; [GPU evaluation](../src/gpu_flat_field.rs) shares the source mapping. [Editor correction](../src/editor_flat_field.rs) owns the temporary reference transaction, and [project correction](../src/project_flat_field.rs) owns saved-map validation.

Start with [project tests](../tests/flat_field_project.rs), [export tests](../tests/flat_field_export.rs) and [editor tests](../src/editor_flat_field_tests.rs). [Diagnostics](diagnostics.md#flat-field-correction) provides sample-dependent exercises. Matching software results is not proof that a physical reference matches the scans.
