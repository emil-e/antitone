# Dust repair: main integration

## Agreed outcome

Rebase the experimental dust workflow onto current main and deliver locally without disturbing unrelated work. Move the unchanged frozen healer to full-resolution ungraded signed/HDR AP1 after correction, calibrated inversion and camera conversion, before Roll/Photo photographic exposure/WB and all later grades. Source/development, correction and inversion changes may reevaluate asynchronously; downstream grading, targets and viewport changes must reuse valid repair content.

Preserve independent immutable-base repairs, exact known-pixel bits, signed/HDR values, bounded asynchronous evaluation and source-coordinate non-overlapping masks. Keep direct canvas selection, per-stroke release commit, Shift-add/Option-remove, donor dragging, Delete and contextual reset/Done. No per-repair Apply/Cancel layer.

Use main's physical WorkingView/underlay and purpose-bound OriginalView owners, not the obsolete settled-only lifecycle. Scopes should follow the same accepted repaired viewer result. Keep repairs transient and make unsaved/unexported limitations explicit. No solver tuning, persistence, automatic proposals, repaired exports/thumbnails or broad GPU migration. No additional push is authorized.

## Acceptance

- Unchanged repairs remain visible during navigation and subsequent cleanup, with correctly filtered source-coordinate placement in detail/underlay/original-reference views.
- Roll/Photo grade and target edits do not reevaluate healing; upstream invalidation rejects incompatible patches and late results. Cancellation cannot permanently suppress a valid unchanged repair.
- Deletion/reset/source switching cannot leave stale GPU patches or affect another source. Every repair uses immutable unrepaired neutral AP1.
- Main's fullscreen suspension, Analyze review, source inspection, roll-removal shortcut ownership, retained-source and immutable grade routing remain intact.
- Independent numerical/GPU, controller and composed UI checks cover the new stage and lifecycle. Preserve main's production/reference feature split and numerical gates.
- Documentation reflects current main and the new boundary; unresolved photographic/resource acceptance stays explicit.

## Remaining steps

1. Complete current-main verification of the port, including neutral-stage/corrected/signed-HDR cases, repair-bound preflight, controller stale/cancel/reset behavior and composed Dust/fullscreen/roll shortcut ownership.
2. Run independent review at the broad integration handoff, resolve necessary findings and inspect relevant current-main changes again.
3. Finalize owning guides/topic plan and transfer any remaining acceptance from this file, then remove it before delivery.
4. Commit and fast-forward main only when unrelated working-tree edits can be preserved safely. Audit private ignored research evidence before cleanup; do not discard unique artifacts.

## Constraints and blockers

- Main is b3a39f2 at start. Its checkout has unrelated uncommitted PLAN.md, docs/feedback.md, docs/plans/{README,completion,local-adjustments}.md and .pi/ work; do not overwrite, stash or commit it.
- The published branch checkpoint is ead1be0. Rebase rewrites only task history locally; leave the published remote untouched unless separately requested.
- Private samples remain read-only; all generated experiments/check outputs stay in this worktree's ignored .cache/.
- Hidden-pixel photographic accuracy, fine-wire reconstruction and aggregate/device/physical-input acceptance remain unestablished. Do not claim these from numerical tests.

## Next action

Finish the rebase checkpoint and run production/reference, GPU and composed checks. The port retains sparse original/repaired support records and view-sized neutral copies; repair deltas are filtered independently into those copies without reacquiring neutral detail/underlay. Original references grade absolute repaired neutral values. Preflight incorporates the repaired-value envelope without weakening existing gates. Remaining photographic and aggregate resource acceptance belongs in docs/plans/dust-repair.md.
