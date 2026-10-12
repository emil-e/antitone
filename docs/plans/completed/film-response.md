# Adjustable film response and compact GPU source — completed

Status: completed response/source integration. Historical AP0/model-2/3 compatibility clauses were subsequently superseded by the [native-camera replacement](camera-inversion.md); response mathematics/defaults and compact-source precision policy remain.

## Result

- Adjustable toe/shoulder response, actual-response full-resolution neutral fitting, scoped controls/defaults and explicit response state.
- New-grade toe 2.5/shoulder off and signed continuation, with anchor and signed/HDR behavior preserved.
- One exact RGB16 production source; extra precision lives in shader temporaries/constants, not a wide retained cache.
- Matching CPU/GPU, preview/refinement and TIFF/AP0 EXR response behavior with unchanged RAW development, ACES and output encoding.

All current response shapes use camera processing model 4; old AP0 grades reject without migration. See the [processing contract](../../contracts/processing-model.md), [response decision](../../decisions/versioned-film-response.md) and [source decision](../../decisions/rgb16-gpu-source.md).

## Verification and limits

[The precision report](../../research/film-response-precision-gate.md#production-film-response-checkpoint) and [selected evidence](../../research/evidence/README.md) retain the tested domains, arithmetic sweeps, target comparisons, controls/export checks and historical failures. Historical Original-model expanded extreme-EV cancellation limits remain documented; the AP0 snapshots do not establish universal model-4 limits.

Finite device/domain tests and user testing do not establish universal numerical-domain/device, macOS-14 or physical-monitor acceptance. Recovery/undo and flat-field correction were separate implementations, not acceptance implied by this response work.

[Archived implementation record](../../research/history/implementation/film-response.txt) and [original execution record](../../research/history/execution-plan.md#film-response-production-integration-approved) preserve the historical scope and checkpoints.
