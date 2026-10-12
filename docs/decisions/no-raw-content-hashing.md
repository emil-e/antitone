# Do not hash RAW files as photograph identity

Status: accepted. Supersedes the photo-fingerprint clauses of [RGB-only calibration](rgb-only-calibration.md), the schema-4 storage choice of [native-camera inversion](native-camera-inversion.md), and the original full-byte thumbnail-verification policy. Native-camera processing model 4 and numerical/development/output contracts are unchanged.

## Context

Whole-file BLAKE3 checks enforced byte immutability, not photograph identity. They rejected intentional RAW edits, read each selected file repeatedly and dominated normal-roll registration and disk-thumbnail reuse in [measured local cases](../research/import-fingerprints.md). The user did not establish byte-change protection as a requirement. Relinking is not implemented, so choosing a strict capture-metadata identity or automatic candidate policy now would be premature.

## Decision

Remove RAW content fingerprints, hashing, changed-byte rejection and unused hash-based inspect/relink/explicit-change primitives. Keep stable project source IDs and stored paths for current ownership/loading, and preserve camera/development compatibility independently.

Use project schema 5 with no fingerprints in source or flat-field records. The user explicitly declined compatibility: schemas 1–4 reject, with no migration, ignored legacy fields or fabricated hashes. Processing model remains 4.

Keep accepted flat-field maps as saved snapshots associated with reference source ID and source/roll development. A byte edit alone does not invalidate a map or resample Dmin/Dmax. Header/development incompatibility still rejects.

Use resolved/canonical file paths and cheap file stats solely for optional thumbnail-cache freshness: length/mtime, plus device/inode/ctime on Unix. Capture the selected proxy's file key before opening/development; do not tag old pixels with later file stats. A changed key causes a cache miss/stale-result rejection, not a grade incompatibility. Unavailable stats disable disk reuse. Start a new `thumbnails-v2` namespace rather than interpret old hash-keyed entries. Retain atomic installation, bounded pruning, serialized-key locator hashing and generated-RGB16 pixel checksums; none hashes RAW contents.

## Consequences

Normal byte edits are accepted when the file remains loadable and camera/development compatible. The app does not prove that a path still denotes the same capture, distinguish reused card filenames, or guarantee freshness when all stat fields are preserved. No metadata identity, relink UI/directory scan, new preview tier or import concurrency is introduced. Those choices await actual workflow needs.

## Evidence

- [Project contract](../contracts/project-state.md), [optional-cache contract](../contracts/output.md#responsiveness-and-optional-caches).
- `src/{project,project_flat_field,editor_document,editor_thumbnails,thumbnail_disk}.rs`.
- Project/cache/selected-reference tests and the ignored real-RAW edit/reopen and [import/switching probe](../build-and-probes.md#renderingimport-performance-probes).
- [Removal scope/checks](../plans/completed/remove-raw-hashing.md), [historical capture metadata inventory](../research/capture-metadata.md).
