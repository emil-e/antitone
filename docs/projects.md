# Projects, identity and saved state

A project is a saved description of an accepted grade, not a copy of the RAW photographs or a cache of rendered pixels. Reopening it should restore the same processing choices without quietly estimating them again.

## Separate sources from the filmstrip

Each registered source has a stable ID and owns its Photo settings and composition. Filmstrip order is a separate list of those IDs. Removing a photo from that list does not erase its record or invalidate a stored calibration value. Even an empty roll retains its camera basis, calibration, correction and Roll settings. Adding a previously removed compatible RAW restores its existing source record and Photo settings; [selection/removal](editor.md#select-and-remove-roll-photos) is controlled by the editor.

A bare-light reference can be registered without appearing as a photograph. Dmin/Dmax themselves are RGB values: their continued use does not depend on reopening the photo from which they were picked. A saved flat-field map similarly remains usable without its nonordered reference; editing it is a different operation.

Paths locate external files. Relative paths resolve beside the project document, and Save As rebases them for its new location. A path or stable ID is not proof of photographic capture identity.

## Save accepted choices, not working machinery

The project retains calibration, Roll and Photo grading, source/development identity, orientation/crop, accepted correction, source-owned Analyze support with shared/exception provenance and compact retained frame evidence (and older shared explicit picture geometry) and output settings. These are the choices that determine its processing.

It does not retain active/multi-selection, canvas navigation, source-inspection exposure, picker return context, tool drafts, GPU objects or caches. Inspector expansion and scope/filmstrip sizing are application preferences saved independently; they are not part of a photographic grade.

This distinction defines dirty state. Accepted project changes are compared with the last successful Save/Open snapshot. Looking around, selecting a photo or cancelling a draft does not make a project modified. Analyze review areas remain drafts until a successful analysis installs them with the grade. Cancelling or failing the operation saves neither. Applying an older shared explicit picture area is still an accepted geometry change without an immediate reanalysis.

## Source-owned local adjustments

The schema-5 library extension optionally stores an ordered collection of source-anchored local adjustments. Stable IDs and a retained allocator support renaming/reordering without changing identity; deletion does not reuse IDs. Strict contract/record validation rejects unknown fields, malformed geometry and unsupported processing instead of silently dropping a grade. Older strict readers reject the extension rather than displaying a global-only result.

Immutable processing snapshots contain Global settings and ordered local values, not names, IDs or allocator history. Metadata still participates in saved/dirty state, but does not invalidate GPU processing. Editing-target selection, handles, overlays and mask drafts remain transient. [Local feature admission](plans/local-adjustments.md) is unfinished: normal editor Open/Save/source preparation/export currently rejects nonempty collections, including disabled records.

## Compatibility is about the processing basis

Calibration makes sense only in the camera basis and development in which it was defined. A roll therefore retains its camera model and camera-to-AP1 matrix, along with fixed development WB and backend/development identity. Every source and reference must be compatible; removing old records does not reset the retained basis.

The supported format is **schema 5 with processing model 4**. Unsupported schemas, old AP0 grades and incompatible development are rejected, not migrated or reinterpreted. Missing supported optional grade fields preserve existing appearance through explicit defaults; unknown grade fields reject rather than being silently discarded. Open rejects an incompatible saved output-transform identity and preflights the saved processing against current GPU domains. Exact fields and identifiers belong to the deserializers and tests.

RAW contents are not hashed. Changing bytes alone does not invalidate a grade if the source remains loadable and development-compatible. Cheap file revisions tell disposable caches when to refresh; they do not establish capture identity or photographic equivalence. [The identity decision](decisions/no-raw-content-hashing.md) explains why those questions remain separate.

Missing or incompatible **ordered** sources reject Open. Unavailable nonordered records remain preserved without blocking it. Recovery, relinking/source-change UI and undo/redo are not implemented.

## Prepare before replacing

Open validates the candidate, checks sources and prepares the first photo before replacing the live document. Failure retains the previous document and display—even if the user had agreed to discard its unsaved changes.

New/Open/Close use Save/Discard/Cancel protection. Pending numeric edits are processed before file actions. A cancelled or failed Save prevents the requested transition, and stale worker replies cannot complete a different document's operation.

Save validates before writing a same-directory temporary and installing it with no-clobber or explicit replacement semantics. The saved baseline changes only after success. This protects an existing file from ordinary operation failures, not every crash or power-loss scenario.

## Working on this area

[Project types](../src/project.rs) own format, validation and compatibility identifiers. [Document](../src/editor_document.rs) and [file actions](../src/editor_files.rs) own staged editor transitions. [Preferences](../src/editor_preferences.rs) own independent UI persistence.

Exact serialization details belong to those types and [project tests](../tests/project.rs), [camera compatibility tests](../tests/camera_project.rs), [film tests](../tests/film_project.rs) and [correction tests](../tests/flat_field_project.rs). Use [development checks](development.md#feature-specific-checks); see the [unfinished persistence work](plans/durable-projects.md) before proposing recovery or source-identity changes.
