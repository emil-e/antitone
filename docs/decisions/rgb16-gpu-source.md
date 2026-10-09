# Retain compact developed RGB16 on the GPU

Status: accepted; replaces the float-AP0 source-upload path and rejects a proposed wider retained source cache.

## Context

Single-f32 AP0 upload erased distinct near-Dmin values needed by adjustable response. Wider converted-source caches could preserve them but unnecessarily increased retained image memory. LibRaw's developed RGB16 already contains the source information.

## Decision

Use one production upload representation: exact four-u16-slot developed camera RGB, 8 bytes/pixel; ignore the fourth slot. Perform accurate normalization/camera-to-AP0 conversion and response/colour arithmetic in shader temporaries with small precise constants. Remove obsolete float-source APIs/callers. Rendered-only outputs must not retain unused source/working/processing resources.

## Consequences

Source tiles halve relative to the former f32 AP0 path; precision costs arithmetic rather than wider retained images. Preflight rejects unsupported constants/products/ranges. Original model-2 consumes its former f32 AP0 bits and retains its equations; compatibility is mathematical/state compatibility, not duplicate source plumbing. A future float developer needs an explicitly matching source contract, not a speculative backend today. Historical wider-source prototypes remain research, not production alternatives.

## Evidence

- [Source contract](../contracts/processing-model.md#camera-source-representation).
- [Precision investigation and accepted source checkpoint](../research/film-response-precision-gate.md#production-rgb16-upload-checkpoint), [conversion log](../research/evidence/film-model-images/rgb16-conversion-tests.txt).
- `src/gpu.rs`, `src/shaders/camera_conversion.wgsl`, `src/viewer.rs`, `src/gpu_tests.rs`.
