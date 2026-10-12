# Remove full RAW hashing

Status: implemented, verified and delivered to local main over `87f223e`; no RAW hashing remains in production. Metadata identity/relinking and further performance features remain deferred. Approved scope: remove RAW hashing without adding capture identity or relinking, retain cache-only path/stat freshness, and reject old project schemas rather than build compatibility.

## Goal and boundaries

Remove full RAW-content fingerprints from import/Open, selected preparation, thumbnails, persisted sources and flat-field bookkeeping. Intentional RAW-byte edits are not grade incompatibility. [Accepted rationale/current choices](../../decisions/no-raw-content-hashing.md).

Keep camera/development/basis/WB checks, RGB-only calibration, accepted geometry and gain maps, signed/HDR data, unchanged ACES/LibRaw and numerical gates. Stable source IDs remain editor ownership. No metadata identity, relink/directory search, migration, new cache tier, native-processing changes or import scheduling redesign.

## Implementation

- Project schema 5, processing model 4; no source/map fingerprints or legacy parser. Removed full-file reader, comparisons, changed-byte errors and unused hash-based source-management helpers. `project` no longer enables BLAKE3.
- Selected preparation opens one RAW handle and preserves header/developed-source checks. Selected proxies carry the file key captured before opening/development; transient reference locators cannot seed the working photo.
- Thumbnail keys use resolved/canonical paths and ordinary length/mtime/device/inode/ctime stats, never capture metadata or byte identity. Stats are transient/cache-only; changed keys quietly miss/reject stale outcomes. `thumbnails-v2` retains generated-pixel checksums, atomic writes and 128 MiB pruning.
- Archived the old test-only full-read observer/harness with [historical measurements](../../research/import-fingerprints.md). Current `import_switching_latency` measures the same backend/cache boundaries without instrumentation or content hashing.

## Verification

[Retained results/method/limits](../../research/import-fingerprints.md) record the local M5 Pro/48 GiB/macOS 26.6.2/Rust 1.99.0 snapshot. Fresh checks after integrating main's contributor-documentation cleanup: 162 library tests (9 ignored), 19 project-family, 7 viewer, 9 scopes (1 ignored), 3 geometry-export, 5 camera-inversion and 2 full-f64 GPU-response tests passed (207 total). Project-only checks separately passed; strict editor/all-target Clippy, production release build and formatting passed. Delivery reran the 197 library/project/scope/viewer checks, Clippy, release build and formatting on the same integrated source; that narrower recheck is recorded separately in the evidence.

Ignored byte-edit/selected-revision and import/switching checks passed; the timing probe was repeated alone to exclude concurrent regression activity. Prewarm/scopes/seed/disk-restart and project-relative locator checks passed. Scripted Save/Open/thumbnail smoke passed with 626 visible frames and exact grade/custom-WB/working+sRGB+P3 reproduction plus failed Open preservation. These RAW/window checks preceded documentation-only main integration; no source/processing behavior changed in that integration. No fresh native serial/thread comparison was needed because no native source changed.

No native library/bridge, processing shader or sample was changed. The byte-edit regression modifies only a temporary RAW copy and compares exact RGB16 proxy pixels. Actual cold storage, physical input, OS-pressure/device/monitor and distribution acceptance remain separate.

## Remaining acceptance

No implementation remains in this package. Physical/resource acceptance and higher-resolution switching remain separately selected [completion work](../completion.md#rendering-and-thumbnail-performance). No import-overlap, metadata identity or relinking policy is queued.
