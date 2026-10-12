# Roll-level flat-field correction — completed

Status: implemented and user-accepted on 2026-10-11. The [native-camera replacement](camera-inversion.md) and [geometry integration](flip-crop-rotate.md#integrated-native-camera--flat-field-checkpoint) subsequently incorporated it into model/schema 4 and composed views.

## Scope and result

A separate unclipped reference RAW supplies a bounded smooth three-channel gain field over an explicitly accepted illuminated rectangle. The target is uniform gray with centre luminance preserved, not preservation of the reference's central tint. Automatic rectangle proposals remain editable; ambiguous support requires manual acceptance.

Gains apply in original source coordinates before native-camera inversion, consistently across source view, full-resolution measurements/fitting, GPU previews/refinement, thumbnails and exports. Dmin/Dmax remain RGB-only and are never silently resampled. The accepted map, support, Enabled state and reference/development identity persist; missing nonordered reference files do not prevent use of a saved map.

The Roll panel provides Choose reference RAW, Enabled, Edit area and Clear before Calibration. Failed/cancelled reference work preserves accepted state and returns to the working view. Reference-only records do not enter the filmstrip; existing photos remain visible.

Current behavior is defined by the [processing](../../contracts/processing-model.md), [project](../../contracts/project-state.md) and [geometry](../../contracts/geometry.md) contracts. No holder-specific geometry, changed RAW development, dust-removal feature or ACES modification was introduced.

## Verified estimator checkpoint

[Estimator evidence](../../research/evidence/flat-field-initial.txt) records independent full-resolution patches, boundary/centre errors, dust rejection and field size. The original AP0-baseline method and measured values are retained in the [archived implementation record](../../research/history/implementation/flat-field-correction.txt); they are not current compatibility requirements.

## Verified processing checkpoint

[Processing evidence](../../research/evidence/flat-field-gpu.txt) covers source-coordinate CPU/GPU gains, corrected precision floors, source inspection and both rendered targets. Temporary extra precision does not change RGB16 retention or stored calibration. Current native-camera/composition coverage is linked from the geometry closure above.

## Verified editor checkpoint

[Editor evidence](../../research/evidence/flat-field-editor.txt) covers map proposal/accept/cancel, transactional state, thumbnail completion, exact Save/Open, corrected small TIFF/AP0 EXR reopen and Enabled/view preservation. Headless checks do not establish native-dialog or physical-input acceptance. Earlier zero-visible-frame smoke failures remain recorded failures.

## Verified automated acceptance

[Acceptance evidence](../../research/evidence/flat-field-acceptance.txt) records visible-frame smoke on both approved RAW pairs and full-size sRGB/P3 TIFF plus half/full AP0 EXR reopen, including scalar-derived pixel/metadata checks and signed/HDR range. Inputs remain read-only.

Native file dialogs, physical gestures, physical-monitor matching, actual macOS-14 execution and broader-camera/device acceptance remain unperformed. The two reference pairs represent one scanning setup, not universal holder/illumination coverage. Further release acceptance is tracked in [completion work](../completion.md#export-and-release-work).
