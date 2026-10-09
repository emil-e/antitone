# Project-state foundation

Implemented in `src/project.rs` behind the `project` feature; **not connected to Editor Save/Open**. The live editor remains unsaved and path-keyed. This describes existing primitives, not a completed recovery/undo workflow.

## Document and identity

- Bounded JSON: schema 3, 8 MiB budget, unknown fields rejected, exact f64 serialization round-trips.
- Stable `SourceId` allocator; source records are separate from ordered filmstrip membership. Removal from the filmstrip retains the record and never invalidates Dmin/Dmax.
- Each source stores path, full-byte BLAKE3 fingerprint, development dimensions/active area/camera matrix/floor, reference-only flag and photo settings. Relative paths resolve against the project document's directory.
- Roll identity stores backend version, fixed WB, development contract and declared inversion/working spaces. Output stores its transform contract, TIFF target and EXR precision.
- Dmin/Dmax each store only an optional RGB array; floor/development compatibility is roll-level. Picker source/rectangle/fingerprint does not belong to a reference.
- Model 2 requires Original response; model 3 requires Film response. New `Project` state uses the new-grade factory. Missing response in a model-2 schema-3 document retains Original; mismatches/old schemas are rejected, not migrated silently.

## Validation and source changes

`validate()` checks structure, numerics, identity consistency and calibrated inversion. `require_runtime()` additionally checks current RAW/backend/space/output compatibility; a future editor load must not stop at JSON parse/structural validation.

`inspect_source()` distinguishes matching, missing and changed content. `relink_matching()` requires identical content and retains the source ID. `accept_changed_source()` is a separate explicit action that captures the new fingerprint/development record. Neither operation resamples calibration RGB.

Current contract identifiers are defined in `src/project.rs`:

```text
development: libraw-camera-ahd-u16-linear-ap0-bradford-row-sum-floor-v1
output: ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2
```

## Save/load guarantees and limits

`load()` bounds input and validates the resulting state. `save()` validates before writing a same-directory temporary file, syncs it, then persists with New/no-clobber or Replace semantics. Failure preserves an existing document; this is not a complete crash/power-loss durability claim.

Separate recovery files, saveable dirty state, stable-ID editor ownership, source-change UI, undo/redo and document-transition behavior remain in the [durable-project work package](../plans/durable-projects.md). Derived previews, scopes, thumbnails and decoded/GPU caches never belong in the saved document. Planned recovery must never silently overwrite an explicit save.

Evidence: `tests/project.rs`, `tests/film_project.rs`. Rationale: [RGB-only calibration decision](../decisions/rgb-only-calibration.md), [versioned response decision](../decisions/versioned-film-response.md).
