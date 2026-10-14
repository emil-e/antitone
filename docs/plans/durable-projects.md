# Recovery, source changes and undo

[Projects](../projects.md) explains implemented Save/Open and compatibility. [The roadmap](../../PLAN.md) owns priority. Recovery, relinking/source-change UI and undo/redo remain unfinished.

## Scope and open choices

1. **Missing sources/relink:** define matching candidates and explicit acceptance from workflow needs. Do not assume byte immutability or invent capture identity. Retain source IDs and Photo settings; source changes/removal never resample calibration.
2. **Recovery/autosave:** use separate recovery state with explicit document identity and stale/corrupt handling. Preserve the last explicit save; exercise interrupted/failed writes before making durability claims.
3. **Undo/redo:** settle shared versus Photo scope and gesture coalescing. Include accepted geometry/correction changes; invalid picks create no entry, and drafts/navigation/caches stay transient.

No catalog, automatic schema migration, derived images in project JSON or silently overwritten explicit saves are implied.

## Acceptance and next action

Select and design one slice. Start with focused state tests, then exercise exact settings/output reopening, worker-active transitions, stale replies and the selected recovery/undo decisions. Legal values outside a slider's normal span must survive.

Likely owners are [document transitions](../../src/editor_document.rs), [file actions](../../src/editor_files.rs) and [project validation](../../src/project.rs). Use [development checks](../development.md) before a sample-dependent Save/Open exercise.
