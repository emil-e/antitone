# Project state

Implemented in `src/project.rs` behind the `project` feature and connected to Editor through `src/editor_document.rs` and `src/editor_files.rs`. Explicit Save/Open is implemented; recovery, relink/source-change UI and undo remain separate work.

## Document and identity

- Bounded JSON: schema 5, 8 MiB budget, exact f64 round-trips and structural unknown-field rejection. Film shape objects reject unknown fields; the unit `Original` response deserializer ignores extra members.
- Stable `SourceId` allocator; source records are separate from ordered filmstrip membership. Removal from the filmstrip retains the record and never invalidates Dmin/Dmax.
- Each source stores path, development dimensions/active area/camera model/`camera_to_ap1` matrix/native-code floor, reference-only flag, photo settings, independent exact photo orientation and optional source-anchored crop. Relative paths resolve against the project document's directory.
- Roll identity stores backend version, fixed WB, development contract and declared inversion/working spaces. Its retained `camera_basis` records camera model and matrix; every source must match, even after all earlier source records are removed. Mixed native bases require separate rolls/calibration, not silently reused anchors. Output stores its transform contract, TIFF target and EXR precision.
- Dmin/Dmax each store only an optional native-camera RGB array; floor/development compatibility is roll-level. Uncorrected floor is `1/65535`; correction scales a runtime copy by the maximum gain including identity. Picker source/rectangle does not belong to a reference.
- Processing model 4 requires native-camera inversion and AP1/D60 working identity. Response is explicit (classic Original or Film); missing response rejects. New projects use Film toe 2.5/shoulder off. Only project schema 5 and processing model 4 are supported. Incompatible schema/model/development state rejects without migration or reinterpretation.

## Photo grade extension

Schema-5 Photo settings retain exposure/WB and add `contrast`, `pivot`, `contrast_colour_mix`, `saturation`, `saturation_density`. Missing new fields default to 1 / 0.18 / 0 / 1 / 0, so existing supported grades retain their exact appearance. New saves retain all fields with f64 round trips. The Colour preservation UI label still serializes as `contrast_colour_mix`; explicit saved blend values, including halfway, are not changed by new defaults. Source validation and runtime GPU preflight check their declared processing domains; invalid edits/Open/save fail atomically. Unknown fields remain rejected, so older readers cannot silently discard a new grade. No schema-4 migration or legacy processing path is introduced.

