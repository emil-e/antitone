# Research knowledge base

Studies, experiments, measured findings and supporting evidence, organized by question. Reports distinguish observations, hypotheses, limitations and superseded conclusions. Current accepted behavior is defined in [contracts](../README.md#current-contracts) and [decisions](../decisions/README.md).

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

- [Feedback inbox](../feedback.md): new user observations awaiting triage. Completed or already-planned items are removed; remaining symptoms and uncertainties move into the relevant [work plan](../plans/README.md). The inbox is not the persistent requirement record or execution queue.
- [Current tools](tools/README.md): verification, extraction and capture helpers, separated from historical experiments.
- [Selected evidence](evidence/README.md): archived numerical/verification results; photographic and third-party inputs stay local.
- [Historical records](history/README.md): earlier product/feedback plans, execution/checkpoint narratives, archived tooling notes and mockup.
