# Use established RAW development

A roll's calibration must refer to a consistent developed source. Automatic brightness or per-photo WB substitution would change that basis beneath shared settings.

We use pinned LibRaw development with one fixed roll WB and established AHD demosaicing, retrieving linear camera RGB before standard-space output clipping. Reusing a mature developer avoids the uncertainty and maintenance cost of a custom demosaicer or camera-specific repair.

Native parallelism improves development time, but background work must leave room for foreground interaction. Scoped OpenMP budgets follow priority and pressure without changing pixels or leaking task settings. The pinned source-built runtime makes this reproducible rather than dependent on a system installation.

Original capture diagnostics remain separate from the developed image. Regional checks can still detect sensor clipping after development; whole-frame statistics are computed only when explicitly requested.

See [the bridge](../../native/raw_bridge.cpp), [RAW ownership](../../src/raw.rs) and [native diagnostic](../native-development.md). Runtime inspection is not distributable packaging or broader-camera acceptance.
