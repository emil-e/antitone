# Remaining editor state work

Scope ownership and source acquisition are implemented in the [architecture](docs/architecture.md). Two state-model follow-ups remain separate from those lifetime changes. The [editor guide](docs/editor.md) describes current interactions and accepted/draft behavior.

## Accepted state and editable controls

Document and photo scopes own their controls, but input, roll, crop and adjustment values still mirror accepted project data. Editor synchronization copies those values into the project; preparation and history restore controls from accepted snapshots.

Investigate one coherent accepted/editable model rather than moving the duplication again. Preserve explicit draft acceptance, precise numeric commits, chronological undo/redo, captured Save/Export inputs and failure-atomic preparation. Check authoritative values at each editing boundary, including calibration, layer selection, temporary reference visits and history restoration.

Acceptance should demonstrate that edits have one defined authority, drafts cannot leak into accepted captures, and navigation or failed preparation cannot overwrite newer edits. The implementation approach is open.

## Operations and activity presentation

Feature scopes own their pending work, while foreground activity, progress, cancellation feedback and `busy` still require separate controller bookkeeping.

Derive presentation and availability from actual operation state where practical. Preserve cancellation feedback through acknowledgement, stale-result rejection, captured inputs through physical cleanup and successful side effects committed before cancellation. Exercise owner retirement, request supersession, failed admission and completion without another paint.

Keep this change distinct from editing authority so each state model can be checked independently. No general operation registry or framework has been selected.

## Cache tuning

Further cache optimization follows measured workloads. The two state-model changes above do not require additional cache layers or speculative cross-document RAM retention.