The fields are source-owned grading state, not UI preferences or derived cache data. Dirty checks, source switching, thumbnails/scopes/preview/refinement and export snapshots include the full Photo settings. [Exact operator contract](processing-model.md#photo-contrastpivot-and-saturation).

## Flat-field state

Schema 5 has an optional `flat_field`; an absent field still selects the uncorrected camera source path. Present state stores the explicit algorithm contract from `src/flat_field.rs`, Enabled, reference source ID, source development, roll development including fixed WB, accepted support and bounded RGB gain nodes (maximum 256×256). Deserialization validates node geometry/representability; document validation rejects unknown contracts, snapshot mismatches, incompatible enabled photo geometry/matrix/floor and invalid corrected calibration domains. The existing 8 MiB document limit remains.

The map is saved, not silently recomputed on Open: a missing nonordered reference file does not block use of its accepted map. Edit area reopens that reference and checks development compatibility. Choosing a separate reference registers it outside the filmstrip; an existing photo remains visible. Byte changes to the reference do not invalidate or silently rebuild the saved map; source/roll development and geometry compatibility still apply. Dmin/Dmax/base floor remain untouched; runtime calibration replaces a copy's base floor with the exact corrected bound once. Both roll and source base floors must equal `CAMERA_INPUT_FLOOR = 1/65535`; altered saved floors reject. Changing/enabling correction recommends re-picking calibration for the setup. Accepted map changes preserve an existing Enabled setting; first acceptance defaults to enabled.

## Geometry state

Schema 5 stores orientation separately from numerical grading: shared roll quarter-turns followed by horizontal/vertical flips, then per-source photo quarter-turns followed by displayed-axis flips. Missing orientation means identity; identity is omitted on write. Roll orientation uses flat `horizontal`/`vertical` fields and optional `turns` (missing = `zero`, zero omitted). Canonical turn variants and strict orientation objects reject invalid/unknown state.

Optional source `crop` stores source centre/pivot, positive integer frame extents in native units and optional source-plane basis `angle` in radians. Missing/zero angle selects exact integer-edge framing; zero is omitted. Nonzero angles allow fractional source centres, must be finite within −π…π, and require all four rotated frame corners inside the active area. Missing/omitted crop means full active source; applying a full-frame, zero-angle draft normalizes to omission. Document validation rejects crops outside recorded source coverage; loading rejects incompatible actual development.

Accepted geometry is dirty/saveable source-ID-owned state; viewport, source inspection and picker return context remain transient. New/Open/selection restore the appropriate orientation; newly imported photos inherit shared roll orientation, including turns, with identity photo orientation. Geometry reset leaves calibration and colour settings intact. Export holds composition and grade in one request snapshot and writes composed pixels/dimensions. Crop drafts/aspect preferences are transient: Save/Export use only accepted framing. Integer cropping and arbitrary-angle framing with coverage-aware reconstruction/measurement are implemented. See the [geometry contract](geometry.md).

## Validation and source changes

`validate()` checks structure, numerics, identity consistency and calibrated inversion. `require_runtime()` additionally checks current RAW/backend/space/output compatibility; Editor Open also checks runtime compatibility, stored GPU processing domains and ordered-source header/development identity before preparing the first photo with saved WB.

RAW contents are not hashed or treated as immutable. Changed bytes alone do not reject Open or development; files must remain loadable and match stored camera/development identity. Paths and stable IDs do not prove capture identity. Metadata identity, relinking and explicit source-change policy are not implemented. File stats belong only to optional thumbnail freshness, never persistent processing state. [Rationale](../decisions/no-raw-content-hashing.md).

Current contract identifiers are defined in `src/project.rs`:

```text
development: libraw-camera-ahd-u16-linear-camera-inversion-ap1-bradford-code-floor-v2
output: ocio-2.6.0-aces2-100nit-antitone-srgb-displayp3-v2
```

## Save/load guarantees and limits

`load()` bounds input and validates the resulting state. `save()` validates before writing a same-directory temporary file, syncs it, then persists with New/no-clobber or Replace semantics. Failure preserves an existing document; this is not a complete crash/power-loss durability claim.

The editor's source records own photo settings by stable ID, not path. New documents fix development WB on first successful source development; Open restores recorded WB. Imports capture headers without hashing or developing the whole filmstrip. Calibration sampling selects only ordered roll sources; separate bare-light flat-field references register outside the filmstrip.

File New/Open/Add RAWs/Save/Save As and Export TIFF/EXR half/full occupy the existing header row. `.antitone.json` is the default naming convention; Save accepts JSON filenames and compatibility is validated from content, not naming alone. Save As rebases relative locators against the new directory. Input RAWs cannot be project destinations, and Export cannot replace the currently open project.

Dirty state compares accepted persistent values with the last successful Save/Open snapshot. Numeric focus loss is processed before file actions; views, selection, rectangles and source bypass do not dirty a document. Dirty New/Open/Close asks Save/Discard/Cancel. Failed/cancelled Save prevents continuation; failed Open retains old state and display even after Discard. Monotonic request IDs reject stale results during document work.

Open selects the first ordered photo at Fit, or no scan for an empty filmstrip. Missing/unloadable/development-incompatible ordered RAWs reject Open with the path; unavailable nonordered records are retained without blocking Open or invalidating RGB calibration. Intentional byte edits need no separate acceptance. No automatic source substitution or migration occurs.

Separate recovery files, source-change UI and undo/redo remain in the [durable-project work package](../plans/durable-projects.md). Derived previews, scopes, thumbnails and decoded/GPU caches never belong in the saved document. Inspector expansion, scope visibility/sizing and preferred filmstrip height are application preferences saved independently on exit, not through project Save; failures remain project-neutral. [Preference contract](editor-controls.md#shared-visual-language). Planned recovery must never silently overwrite an explicit save.

Regression coverage: `tests/{project,film_project,flat_field_project}.rs` and `src/editor_{document,files,flat_field}_tests.rs`. Rationale: [RGB-only calibration](../decisions/rgb-only-calibration.md), [camera inversion](../decisions/native-camera-inversion.md).
