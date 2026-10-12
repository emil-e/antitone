# Archived full-RAW-hash measurement harness

Historical test-only source for the [import/switching fingerprint measurements](../../import-fingerprints.md). Not compiled by current Cargo targets, not a supported hashing/relinking path, and not an active implementation queue. Current code has no `Fingerprint` type/full-file reader; use [the current readiness probe](../../../build-and-probes.md#renderingimport-performance-probes) instead.

The retained modules ran against commit `6809ff5` (ordinary-Cargo repeat; original snapshot `97b7ad4`). They require that baseline's editor/project/cache APIs and LibRaw/Metal prerequisites:

1. Include `fingerprint_measurements.rs` as `project::fingerprint_measurements` only under `cfg(all(test, feature = "editor"))`.
2. In the baseline `Fingerprint::read`, call `fingerprint_measurements::start()` before opening the file, and `finish(path, bytes, started)` only after successfully constructing the complete fingerprint; both calls use the same test/editor cfg. The production reader remains otherwise unchanged.
3. Include `editor_fingerprint_tests.rs` as a nested module of `editor_performance_tests.rs`, which supplies the thumbnail-worker/editor imports.
4. On that baseline, run `cargo test --locked --offline --release --features editor --lib import_switching_fingerprint_latency -- --ignored --nocapture`.

This is reconstruction context for historical evidence, not an alternative setup or supported RAW-hashing implementation. Generated/private outputs stay local. The small generated-thumbnail checksums retained in current production code are unrelated to the archived full-file observer.
