# Documentation map

Start with [the roadmap](../PLAN.md) and [repository workflow](../AGENTS.md). Read only the material relevant to the selected task; historical experiments are not mandatory startup context.

## Architecture and operation

- [Architecture](architecture.md): implemented modules, colour/data flow, ownership, workers, GPU resources and gaps.
- [Build and probes](build-and-probes.md): current checkout-local commands and verification boundaries, not a rolling list of test counts.
- [Work plans](plans/README.md): focused scopes/acceptance/handoff; root roadmap owns status/order.
- [UI polish package](plans/ui-polish.md): agreed workspace/UX direction, audit and wireframe; first control-primitives slice is implemented, with remaining slices and visual limits explicit.

## Current contracts

- [Product/scope](contracts/product.md).
- [Processing models, source representation, calibration and tolerances](contracts/processing-model.md).
- [Editor controls, defaults, scope and interactions](contracts/editor-controls.md).
- [Image geometry, coordinate terminology and picker-view requirements](contracts/geometry.md).
- [Display-referred scopes](contracts/scopes.md): target-native waveform/vector conventions, approximate accepted-frame coverage and bounded lifecycle.
- [Rendering, presentation, TIFF/AP0 EXR and safety](contracts/output.md).
- [Project state](contracts/project-state.md): live stable-ID Save/Open, schema-4 identity, composition and flat-field persistence; recovery/relink/undo remain unwired.
- [Pinned OCIO artifacts](../assets/ocio/README.md) and [ICC profiles](../assets/profiles/README.md): asset definitions/checksums.

## Rationale and knowledge

- [Accepted decisions](decisions/README.md): why consequential choices were made; change only through explicit agreement.
- [Research knowledge base](research/README.md): loosely structured diagnoses, experiments, references and evidence; observations/hypotheses are not automatic contracts.
- [Research tools/cache policy](research/tools/README.md): authored sources preserved outside the cache, snapshot compatibility and local outputs.
- [Retained evidence](research/evidence/README.md): small checkpoint logs, environment and reproduction limits.
- [Historical plans/checkpoints](research/history/README.md): preserved original execution/product/feedback material, not current work queues.

When behavior changes, update its contract and architecture if boundaries change. Record consequential rationale in an ADR and task progress in the active plan. Do not repeat status/test counts across every document. See `AGENTS.md` for checkpoint/handoff rules.
