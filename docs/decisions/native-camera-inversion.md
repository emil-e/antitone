# Invert camera RGB before colour conversion

Inversion is nonlinear and channel-specific. A colour matrix mixes channels, so applying it before inversion changes the result. Native-camera RGB is the most direct developed measurement available; a mixed working-space channel is not a measurement of an isolated film dye.

We therefore calibrate and invert in camera RGB, then use the metadata-derived camera matrix to enter AP1 for photographic grading. This separates the negative's response from working-space and output conventions.

The matrix is a pragmatic positive colour conversion, not recovered scene radiometry. Sensor/film spectral overlap and information loss remain. Those limitations do not justify invented dye unmixing or hidden source repair.

[Processing](../processing.md) explains the model. [The scalar pipeline](../../src/processing.rs) and [camera-inversion tests](../../tests/camera_inversion.rs) define its implementation.
