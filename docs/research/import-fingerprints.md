# Import and switching: historical hashing and removal measurements

Full RAW hashing is removed. [The accepted decision](../decisions/no-raw-content-hashing.md) rejects byte immutability and defers capture identity/relinking; project schema 5 has no fingerprints or older-project compatibility. File stats are optional thumbnail-cache freshness only. This record preserves measured costs before/after, not a requirement to restore hashing or overlap its work.

## Method and limits

Both timing harnesses use three sequential trials each for four and 36 consecutive supplied Sony sources, fresh documents/private temporary project/proxy caches, real backend preparation and four selections including revisits. The renderer is prewarmed once and then reused; foreground/background development remains 8/2 threads in the recorded normal-pressure runs. Processing, native inputs, RGB16, ACES and numerical gates are unchanged.

Thumbnail states are controlled: empty source/disk caches; retained source rendered in the other output target; lost source memory with a written disk proxy; and selected-source RGB16 seeding. Each thumbnail case starts/joins a worker. Cold deliberately omits selected seeding, unlike normal selected-thumbnail behavior. Memory/seed timing measures image construction, not an existing GPU-image cache hit.

Filesystem/page-cache state and other system activity are **uncontrolled**. No purge/uncached-read override, injected foreground GPU/refinement/scopes/prefetch load, forced pressure or original sample writes. Backend Ready/image construction is not painting/presentation or physical input response. The separately invoked byte-edit regression modifies only a temporary RAW copy. Separate runs are snapshots, not controlled universal speedup guarantees.

Environment: Apple M5 Pro / Mac17,9, 48 GiB, macOS 26.6.2 (25G83), Rust 1.99.0, Apple Clang, LibRaw 0.22.0, LLVM OpenMP 22.1.8, OCIO 2.6.0 and Metal/wgpu 30.0.1. These are observations, not minimum requirements.

## Historical full-read baseline

The original `97b7ad4` snapshot and ordinary-Cargo repeat on `6809ff5` are retained separately in [historical evidence](evidence/import-fingerprints.txt). The now-[archived test-only observer/harness](tools/import-fingerprints/README.md) counted successful complete-file BLAKE3 reads, including open/read/hash/finalization timing. Failed calls and uncaptured threads were excluded; production builds had no observer. Logical read bytes were not physical disk IO.

| Successful baseline lifecycle | Full-file fingerprint reads |
| --- | ---: |
| Fresh import/Open with N ordered sources | N + 2 |
| Selected source within import/Open | 3 (registration, selected pre/post-development) |
| Other ordered sources | 1 each |
| Later selection/revisit | 2 |
| Cold thumbnail | 2 |
| Disk-thumbnail reuse | 1 |
| Retained/selected-seed RGB16 | 0 additional |

At 36 sources, ordinary-Cargo baseline import/Open medians were 1.775/1.760 s, with 1.081/1.080 s hashing (about 61%). Four-source medians were 0.833/0.833 s with about 0.163 s hashing. Switching medians were 0.726/0.739 s for four/36 rolls, including approximately 58/59 ms hashing; native development remained dominant. Disk reuse was around 25.4–25.5 ms, about 98% hashing.

The baseline preserved full-byte identity because that was the implementation under measurement, not because the measurements justified the policy. The proposed registration/hash-overlap experiment was never selected and is obsolete after removal.

## Hash-free snapshot

Measured on the removal implementation over `d03e342`; subsequent integration of `87f223e` changed documentation only. Current `src/editor_import_tests.rs::import_switching_latency` uses the same backend/cache boundaries without the observer or RAW hashing. The latency probe was repeated **alone** after an initial combined run with the independent byte-edit regression; only the standalone run supplies this table. [Compact result lines](evidence/import-without-hashing.txt).

Median backend readiness / thumbnail image construction:

| Case | 4-source baseline | 4-source removal | 36-source baseline | 36-source removal |
| --- | ---: | ---: | ---: | ---: |
| Import | 0.833 s | 0.668 s | 1.775 s | 0.695 s |
| Project Open | 0.833 s | 0.672 s | 1.760 s | 0.675 s |
| Switching/revisits (12 selections pooled per roll) | 0.726 s | 0.665 s | 0.739 s | 0.669 s |
| Cold thumbnail | 0.970 s | 0.920 s | 0.974 s | 0.914 s |
| Disk thumbnail | 25.43 ms | 0.427 ms | 25.49 ms | 0.428 ms |
| Retained-source thumbnail | 0.160 ms | 0.223 ms | 0.179 ms | 0.260 ms |
| Selected-seed thumbnail | 0.174 ms | 0.155 ms | 0.154 ms | 0.168 ms |

The standalone probe passed in 29.88 s. Hash removal removes the measured registration/disk-verification work; it does not cache a complete selected Scan, so revisits still develop RAWs. Warm sub-millisecond differences are not evidence of a regression or guarantee. No higher-resolution switching tier or import concurrency is implemented.

## Reproduction and verification

Use ordinary Cargo with existing pinned prerequisites:

```sh
cargo test --locked --offline --release --features editor --lib import_switching_latency -- --ignored --nocapture
cargo test --locked --offline --release --features editor --lib raw_byte_edits_reopen_without_identity_rejection -- --ignored --nocapture
```

Source/project tests verify absent/rejected fingerprint fields, old-schema rejection, unchanged camera/basis/development checks and exact grade/geometry/map round-trips. Cache tests cover path/stat separation, missing stats, stale outcomes, checksummed disk restart and reference/selected-source isolation. The real-RAW test confirms intentional byte editing/reopen and exact RGB16 while preserving an old seed's original file key. [Implementation/checks](../plans/completed/remove-raw-hashing.md).

True filesystem-cold storage, physical scrolling/control latency, actual OS pressure, aggregate memory and broader camera/device/monitor acceptance remain unperformed. Metadata-based identity/relinking and higher-resolution switching caches remain deferred, not a next implementation queue.
