# Invert native-camera RGB, then map positives to AP1

Status: accepted.

## Choice and rationale

Calibrate, floor, fit gamma and invert developed native-camera RGB, with optional flat-field gains applied first. Channel-wise nonlinear inversion does not commute with a colour matrix. Camera channels are the most direct available measurements of the negative; colour-mixed working-space coordinates are not physical film-dye measurements.

Map inverted positives through the metadata-derived camera→AP1 matrix, then apply exposure and photographic Bradford WB. AP1 supplies a standard photographic working space; AP0 belongs only to EXR interchange and unchanged ACES internals. Preserve signed/HDR values throughout working processing.

Manual gamma-neutral fitting targets `inverse(camera_to_AP1) * [1,1,1]`, not equal camera channels. Analyze initializes channel curves separately with WB-invariant tonal drift, then shared photographic WB; neither changes the native inversion boundary. Retain camera model/matrix identity with the roll: incompatible native bases cannot share calibration anchors.

Use explicit processing model 4 and project schema 5, one RGB16 source path and accurate shader temporaries. New grades use a shared toe of 2.5 and shoulder off; the classic toe-1 curve uses the same camera model. Adjustable toe separates near-base response shape from display rendering. Signed-root continuation provides defined above-base noise behavior without a hidden pedestal.

## Limits

Spectral overlap and film/scene information loss remain. The positive camera matrix is a pragmatic baseline, not calibrated scene radiometry, dye unmixing or stock identification. Toe defaults are generic, not a calibrated film profile. Working-space brightness is not a colour-accuracy criterion. These limits do not justify stock-specific repair, source clamps or altered ACES mathematics.

[Processing contract](../contracts/processing-model.md) · [Project identity](../contracts/project-state.md) · [Output](../contracts/output.md)

Implementation: `src/{raw,scan,processing,sampling,gpu}.rs`, owned shaders and `tests/camera_inversion.rs`.
