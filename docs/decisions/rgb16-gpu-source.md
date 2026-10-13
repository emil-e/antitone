# Keep the retained source compact

The developed source is RGB16. Retaining it exactly in four u16 slots preserves its integer information without a wide converted-image cache.

Storage precision and arithmetic precision solve different problems. Near film base, nonlinear inversion makes small rounding differences significant. Shader temporaries and small constants therefore use additional precision where needed, without multiplying the retained source's memory cost.

Unsupported numerical domains reject rather than changing the source representation or falling back to CPU interactive processing. Rendered-only images need not retain unused source resources.

[GPU processing](../../src/gpu.rs) and [response tests](../../tests/gpu_film_response.rs) own the mechanics. [Editor previews](../editor.md#fast-feedback-is-not-authoritative-data) explain how those resources are used.
