# Add adjustable film response without reinterpreting existing grades

Status: accepted; production integration complete for the tested domain.

## Context

Corrected Dmin resolves most hard black clipping in the examined grade; surviving positive shadows remain strongly compressed. Published film curves and actual-image comparisons support a separately adjustable toe, not an ACES-derived lift, hidden pedestal or claimed stock calibration. Near-base and high-exposure arithmetic required additional temporary precision.

## Decision

Retain Original/model 2 unchanged. Explicit new grades use Film/model 3 with shared toe beta 2.5 and shoulder 0/off, within validated ranges/domain. Existing grades require explicit opt-in. Preserve Dmin→0/Dmax→1, actual-response fitting and signed/HDR data. Use the approved signed-root continuation above measured base as a numerical/noise policy, not physical below-base exposure. Store/emit actual model and shape.

## Consequences

Schema 3 remains; absent response with model 2 retains Original, while response/model mismatches reject. New-grade factories are distinct from numerical compatibility defaults. Reject floor-domain poles, unsupported precision/range and response exponents beyond the supported GPU domain before mutation. No gate weakening or changes to RAW, ACES, ICC or AP0 EXR. Model 3 resolves the tested candidate cancellation cases; original model-2 expanded extreme-EV limitations remain documented, not silently fixed by regrading. The generic default is not a population median or calibrated physical film profile.

## Evidence

- [Versioned processing contract](../contracts/processing-model.md#versioned-film-response), [controls](../contracts/editor-controls.md).
- [Manufacturer study](../research/film-response-model-study.md), [Dmin trace](../research/dmin-shadow-trace.md), [photographic check](../research/film-response-image-validation.md).
- [Production precision/rendering checkpoint](../research/film-response-precision-gate.md#production-film-response-checkpoint), [completed integration plan](../plans/completed/film-response.md).
- `tests/{film_response,film_project,film_export,gpu_film_response}.rs`; subsequent user testing reported that it works well. That is real-use confirmation, not universal domain/device proof.
