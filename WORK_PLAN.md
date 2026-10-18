# Undo/redo

## Agreed scope

Session-only, whole-project chronological history using cloned `Project` snapshots, capped at 100 actions. Include import/removal and accepted Roll/Photo/Rendering, geometry, local adjustment, Dust, calibration, correction, Analyze and creative-look edits. Group continuous gestures. Photo undo/redo activates the affected photo without restoring old pan/zoom. Preserve history and written dirty baselines across Save/Save As; clear only on accepted New/Open. Drafts, navigation, preferences, pixels and caches are not history. No schema changes or inverse-operation/delta framework.

Follow-ups explicitly authorized: zero idle history Project clones; immutable mask-run sharing; rebase onto current main and integrate native menus. Integration is now explicitly requested: checkpoint and rebase as needed, verify, remove this plan after transferring lasting scope, commit and fast-forward local main, then safely remove only this task worktree and disposable artifacts. Do not push. Preserve unrelated main-checkout changes and other worktrees.

## Current implementation

- Rebased `feat/undo-redo` onto main `e64a1f3`, retaining task edits uncommitted. UI hooks use the extracted frame owner and preserve current-main repaired-output/recipe semantics.
- Idle control passes capture only small values. Accepted record replacements retain before-state lazily; scalar edits materialize one before/after pair. Tests count history snapshot clones on repair-bearing idle/navigation/draft/cancelled-input frames.
- `SupportMask` shares immutable `Arc<[[u32; 2]]>` runs. Replacement preserves older masks; JSON and value equality are unchanged. Logical retention charges the full shared payload conservatively to every owner.
- Undo/Redo use the shared native/fallback menu model with dynamic labels and focused-text routing. Held controls, popups, drafts and foreground work cannot dispatch project history. Queued native commands also check current input, not only previous-frame menu state.
- Native accelerators use logical character equivalents, including uppercase shifted letters required by AppKit. Native file-panel Undo/Redo remain responder-based rather than entering the editor queue.
- Accepted look selection/reset participates in history and shares the LUT allocation. Existing repaired TIFF/EXR/thumbnail behavior is preserved.

## Verification and remaining checks

Passing so far: 50 selected debug controller/composed/mask/menu/look checks; 32 release checks; 23 core Dust checks; 21 Dust/Look/Project persistence checks. Explicit AppKit checks pass for dynamic Undo/Redo labels, Cmd-Z/Shift-Cmd-Z, native-panel responder restoration, wakeup, minimization/restoration and guarded Quit. Final Clippy (warnings denied), production release build and contributor formatting checks pass. Three image-recipe identity checks and the authored reopened cold repaired-export/destination-safety check pass. Explicit AppKit checks also pass in both debug and release, including the logical shifted-key adapter. Latest logs are `.cache/final-*`, `.cache/rebased-recipes.*` and `.cache/rebased-repaired-exports.*`.

One review covered the original undo implementation, and a separate focused review covered the newly requested sharing/native-menu/rebase scope. Findings have reproducing tests and fixes; do not repeat reviews solely for those fixes.

Remaining acceptance: physical editing/file-panel interaction, long mixed sessions and aggregate memory/latency with large Dust histories. The bounded AppKit executable is not broad physical-input, accessibility or monitor acceptance. See `docs/plans/durable-projects.md` and topic guides. No full all-features suite or private-photograph acceptance was run for this rebased result.

## Recovery / next action

The pre-rebase task was preserved with a task-owned Git stash and checkout-local backup: `.cache/pre-main-rebase/stash-oid`, `task.tar`, `original/` and `tracked.patch`. Do not drop unrelated stashes. Native dependencies use the normal shared-cache link and documented `LIBRAW_SOURCE`/`OCIO_PREFIX` overrides.

Integration in progress. Main advanced to 0f1ed11: contributor/editor prose cleanup, development configuration and restored contract-focused tests. Renderer changes are formatting plus a test-only initialization observer; strengthened cold export and look-cache assertions should be rerun. Checkpoint the task, rebase, preserve the updated prose/tests, reconcile any stale undo-deferred wording, then run focused integrated checks. Existing independent reviews cover original undo and sharing/native-menu/rebase scopes; findings were accepted, fixed and verified, so do not repeat those reviews. Main's unrelated state is docs/feedback.md and .pi/settings.json; neither belongs to this delivery.
