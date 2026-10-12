# Reuse established LibRaw development

Status: accepted; recorded from the established implementation agreement.

## Context

Camera-scanned negatives need controlled linear development without opaque per-image looks. Sony ILCE-1M2 compressed ARW is the initial source. A custom demosaicer/camera repair adds complexity without a demonstrated need; Apple RAW development was investigated but not selected.

## Decision

Use pinned LibRaw 0.22.0 black subtraction, one roll-fixed WB tuple and AHD development. Retrieve linear developed camera RGB16 before standard-space output matrix/curve clipping, then apply the documented camera conversion. No automatic brightness, per-photo as-shot WB substitution or camera-specific hack. The approved [parallel implementation](parallel-libraw.md) enables upstream OpenMP with bounded per-job resources and lazy diagnostics, without replacing this algorithm/settings/source contract.

## Consequences

Reuse a mature decoder/developer behind a narrow owned bridge, while taking responsibility for matrix/WB conventions, normalization, quantization, clipping diagnostics and reproducibility. Integer-developed samples are the source contract—not a promise of float RAW intermediates or calibrated scene radiometry. Changing the backend/development policy requires new validation and explicit compatibility handling. Native bundling/licensing remains distribution work.

## Evidence

- [Architecture](../architecture.md#data-and-colour-flow), [processing contract](../contracts/processing-model.md#camera-source-representation).
- [Historical investigation and alternatives](../research/history/product-plan.md#raw-development-options-and-probe), [execution evidence](../research/history/execution-plan.md#progress-evidence).
- `native/raw_bridge.cpp`, `src/raw.rs`, `src/scan.rs`; [RAW reproduction](../build-and-probes.md#raw-and-complete-gpu-chain).
