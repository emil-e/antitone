# Use established RAW development

A roll's calibration must refer to a consistent developed source. Automatic brightness or per-photo WB substitution would change that basis beneath shared settings.

We use pinned LibRaw development with one fixed roll WB and established AHD demosaicing, retrieving linear camera RGB before standard-space output clipping. Reusing a mature developer avoids the uncertainty and maintenance cost of a custom demosaicer or camera-specific repair.

Native parallelism improves development time, but RAW work must leave room for interaction. Scoped OpenMP budgets preserve pressure reductions without changing pixels or leaking task settings. The pinned source-built runtime makes this reproducible rather than dependent on a system installation.

The editor now replaces concurrent foreground/background thread-budget splitting with one high-resource priority queue. Whole-roll readiness motivates using the established high budget for every development and joining compatible in-flight work rather than developing the same source again. Normal/Warning/Critical budgets are 8/2/1, additionally capped by available CPUs minus two (minimum one); high-resource does not mean all-core. Foreground, visible-thumbnail and eager priorities act between non-preemptible native calls. An unrelated cold selection may therefore wait for an active call. Bounded comparisons support faster whole-roll thumbnail completion at the cost of unrelated cold-selection wait, not a universal throughput/latency optimum. [Scheduling diagnostics](../native-development.md#editor-scheduling) describe those limits; [broader real-use/resource acceptance](../plans/visual-feedback.md) remains separate. Standalone RAW APIs/probes retain their explicit priority budgets.

Original capture diagnostics remain separate from the developed image. Regional checks can still detect sensor clipping after development; whole-frame statistics are computed only when explicitly requested.

See [the bridge](../../native/raw_bridge.cpp), [RAW ownership](../../src/raw.rs) and [native diagnostic](../native-development.md). Runtime inspection is not distributable packaging or broader-camera acceptance.
