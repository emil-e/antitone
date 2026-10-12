# Adjustable film response and compact GPU source

Status: completed for the tested domain; user approved integration/signed continuation and subsequently reported successful testing. This is a historical closure record, not an active queue. Its AP0/model-2/3 compatibility/version clauses were superseded by the [completed native-camera replacement](camera-inversion.md); response mathematics/defaults and compact-source precision policy remain.

## Historical agreed scope

Preserve existing Original/model-2 grades. New grades use Film/model 3, toe 2.5/shoulder off, with explicit opt-in for old grades. Keep anchors, signed/HDR values, actual-response full-resolution fitting, current calibration/control scopes and unchanged RAW/ACES/ICC/AP0 EXR. Use one exact RGB16 production source with precision in shader temporaries/constants, not a wider retained cache or obsolete upload shim.

## Completed steps

- [x] Isolated source/response precision/domain gates without weakening tolerances.
- [x] Compact RGB16 source replacement and exact conversion/original input-bit checks.
- [x] Versioned scalar/state/actual-model fitting and model-2 compatibility.
- [x] Precise model-3 GPU arithmetic, preflight rejection and preview/refinement integration.
- [x] Toe/shoulder controls, scoped defaults/resets, section order and original-grade opt-in.
- [x] Actual-response TIFF/EXR metadata and numerical/editor closure checks.

## Verification and limits

[The production checkpoint](../../research/film-response-precision-gate.md#production-film-response-checkpoint) is the detailed numerical record; [selected logs](../../research/evidence/README.md) retain small decisive results outside the cache. Scalar/GPU/complete-target, original camera-chain, state/export, controls/layout, Clippy/format/release and real editor checks passed at that checkpoint.

Original-model expanded extreme-EV cancellation limitations remain preserved/documented. Finite M5 tests and user testing do not establish universal numerical-domain, M1 Pro/16 GB, macOS-14 or physical-monitor acceptance. Editor persistence/recovery/undo and flat field were not part of this integration.

## Handoff

Do not reopen the completed source/response integration merely because historical prototypes failed. Use [current contracts](../../contracts/processing-model.md), [response ADR](../../decisions/versioned-film-response.md) and [source ADR](../../decisions/rgb16-gpu-source.md). Original approved plan/checkpoints are preserved in [the execution archive](../../research/history/execution-plan.md#film-response-production-integration-approved). Current next work belongs to [the roadmap](../../../PLAN.md).
