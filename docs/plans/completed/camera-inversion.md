# Native-camera inversion replacement — completed

Status: completed implementation, delivered on `main` in `5efb302` (`Replace AP0 inversion with native-camera RGB`). The accepted decision replaces AP0 inversion outright; photographic/onscreen review remains a separate acceptance limit, not an unfinished implementation step. [The roadmap](../../../PLAN.md) owns current priority.

## Scope and decisions

Calibrate, floor, fit channel gamma and invert developed native-camera RGB; apply optional flat-field gains there first. Map inverted positives through the metadata camera→AP1 matrix, then retain AP1 exposure/Bradford WB and unchanged ACES/output processing. Preserve response mathematics, signed/HDR data, compact RGB16 source and precise shader temporaries. No PCA transform, alternate AP0 path or old-grade migration.

Schema/model 4 and camera-inversion development identity explicitly reject old AP0 projects. A retained roll camera model/matrix rejects incompatible sources and transient references. Persisted base floors equal one native code; correction derives one shared runtime bound without resampling Dmin/Dmax. [Accepted decision](../../decisions/native-camera-inversion.md), [processing contract](../../contracts/processing-model.md), [project contract](../../contracts/project-state.md).

## Completed steps

1. [x] Scalar camera→AP1 metadata matrix, native calibration/Scan/fitting/code floor and unchanged physical flat-field neutral target through AP1/XYZ.
2. [x] One precise GPU response/source path with matrix after inversion, negative-continuation/range validation and unchanged tolerances; no retained wide source or legacy inversion branch.
3. [x] Editor/thumbnails/refinement/export and schema/model/development metadata; explicit response, retained camera basis and exact base/corrected floors.
4. [x] Scalar/project/Metal/target/editor checks, real train and corrected Save/Open/small TIFF/AP0 EXR regressions, strict Clippy, formatting and release editor build.
5. [x] One independent implementation review; basis/floor findings fixed with targeted regressions. Contracts/architecture/ADRs updated.

## Verification and delivery record

The implementation began in `.worktrees/camera-inversion` on `impl/camera-inversion`, initially using the copied flat-field snapshot from `ace5628`, then transplanted onto finalized flat-field `c0b664a`. It was committed and integrated onto main as `5efb302`; the earlier “main unchanged/uncommitted” handoff is obsolete. Disposable cache/transplant files are not the authoritative implementation.

[Retained verification evidence](../../research/evidence/camera-inversion.txt) records the pre-integration environment, commands and results: 151 all-feature tests passed/3 ignored, strict all-target Clippy, formatting and release build; explicit release real-RAW train and corrected Save/Open/small-export tests passed. These are historical snapshots, not fresh runs during plan closure. The later [geometry integration](flip-crop-rotate.md#integrated-native-camera--flat-field-checkpoint) preserves this model and verifies combined original-source flat-field/composition paths.

## Limits and follow-up

Finite scalar/Metal agreement does not establish calibrated stock/scene colour, universal metadata/domain support or photographic/physical-monitor acceptance. No new onscreen smoke or expensive full-size export was run during the camera rebase; the evidence preserves those limits. The pre-existing Original unit-response extra-field serde limitation remains reported in the project contract, not silently fixed here.

No active implementation remains in this package. Further photographic review belongs to [completion acceptance](../completion.md#geometry); broader device/monitor/interoperability checks belong to [release work](../completion.md#export-and-release-work). Address concrete findings only under a new request; do not resume historical transplant or implementation steps.
