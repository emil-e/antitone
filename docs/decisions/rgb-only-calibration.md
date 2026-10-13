# A calibration value is not its picker context

Dmin/Dmax persist native-camera RGB values, not a source dependency or selection rectangle. Once measured, a reference should remain usable if the originating photo is removed from the filmstrip or becomes unavailable.

Picker navigation, source and rectangle are transient interaction context. Keeping them out of calibration avoids accidental resampling and makes re-picking an explicit operation. Camera/development compatibility still belongs to the roll; independence from a source path is not independence from the measurement basis.

[Projects](../projects.md) explains ownership. [Project types](../../src/project.rs) and [editor pick tests](../../src/editor_tests.rs) define storage and invalid/cancelled-pick behavior.
