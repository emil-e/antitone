# Project state

Implemented in `src/project.rs` behind the `project` feature and connected to Editor through `src/editor_document.rs` and `src/editor_files.rs`. Explicit Save/Open is implemented; recovery, relink/source-change UI and undo remain separate work.

## Document and identity

- Bounded JSON: schema 3, 8 MiB budget, unknown fields rejected, exact f64 serialization round-trips.
- Stable `SourceId` allocator; source records are separate from ordered filmstrip membership. Removal from the filmstrip retains the record and never invalidates Dmin/Dmax.
- Each source stores path, full-byte BLAKE3 fingerprint, development dimensions/active area/camera matrix/floor, reference-only flag and photo settings. Relative paths resolve against the project document's directory.
- Roll identity stores backend version, fixed WB, development contract and declared inversion/working spaces. Output stores its transform contract, TIFF target and EXR precision.
- Dmin/Dmax each store only an optional RGB array; floor/development compatibility is roll-level. Picker source/rectangle/fingerprint does not belong to a reference.
- Model 2 requires Original response; model 3 requires Film response. New `Project` state uses the new-grade factory. Missing response in a model-2 schema-3 document retains Original; mismatches/old schemas are rejected, not migrated silently.

## Validation and source changes

`validate()` checks structure, numerics, identity consistency and calibrated inversion. `require_runtime()` additionally checks current RAW/backend/space/output compatibility; Editor Open also checks runtime compatibility, stored GPU processing domains and ordered-source fingerprints/header identity before preparing the first photo with saved WB.

`inspect_source()` distinguishes matching, missing and changed content. `relink_matching()` requires identical content and retains the source ID. `accept_changed_source()` is a separate explicit action that captures the new fingerprint/development record. Neither operation resamples calibration RGB.

Current contract identifiers are defined in `src/project.rs`:

```text
development: libraw-camera-ahd-u16-linear-ap0-bradford-row-sum-floor-v1
output: ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2
```

## Save/load guarantees and limits

`load()` bounds input and validates the resulting state. `save()` validates before writing a same-directory temporary file, syncs it, then persists with New/no-clobber or Replace semantics. Failure preserves an existing document; this is not a complete crash/power-loss durability claim.

The editor's source records own photo settings by stable ID, not path. New documents fix development WB on first successful source development; Open restores recorded WB. Imports capture headers/fingerprints without developing the whole filmstrip. Unregistered picker-only sources remain transient.

File New/Open/Add RAWs/Save/Save As and Export TIFF/EXR half/full occupy the existing header row. `.antitone.json` is the default naming convention; Save accepts JSON filenames and compatibility is validated from content, not naming alone. Save As rebases relative locators against the new directory. Input RAWs cannot be project destinations, and Export cannot replace the currently open project.

Dirty state compares accepted persistent values with the last successful Save/Open snapshot. Numeric focus loss is processed before file actions; views, selection, rectangles and source bypass do not dirty a document. Dirty New/Open/Close asks Save/Discard/Cancel. Failed/cancelled Save prevents continuation; failed Open retains old state and display even after Discard. Monotonic request IDs reject stale results during document work.

Open selects the first ordered photo at Fit, or no scan for an empty filmstrip. Missing/changed/incompatible ordered RAWs reject Open with the path; unavailable nonordered records are retained without blocking Open or invalidating RGB calibration. No source substitution, changed-byte acceptance or migration occurs.

Separate recovery files, source-change UI and undo/redo remain in the [durable-project work package](../plans/durable-projects.md). Derived previews, scopes, thumbnails and decoded/GPU caches never belong in the saved document. Planned recovery must never silently overwrite an explicit save.

Evidence: `tests/project.rs`, `tests/film_project.rs`, `src/editor_{document,files}_tests.rs` and the [Save/Open closure](../plans/completed/save-open.md). Rationale: [RGB-only calibration decision](../decisions/rgb-only-calibration.md), [versioned response decision](../decisions/versioned-film-response.md).
