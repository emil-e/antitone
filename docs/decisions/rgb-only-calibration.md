# Store calibration references as RGB values only

Status: accepted.

## Choice and rationale

Dmin/Dmax each persist one native-camera RGB value. Picker source, rectangle and working-photo return context are transient; re-picking starts on the current photo with an empty region.

A measurement is independent of the continued availability of the photograph it came from. Missing, changed or removed photos must not erase or silently resample calibration. Development, camera-basis, floor and model compatibility remain roll-level constraints. Stable IDs/paths belong to source records; RAW byte changes do not invalidate calibration or require separate acceptance.

Full-resolution compensated means, calibrated-domain validation and invalid/cancelled-pick atomicity remain required.

[Processing contract](../contracts/processing-model.md#inversion-and-validation) · [Project state](../contracts/project-state.md)

Implementation: `src/editor.rs`, `src/project.rs`, `src/editor_tests.rs`, `tests/project.rs`.
