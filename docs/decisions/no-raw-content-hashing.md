# RAW loading and cache-only file revisions

Status: accepted.

## Choice and rationale

RAW files may be intentionally edited. Byte immutability is not photograph identity, and full-file hashing would add reads and reject those edits without a capture-identity requirement.

Use stable project source IDs and paths for ownership/loading, with stored camera/development compatibility. Processing model is 4; project schema 5 stores no RAW fingerprints in source or flat-field records. Only the current schema is supported, without migration or ignored compatibility fields.

Accepted flat-field maps remain saved snapshots associated with reference source ID and source/roll development. Byte edits alone do not invalidate the map or resample Dmin/Dmax; incompatible development/geometry still rejects.

Optional thumbnail freshness uses resolved/canonical paths and cheap file stats: length/mtime, plus device/inode/ctime on Unix. Capture the selected proxy key before opening/development; never tag existing pixels with later stats. Changed keys cause cache misses/stale-result rejection, not grade incompatibility. Unavailable stats disable disk reuse. The `thumbnails-v2` cache retains bounded pruning, atomic installation, small locator-key hashes and generated-RGB16 checksums, none of which hashes RAW contents.

## Limits

Paths, stable IDs and file stats do not prove capture identity or guarantee freshness if all stat fields are preserved. Files must be loadable and camera/development compatible, but changed bytes alone are accepted. Metadata identity and relinking policy remain undesigned; no automatic candidate selection or hash fallback is implied.

[Project contract](../contracts/project-state.md) · [Cache contract](../contracts/output.md#responsiveness-and-optional-caches) · [Checks](../build-and-probes.md#renderingimport-performance-probes)

Implementation: `src/{project,project_flat_field,editor_document,editor_thumbnails,thumbnail_disk}.rs`; regression coverage in project/cache tests and `src/editor_import_tests.rs`.
