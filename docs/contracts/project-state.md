# Project state

Implemented in `src/project.rs` behind the `project` feature and connected to Editor through `src/editor_document.rs` and `src/editor_files.rs`. Explicit Save/Open is implemented; recovery, relink/source-change UI and undo remain separate work.

## Document and identity

- Bounded JSON: schema 4, 8 MiB budget, exact f64 round-trips and structural unknown-field rejection. Film shape objects reject unknown fields; the existing unit `Original` response deserializer ignores extra members (a pre-existing limitation, not a migration path).
- Stable `SourceId` allocator; source records are separate from ordered filmstrip membership. Removal from the filmstrip retains the record and never invalidates Dmin/Dmax.
- Each source stores path, full-byte BLAKE3 fingerprint, development dimensions/active area/camera model/`camera_to_ap1` matrix/native-code floor, reference-only flag, photo settings, independent exact photo orientation and optional source-anchored crop. Relative paths resolve against the project document's directory.
- Roll identity stores backend version, fixed WB, development contract and declared inversion/working spaces. Its retained `camera_basis` records camera model and matrix; every source must match, even after all earlier source records are removed. Mixed native bases require separate rolls/calibration, not silently reused anchors. Output stores its transform contract, TIFF target and EXR precision.
- Dmin/Dmax each store only an optional native-camera RGB array; floor/development compatibility is roll-level. Uncorrected floor is `1/65535`; correction scales a runtime copy by the maximum gain including identity. Picker source/rectangle/fingerprint does not belong to a reference.
- Processing model 4 requires native-camera inversion and AP1/D60 working identity. Response is explicit (classic Original or Film); missing response rejects. New projects use Film toe 2.5/shoulder off. Schemas/models 2/3 and AP0 development identities are incompatible; no migration or reinterpretation.

## Flat-field state

Schema 4 has an optional `flat_field`; an absent field still selects the uncorrected camera source path. Present state stores the explicit algorithm contract from `src/flat_field.rs`, Enabled, reference source ID/fingerprint, source development, roll development including fixed WB, accepted support and bounded RGB gain nodes (maximum 256×256). Deserialization validates node geometry/representability; document validation rejects unknown contracts, snapshot mismatches, incompatible enabled photo geometry/matrix/floor and invalid corrected calibration domains. The existing 8 MiB document limit remains.

The map is saved, not silently recomputed on Open: a missing nonordered reference file does not block use of its accepted map. Edit area reopens/verifies that reference. Choosing a separate reference registers it outside the filmstrip; an existing photo remains visible. Replacing/changing accepted reference content requires rebuilding/clearing its map, not silently updating identity. Dmin/Dmax/base floor remain untouched; runtime calibration replaces a copy's base floor with the exact corrected bound once. Both roll and source base floors must equal `CAMERA_INPUT_FLOOR = 1/65535`; altered saved floors reject. Changing/enabling correction recommends re-picking calibration for the setup. Accepted map changes preserve an existing Enabled setting; first acceptance defaults to enabled.

## Geometry state

Schema 4 permits optional `orientation` fields separately from numerical grading: shared roll quarter-turns followed by horizontal/vertical flips, then per-source photo quarter-turns followed by displayed-axis flips. Missing fields mean identity; identity fields are omitted on write, preserving supported pre-geometry native-camera documents. Geometry adds no model-2/model-3 compatibility or migration. Older applications reject nonidentity orientation fields rather than ignoring them. Roll orientation retains the legacy flat `horizontal`/`vertical` fields and adds optional `turns` (missing = `zero`, zero omitted). Existing flip-only objects therefore load and serialize unchanged; nonzero roll turns are rejected by older applications instead of silently ignored. Per-photo serialization remains unchanged. Canonical turn variants and strict orientation objects reject invalid/unknown state. Development/output contract identifiers and numerical processing models are unchanged.

Optional source `crop` stores source centre/pivot, positive integer frame extents in native units and optional source-plane basis `angle` in radians. Missing/zero angle preserves the old integer-edge representation exactly; zero is omitted. Nonzero angles allow fractional source centres, must be finite within −π…π, and require all four rotated frame corners inside the active area. Missing/omitted crop means full active source; applying a full-frame, zero-angle draft normalizes to omission. Older crop readers reject nonzero angle fields rather than silently misrendering them. Changed-source acceptance rejects incompatible actual crop coverage before mutating the record.

Accepted geometry is dirty/saveable source-ID-owned state; viewport, source inspection and picker return context remain transient. New/Open/selection restore the appropriate orientation; newly imported photos inherit shared roll orientation, including turns, with identity photo orientation. Geometry reset leaves calibration and colour settings intact. Export holds composition and grade in one request snapshot and writes composed pixels/dimensions. Crop drafts/aspect preferences are transient: Save/Export use only accepted framing. Integer cropping and arbitrary-angle framing with coverage-aware reconstruction/measurement are implemented. See the [geometry contract](geometry.md).

## Validation and source changes

`validate()` checks structure, numerics, identity consistency and calibrated inversion. `require_runtime()` additionally checks current RAW/backend/space/output compatibility; Editor Open also checks runtime compatibility, stored GPU processing domains and ordered-source fingerprints/header identity before preparing the first photo with saved WB.

`inspect_source()` distinguishes matching, missing and changed content. `relink_matching()` requires identical content and retains the source ID. `accept_changed_source()` is a separate explicit action that captures the new fingerprint/development record. Neither operation resamples calibration RGB.

Current contract identifiers are defined in `src/project.rs`:

```text
development: libraw-camera-ahd-u16-linear-camera-inversion-ap1-bradford-code-floor-v2
output: ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2
```

## Save/load guarantees and limits

`load()` bounds input and validates the resulting state. `save()` validates before writing a same-directory temporary file, syncs it, then persists with New/no-clobber or Replace semantics. Failure preserves an existing document; this is not a complete crash/power-loss durability claim.

The editor's source records own photo settings by stable ID, not path. New documents fix development WB on first successful source development; Open restores recorded WB. Imports capture headers/fingerprints without developing the whole filmstrip. Unregistered picker-only sources remain transient.

File New/Open/Add RAWs/Save/Save As and Export TIFF/EXR half/full occupy the existing header row. `.antitone.json` is the default naming convention; Save accepts JSON filenames and compatibility is validated from content, not naming alone. Save As rebases relative locators against the new directory. Input RAWs cannot be project destinations, and Export cannot replace the currently open project.

Dirty state compares accepted persistent values with the last successful Save/Open snapshot. Numeric focus loss is processed before file actions; views, selection, rectangles and source bypass do not dirty a document. Dirty New/Open/Close asks Save/Discard/Cancel. Failed/cancelled Save prevents continuation; failed Open retains old state and display even after Discard. Monotonic request IDs reject stale results during document work.

Open selects the first ordered photo at Fit, or no scan for an empty filmstrip. Missing/changed/incompatible ordered RAWs reject Open with the path; unavailable nonordered records are retained without blocking Open or invalidating RGB calibration. No source substitution, changed-byte acceptance or migration occurs.

Separate recovery files, source-change UI and undo/redo remain in the [durable-project work package](../plans/durable-projects.md). Derived previews, scopes, thumbnails and decoded/GPU caches never belong in the saved document. Planned recovery must never silently overwrite an explicit save.

Evidence: `tests/{project,film_project,flat_field_project}.rs`, `src/editor_{document,files,flat_field}_tests.rs` and the [Save/Open closure](../plans/completed/save-open.md). Rationale: [RGB-only calibration decision](../decisions/rgb-only-calibration.md), [camera inversion decision](../decisions/native-camera-inversion.md).
