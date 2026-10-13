# Measure scopes in the selected output

Scopes help judge the rendered photograph. Measuring the selected output's native encoded RGB, before monitor conversion, keeps them tied to that photograph rather than the monitor or an unrelated analysis gamut.

A P3 target therefore uses P3-derived luma/chroma conventions. Equal codes across targets are not asserted to be equal colours. Whole-frame proxy sampling makes updates responsive, provided the approximation is visible; a regional settled viewer image cannot stand in for whole-frame statistics.

Vectorscope magnification is independent of grade and widget size. Re-binning cached rendered samples reveals detail without changing the photograph or rerunning ACES. Full-source statistics are a separate feature with different costs.

[Scopes](../scopes.md) explains interpretation and coverage. [Measurement code](../../src/scopes.rs) defines the equations and plot conventions.
