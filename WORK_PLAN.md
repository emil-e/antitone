# Adjustment layers

## Agreed scope and acceptance
- One bottom-to-top stack with global, linear and radial coverage and identical photographic adjustment semantics.
- One ordinary global layer initially; movable, renameable, disableable and deletable. Empty stacks are valid. Insert above selection.
- Persistent Layers section and one shared control set with a clear selected target. No coverage-conversion UI initially.
- Input interpretation and Roll stay upstream. Use AdjustmentLayer terminology. No legacy-project migration.
- Follow-up explicitly authorized: behave as if there had never been a privileged global adjustment, including execution, admission and picking—not merely saved/editor state.

## Integration delivery
The user explicitly requested integration: checkpoint agreed work, reconcile against the fetched GitHub main, verify/review the integrated result, push the task branch, open/update a PR and watch CI. The user merges on GitHub; do not push main or remove the active task worktree before integration and artifact inspection.

Fetched main is `9c63d1e` (Windows support #62), following `5c888be` (single response #64). Reconciliation must preserve the response-only model and Windows `sys`/GPU/fixture boundaries, and advance the project schema beyond main's 10. Initial implementation and retirement reviews are complete; any further independent review should target new integration/platform interactions and unreviewed clean-view changes, not repeat those reviews.

Remaining: checkpoint, rebase and resolve semantic interactions; run integrated checks including local Metal; inspect/review new integration changes; remove this session plan after lasting guidance is in topic guides; push/open PR and watch CI. Preserve the unrelated main-checkout edits (`src/editor_analysis_private_review_tests.rs`, `ISSUE_TRIAGE_DRAFTS.md`).

## UI feedback
The explicit copy/disabled-state requests are implemented: no Layers order explanation or disabled-layer no-effect message; disabled layers grey out photographic and coverage controls while their enable toggle remains usable. At zero strength, ineffective controls are greyed out but Strength remains usable to restore the layer. Pending numeric text commits before disabling a layer. The 12 composed layer UI cases pass in debug and release, including normal/narrow disable/re-enable and zero-strength recovery; production release build, all-feature/all-target Clippy, formatting and diff whitespace checks pass. Logs: `.cache/layer-ui-feedback-*.log`.

The accepted clean-view changes are complete: the selected name is in the Layers header, including collapsed state; the separate pinned label is removed. Mask coverage, guides and handles temporarily hide when collapsed or adjusting sliders, numeric fields or colour wheels, without changing the coverage preference. Hidden handles do not intercept navigation. Active gradient creation/geometry gestures retain visible feedback. Header identity survives selected-name changes; truncated titles retain their complete accessible name and hover text. “Zero-strength recovery” only means Strength remains adjustable at zero, not a separate mode.

Verification: 171 broad UI/component/workflow cases passed with only the known baseline correction-widget failure; 84 relevant release UI/component cases passed; final focused layer/header/activity checks passed (17). Shared activity tests cover held-but-unchanged sliders and hidden/disabled controls; composed tests cover both gradient kinds, numeric entry, wheels, collapsed navigation, preserved coverage preference and drawing through collapse. All five guide captures were regenerated from the existing read-only 36-frame roll and inspected, along with normal/narrow layer captures. Guides, release binary, all-feature/all-target Clippy, formatting and link/whitespace checks are current. Evidence: `.cache/clean-view-*.log` and `.cache/clean-view-captures/`. No commits or integration are authorized.

## Current state
Implementation and architectural retirement are complete in this worktree, branch `feature/adjustment-layers`. No commits, pushes or integration were authorized or performed. The production release editor is rebuilt at `target/release/antitone`; launch with `cargo app` here.

Projects use schema 10 and ordinary ordered layers. `AdjustmentLayerSettings` and defaults belong to the layer model. `InputPipeline`/`ProcessingParameters` contain interpretation and Roll only; `AdjustmentLayerPipeline`/`GpuAdjustmentLayers` execute the complete stack. Separate upstream photographic grading, creative shader uniforms, single-global collapse and no-GPU single-global execution are removed. Admission without a device checks the same complete stack against default limits; uncalibrated stacks retain intent regardless of coverage.

WB picking captures the selected layer and its preceding stack, accounts for its exposure/coverage/inversion/strength, excludes its own creative controls and downstream layers, and rejects retargeted asynchronous completion. Relative conditioning supports low-strength WB fits while preserving gamma's weak-anchor rejection. Full-stack consumer oracles and diagnostic names/links follow the layer model.

## Verified state
- Core: 164 passed (`.cache/retirement-core-final.log`).
- Persistence, scalar/GPU, consumers, TIFF/EXR, viewer/scopes and DNG integration: 196 passed, 2 opt-in cases ignored (`.cache/retirement-consumer-final.log`).
- Broad editor/GPU unit/workflow checks: 804 passed, 13 opt-in cases ignored (`.cache/retirement-editor-final-followup.log`).
- Full release framework UI: 103 passed, the previously confirmed baseline failure remains (`.cache/retirement-release-ui.log`). Final focused release layer/project/crop/tool/picker checks after review fixes: 30 passed (`.cache/retirement-release-final-focused.log`).
- Production release build, all-feature/all-target Clippy with warnings denied, source formatting, documentation links and diff whitespace checks pass. A source/test/script/guide audit finds no retired PhotoSettings/PhotoGrade APIs, single-global shortcut, global colour prefix or old local-adjustment types/modules.
- Initial implementation regenerated five guide captures; existing normal/narrow offscreen captures remain under `.cache/ui-captures/`.

The initial implementation and the materially new retirement follow-up each received one independent fresh-context review. Follow-up findings were addressed and verified: mismatched matrix in the migrated shared-renderer fixture, scale-dependent WB conditioning, and fractional patch-area coverage that needed disagreeing colour samples plus independent analytic/repeated-sample controls. No repeat review is pending.

## Known limits and next actions
- `correction_widgets_measure_exactly_render_thumbnails_toggle_and_reopen` fails here and on unchanged b10dc1e: its expected project omits the registered flat-field reference source/allocator changes. Evidence: `.cache/baseline-tests.log`. It was not weakened or skipped.
- Native interactive input/dialogs/VoiceOver were not manually exercised; UI evidence is offscreen. No opt-in private-RAW performance or large-patch resource claim is made.
- Last inspected main is 5c888be (single-response #64); this work is based on b10dc1e. Integration must reconcile response/model/schema changes and rerun affected checks.
- Remaining approved implementation steps/blockers: none. Next action: user tries the release or explicitly requests integration. Integration/commits remain unauthorized.
