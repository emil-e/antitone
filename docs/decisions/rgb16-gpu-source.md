# Retain compact developed RGB16 on the GPU

Status: accepted.

## Choice and rationale

Retain exact developed camera RGB in four u16 slots, 8 bytes/pixel, with the fourth ignored. This preserves the source's integer information without a wide converted-source cache.

Normalize, evaluate optional gains/inversion and apply the post-inversion camera matrix using accurate shader temporaries and small precise constants. Near-base nonlinear response makes premature f32 rounding significant; extra arithmetic precision need not increase retained image storage.

Preflight rejects unsupported constants, products and ranges rather than choosing a different source representation or CPU fallback. Rendered-only outputs retain no unused source/working/processing resources.

[Source contract](../contracts/processing-model.md#camera-source-representation)

Implementation: `src/gpu.rs`, `src/shaders/{camera_conversion,film_response}.wgsl`, `src/viewer.rs`; regression coverage in `tests/{camera_inversion,gpu_film_response}.rs` and `src/gpu_tests.rs`.
