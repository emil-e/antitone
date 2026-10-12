# Native-camera inversion replacement — completed

Status: implemented in `5efb302`. Native-camera inversion replaces AP0 inversion outright; photographic review remains separate from implementation completion. [Current roadmap](../../../PLAN.md).

## Scope and result

Calibration, input floors, channel-gamma fitting and inversion operate on developed native-camera RGB, after optional source-coordinate flat-field correction. Inverted positives then use the metadata camera→AP1 matrix, AP1 exposure/Bradford WB and unchanged ACES/output processing. Signed/HDR data, response mathematics and the compact RGB16 source are preserved.

Schema/model 4 rejects old AP0 projects without migration. A retained roll camera model/matrix rejects incompatible photo and reference sources; runtime correction derives a shared precision bound without resampling Dmin/Dmax.

- [Accepted decision](../../decisions/native-camera-inversion.md)
- [Processing contract](../../contracts/processing-model.md)
- [Project contract](../../contracts/project-state.md)

## Verification and limits

[Numerical/source/editor/output evidence](../../research/evidence/camera-inversion.txt) records scalar/project/Metal checks, real-train processing, corrected Save/Open/small exports and build verification. [Geometry integration](flip-crop-rotate.md#integrated-native-camera--flat-field-checkpoint) covers combined original-source correction and composition. These are recorded checkpoints, not current test-run claims.

Finite agreement does not establish calibrated stock/scene colour, universal metadata/domain support, photographic appearance or physical-monitor matching. Full-size export and onscreen acceptance were not rerun during the camera rebase. The Original unit-response extra-field serde limitation remains documented in the project contract.

Photographic follow-up is in [completion work](../completion.md#geometry); broader device/monitor/interoperability checks are in [release work](../completion.md#export-and-release-work).

[Archived implementation record](../../research/history/implementation/camera-inversion.txt) preserves the original checkpoints and integration history.
