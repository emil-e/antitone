# Measure scopes in the selected output

Scopes help judge the rendered photograph. Measuring the selected output's native encoded RGB, before monitor conversion, keeps them tied to that photograph rather than the monitor or an unrelated analysis gamut.

A P3 target therefore uses P3-derived luma/chroma conventions. Equal codes across targets are not asserted to be equal colours. Visible-region sampling lets zoom/pan isolate a face or subject while sharing viewer processing. Fit covers the composition. The positive working-view prototype now measures the accepted target-linear viewer result, including its explicit original-sample comparison, without another grade, ACES transform or composition. The approximation remains visible: scope reduction has its own bounded sampling density, and initial proxy presentation can miss detail. Neither path claims full-resolution file statistics.

Vectorscope magnification is independent of grade and widget size. Re-binning cached rendered samples reveals detail without changing the photograph or rerunning ACES. Full-source statistics are a separate feature with different costs.

[Scopes](../scopes.md) explains interpretation and coverage. [Measurement code](../../src/scopes.rs) defines the equations and plot conventions.
