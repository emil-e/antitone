# Responsive rendering and thumbnail caching — completed

Status: implemented and integrated with native-camera processing, accepted geometry and responsive scopes. The user reported improved responsiveness from the earlier performance release build. Broader real-use/resource acceptance remains separate [follow-up](../completion.md#rendering-and-thumbnail-performance).

## Scope and result

- Shared/prewarmed immutable GPU processing, both ACES targets, presentation and averaging resources; mutable source/output storage, revisions and preparation signatures remain image/job-owned.
- Rectangular averaging dispatches only conservative tile/output overlap with unchanged weights/order; affine reconstruction retains actual coverage and full-output dispatch.
- Exact verified selected-Scan RGB16 proxies seed thumbnails. Pressure-responsive byte budgets protect visible/current work, retain recent entries and cancel obsolete phases; idle prefetch uses spare capacity without evicting retained work.
- Versioned/checksummed atomic disk proxies retain full-byte RAW/development verification and bounded storage. Corrupt optional cache entries are misses; changed/missing/incompatible RAWs remain errors.
- Pinned local OpenMP accelerates unchanged LibRaw development with scoped/restored bounded CPU policy. Decoding is separate from optional whole-sensor diagnostics; regional picks retain immutable original sensor data.
- Scopes keep independent whole-composition coverage, bins/readback identity and lifecycle. Viewer navigation, thumbnails and regional refinement never substitute for scope input.

Current policy and boundaries: [output/cache contract](../../contracts/output.md#responsiveness-and-optional-caches), [architecture](../../architecture.md#preview-resources-and-caches), [native decision](../../decisions/parallel-libraw.md) and [native probe](../../research/tools/raw-development.md).

## Verification and limits

[Performance evidence](../../research/evidence/rendering-performance.txt) records cache/identity/pressure/cancellation tests, warm-roll scrolling, shared-renderer scope ownership, corrected affine reconstruction, real-train and native comparisons, Save/Open and visible presentation checks. Native RGB16/metadata/full-regional diagnostics matched the serial baseline on the recorded sources/thread counts; this does not establish all-camera equality.

Refinement depth remains **1**: measured deeper queues were bit-identical and gained about 6% idle throughput, but worsened simulated foreground p95 from about 36 ms to 51–52 ms. Other import/prewarm/disk timings are individual local snapshots, not universal speedup claims.

Actual OS-pressure exercise, physical scrolling/gestures, broader camera/device resources, photographic scope appearance, physical monitors, actual macOS-14 execution and clean-machine distribution remain unverified. Higher-resolution switching caches and import fingerprint optimization were assessed, not implemented. No whole-roll developed RAW cache, approximate ACES, new processing schema or CPU interactive fallback was introduced.

[Archived implementation record](../../research/history/implementation/rendering-performance.txt) preserves checkpoint/integration history. [Completion work](../completion.md#rendering-and-thumbnail-performance) owns remaining measurements and acceptance; [release work](../completion.md#export-and-release-work) owns native bundling/licensing/signing and clean-machine checks.
