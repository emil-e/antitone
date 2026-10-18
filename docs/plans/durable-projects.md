# Recovery, source changes and undo

[Projects](../projects.md) explains implemented Save/Open and compatibility; [editor](../editor.md#undo-and-redo) owns session-only project-wide undo/redo. [The roadmap](../../PLAN.md) owns priority. Recovery and relinking/source-change UI remain unfinished.

## Scope and open choices

1. **Missing sources/relink:** define matching candidates and explicit acceptance from workflow needs. Do not assume byte immutability or invent capture identity. Retain source IDs and Photo settings; source changes/removal never resample calibration.
2. **Recovery/autosave:** use separate recovery state with explicit document identity and stale/corrupt handling. Preserve the last explicit save; exercise interrupted/failed writes before making durability claims.
3. **Undo acceptance:** exercise physical pointer/keyboard and long mixed Roll/Photo sessions, including high-volume Dust history, source unavailability and corrected Save As. The current snapshot implementation favors simple code over minimal memory; retention/latency measurements can show whether deltas or broader structural sharing would help.

No catalog, automatic schema migration, derived images in project JSON or silently overwritten explicit saves are implied.

## Acceptance and next action

The recovery/source-change implementation design is open. Focused state tests can establish the selected behavior, with settings/output reopening, worker-active transitions, stale replies and recovery/source-change decisions exercising its interactions. Legal values outside a slider's normal span must survive.

Likely owners are [document transitions](../../src/editor_document.rs), [file actions](../../src/editor_files.rs) and [project validation](../../src/project.rs). Use [development checks](../development.md) before a sample-dependent Save/Open exercise.
