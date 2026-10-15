# Keep the retained source compact

The developed source is RGB16. Retaining it exactly in four u16 slots preserves its integer information without paying for wide arithmetic in every retained original.

Storage precision and arithmetic precision solve different problems. Near film base, nonlinear inversion makes small rounding differences significant. Shader temporaries and small constants therefore use additional precision where needed, without multiplying the retained source's memory cost.

That compact source choice does not prohibit reusing expensive processing. The kernel retains one bounded exact-code response table. The positive editor prototype also retains one physical-view filtered signed/HDR AP1 source, not a float copy of the whole original. Its useful latency saving depends on explicit upstream invalidation and immutable replacement lifetimes; complete concurrent native-memory admission remains acceptance work. Cheap calculations should not accumulate caches merely because storage is available. Preview AP1 is deliberately approximate and never becomes original/export authority; a future float source format must not be quantized into RGB16.

The architectural precedent is visible demand and upstream reuse, not cheap exact-export processing: [darktable 5.6.2](https://github.com/darktable-org/darktable/blob/release-5.6.2/src/develop/develop.c) requests visible ROI/display density, hashes preceding processing and can retain an old backbuffer while replacements run; [RawTherapee's coordinator](https://github.com/RawTherapee/RawTherapee/blob/b2ee504c47e5e6d5e054395aa12bf1fe43bd1c3c/rtengine/improccoordinator.cc) separates reduced preview/detail/export work. Their quality modes and algorithms differ. Neither establishes our preview/export correspondence or justifies lower precision, universal low-detail fallback or a cache after every stage.

Unsupported numerical domains reject rather than changing the source representation or falling back to CPU interactive processing. Rendered-only images need not retain unused source resources.

[GPU processing](../../src/gpu.rs) and [response tests](../../tests/gpu_film_response.rs) own the mechanics. [Editor previews](../editor.md#fast-feedback-is-not-authoritative-data) explain how those resources are used.
