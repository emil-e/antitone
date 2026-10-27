# Unified processing planning and execution

**Status: deferred follow-up.** Integrate the current Film Emulation work first; this plan records the architectural direction without adding it to that implementation scope.

## Current boundary

The [ordered pixel pipeline](src/pixel_pipeline.rs) composes stage support, and its [GPU executor](src/gpu/pixel_pipeline.rs) owns post-grade dispatch, scratch, grain-field reuse and valid-output extraction. View, export and original-refinement adapters separately query that support and arrange acquisition, repair relevance, tiling and output reconstruction. This shares the neighbourhood calculation, not the complete processing plan.

## Direction

Unify planning and the execution contract while retaining source-specific I/O and consumer-specific scheduling/output:

```text
requested output + source geometry + processing stages
                         ↓
                  processing plan
                         ↓
            executor + source adapter
                         ↓
                    valid output
```

The planner would determine required input, per-stage regions, boundary handling and output mapping. The executor would own sequencing, tiled work, intermediate lifetimes and reuse against that plan. Source adapters would supply the requested samples from originals or proxies without knowing which effect requested them. Consumers would choose output geometry, target and work budget rather than independently propagating neighbourhoods.

Keep stage inputs limited to pixels, generic geometry and their own parameters. Adding a neighbourhood-dependent effect should change its implementation and pipeline assembly, not support-handling code throughout the consumers. A small concrete planner/executor is the starting point; a general graph framework, stage trait or plugin system is not a decided design.

## Checks to carry forward

Protect viewport/crop/tile boundaries, actual-source-edge extension, cropped-out repair influence and cache compatibility when support changes. Preserve source coordinates, seed and scale, signed/HDR values, stage order and authoritative export. Keep the physical-view sampling approximation distinct from original-sample processing.

Heavier presets, fine-grain cold-generation optimization and the halation operator remain separate follow-ups.
