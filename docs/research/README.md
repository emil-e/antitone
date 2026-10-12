# Research knowledge base

Flexible notes, studies, experiments and supporting evidence. No mandatory template, numbering or completeness checklist. Read by question, not in chronological bulk. Distinguish measured results, hypotheses and limitations; flag superseded conclusions when they could mislead. Promote accepted conclusions into [contracts](../README.md#current-contracts) or [decisions](../decisions/README.md), not automatically into production.

## Rendering / colour management

- [ACES yellow-collapse mechanism](aces-yellow-collapse.md): concise reproducible finding; current resolution is unchanged ACES with sRGB/P3.
- [ProPhoto diagnosis](prophoto-gamut-diagnosis.md) and [100-nit SDR Rec.2020 gate](rec2020-rendering-gate.md): rejected output candidates, not pending replacements.
- [P3 output implementation](p3-output-update.md): completed output-update evidence, with historical test counts.

## Inversion / shadows / film response

- [Dmin-first shadow trace](dmin-shadow-trace.md): completed dog-frame hard-clipping diagnosis, source measurements and limits; not exact live-session/NLP truth.
- [DRT-aware inversion exploration](drt-aware-inversion-exploration.md): historical alternative; neither lift nor pedestal was deployed.
- [Published film-response model study](film-response-model-study.md): manufacturer drawings support a generic toe candidate, not camera/stock calibration or statistical median.
- [Corrected Dmin plus toe image check](film-response-image-validation.md): historical photographic validation leading to approved integration.
- [GPU precision/source/production gate](film-response-precision-gate.md): current integration closure plus clearly separated failed/superseded probes and preserved Original limits.

## Product observations and reproduction

- [UI polish audit](ui-polish-audit.md): current-code/historical-layout findings, verified darktable/Affinity/Apple patterns and native-menu feasibility; recommendations live in the linked proposal.

- [Feedback inbox](../feedback.md): new user observations awaiting triage. Completed or already-planned items are removed; remaining symptoms and uncertainties move into the relevant [work plan](../plans/README.md). The inbox is not the persistent requirement record or execution queue.
- [Historical records](history/README.md): earlier product/execution/feedback plans and mockup.
- [Authored tools](tools/README.md): durable source outside `.cache`, with compatibility status.
- [Selected evidence](evidence/README.md): compact logs retained outside `.cache`; photographic and third-party inputs stay local.

There is room here for short observations, long investigations, external references and unresolved questions. Add links where useful; do not force every note into an ADR or work-plan format.
