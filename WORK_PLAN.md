# Editor workflows — handoff

## Agreed scope and delivery constraints

Complete the editor application-layer workflow/ownership refactor. Preserve photographic behavior, source/project compatibility, resource/concurrency policies and interaction design. Keep egui, established executors/task handles and useful private resource queues; do not introduce a replacement general framework.

Implementation: `/Users/emilsahlen/src/antitone/.worktrees/editor-workflows`, branch `refactor/editor-workflows`, based on `58fa628`. Main was inspected, not merged, at `7a149645626c8f4aa91eba4255164afec8b654f4`. Preserve the original prototype/planning worktrees and unrelated root changes (`docs/feedback.md`, `.pi/settings.json`). The user now authorizes a local checkpoint of the existing refactor and one measurement-slice UiContext/UiSpawner trial. No main integration, pushes, installations or other-application automation are authorized; trial changes are to remain uncommitted unless separately requested.

## Current task: checkpoint and one UI-entry slice

Checkpoint the current refactor on this branch, including this plan. Then try ordinary mutable Editor UI with a frame-scoped UiContext and spawn-only UiSpawner. The async factory receives a borrowed StateRef only when polled, outside painting; state updates retain the current notify/repaint behavior. Measurement Apply (button and Enter) should capture immediately, spawn directly and keep its standard task handle in PhotoScope, with no central measurement launcher or deferred input slot. Keep other workflows on their existing path. Test real source capture/history, scope cancellation, errors/shutdown and nested egui UI ergonomics using the live fixture; do not add an inert executor mode or a new task/result framework. Stop for architectural feedback once this bounded slice is demonstrated.

## Existing refactor state

The authorized Analyze acknowledgement and Save continuation follow-up is implemented, verified and independently reviewed once as a distinct delta. No implementation steps remain for those two seams; native/physical acceptance limits below remain explicit. Earlier slices were independently reviewed once per distinct delta. The completed `docs/plans/editor-workflows.md` has been retired; lasting explanations live in `docs/architecture.md`, `docs/editor.md`, `docs/development.md` and the relevant processing/output/project guides. The original plan is also preserved in the untouched planning worktree and disposable pre-cleanup snapshot.

- Application drives borrowed, closure-based UI access and local continuations from host wakes outside painting. Host retirement destroys local futures before application/window/GPU state.
- DocumentScope, PhotoScope and GradeScope own actual accepted/session/visit/grade state and task handles. Captured snapshots are data, not owners. Compatible presentation and physical leases can outlive a continuation without retaining publication rights.
- All feature families and background consumers capture inputs and await typed results. The general backend loop, Command/Message/DocumentWork routing, implicit selected/reference worker slots and Editor::receive are gone.
- Save/candidates/reference/Analyze/looks/Dust/tools/export retain their distinct lifetimes, atomic promotion/history and publication boundaries. Working/original acquisition remains separate from frame-bound colour submission/coalescing.
- Analyze User Cancel retains cancelling activity until service acknowledgement: queued CPU admission can stop, admitted CPU stages are awaited, and native demand waits can retire without awaiting producer cleanup. Owner retirement still immediately drops continuation rights; physical work keeps admission/resources. Controlled gates distinguish these cases without claiming numerical or native-interruption acceptance.
- Save completion directly performs the accepted New/Open/Close continuation and enters candidate execution outside painting. Native quit approval is immediate; viewport Close presentation remains host-bound. Cancelled decisions, failed/stale results, written baselines and later edits retain their separate checks.
- Native priority/coalescing, one shared physical foreground admission, worker-local mutable RAW backing, optional cache ownership and private thumbnail/JPEG scheduling remain intact. Running work retains admission/resources through cleanup.
- All 41 stored kittest borrow aliases, inline guard accessors and 84 construction macros are removed. Constructors keep the borrowed owner private; test reads/updates are short transactions. Ordinary EguiHarness component APIs are unchanged.
- Controller-only drawing creates no executor. Measurement Apply captures its source/correction/numerical Input immediately; production post-draw workflow entry starts its photo-owned task. The stopped-executor/controller-pick adapters are deleted. New pre-entry cancellation coverage and the strengthened authored capture test protect this interval.
- Empty transport assertions were either replaced with meaningful typed request/session guards or removed when structurally vacuous. Numerical/photographic expectations and tolerances are unchanged.

## Latest verification

Follow-up evidence is in `.cache/workflow-followup/`:

- Focused Analyze/preparation/fitting/admission/Save/file/menu/quit/history/fullscreen/workspace matrix: **150 passed / 4 existing ignored in each profile**, covering all 154 planned cases. Debug completed through the initial partial batch plus bounded remaining/refined groups; release completed in 11 bounded exact groups. `verification-summary.json` checks exact coverage rather than counting the timed-out batch as complete.
- **34 production editor checks** and **21 composed Analyze/menu/project/history cases per profile** pass. These include controlled CPU acknowledgement, immediate owner retirement, early/queued cancellation, real no-paint Save → New/Open/Close and failed Open after successful write, plus guarded native quit approval and later viewport-command emission.
- Production editor debug/release builds, **151 core tests**, all-target ui-tests Clippy `-D warnings`, contributor formatting, whitespace and **267 local destinations across 7 changed guides** pass.
- Independent review found one existing Quit assertion requiring the removed paint delay. It now expects immediate approval and separately checks later viewport Close output; the focused regression passes. No other actionable findings. Review scope and limits are in `review.md`; this was not a repeat review of earlier milestones.
- Main/root status was rechecked unchanged; no commits or integration. No native windows were launched.

Earlier final-cleanup evidence remains in `.cache/final-lifecycle/`:

- Production editor debug/release and default core builds pass; all-target ui-tests Clippy `-D warnings` passes.
- Default/no-feature core suite: **151 passed**, no failures.
- Measurement/photo/document/activity/analysis/reference controller matrix: **64 passed / 2 existing ignored in each profile**. This includes the new pre-entry cancellation case and strengthened captured-source/no-paint case.
- Root Editor/viewer/workspace controllers: **85 passed per profile**. File/menu/quit controllers: **34 passed per profile**.
- Explicit production application/admission/native/measurement/proper-thumbnail/JPEG matrix: **56 passed**. This includes controlled native backing/admission retirement and real authored extraction/producer/composed consumer cases.
- All **86 composed cases** completed in seven bounded exact groups per profile: debug **79 passed / 7 inherited failures**; release **78 passed / 8 baseline-reproduced failures**. No timed-out group is counted as complete. The seven established primary diagnostics match `.cache/baseline-composed.log` exactly.
- The extra release failure is the unchanged roll-selection accessibility-description assertion: `Active photo. Preparing thumbnail; navigation preview may differ.` versus exactly `Active photo`. The saved pre-refactor foundation release binary independently reproduces the identical failure (`roll-active-baseline-release-0.log`). It is left unchanged, not relabelled as a regression or silently fixed.
- Earlier families' software/native evidence remains in the feature-named `.cache/` directories. The earlier 216-case transport batch timed out; it is historical partial evidence, not a completed suite.

Native executables also compiled and were invoked without `--run`; they explicitly declined to launch windows. This is not a new native lifecycle pass. Final contributor formatting, whitespace and **267 local destinations across 7 changed guides** pass. Main and unrelated root changes were rechecked unchanged; branch HEAD remains `58fa628` with no commits.

## Review and lifecycle audit

The single independent milestone-8 cleanup review reported **no actionable findings**. It examined the isolated cleanup delta, complete measurement lifecycle/cases, harness construction/teardown and selected migrated scenarios/assertions, independent oracles and control discovery. It did not exhaustively inspect every migrated case or independently reproduce verification. Do not repeat this review merely because a later handoff occurs.

The direct source-backed trace is `.cache/final-lifecycle/lifecycle-audit.md`: application/host retirement, renderer prewarm/cache, screening cache, BlockingJobs, native Scheduler/Ticket/NativeBacking/store closure, concrete scope replacement, feature-service captures, private thumbnail/JPEG producers and submission tracking. No missing-close path or ownership cycle was demonstrated; finite jobs/immutable caches did not receive a uniform close API or a new physical policy. Delegated maps that described old task layouts or missing current files were discarded, not used as evidence.

## Remaining limits and next actions

1. The agreed follow-up is ready for its user-authorized checkpoint. Next, carry out only the UI-entry trial above; refresh main/guidance at any eventual integration checkpoint. The two reported gaps are resolved, not a claim of exhaustive workflow/interleaving acceptance.
2. Preserve inherited failures: working-photo/WB/Linear/Radial waits, Dust Reset/Brush-size discovery, physical-view processing identity rejection and the independently reproduced description-timing assumption. Exact diagnostics and per-profile results are retained; no numerical fix or tolerance relaxation is part of this task.
3. Native focus remains unresolved after the earlier two Dust native timeouts. Do not relaunch without a concrete diagnosis, weaken native checks or manipulate other applications. Process listing was sandbox-denied; do not retry that access without approval.
4. Source tracing and controlled gates do not establish native interruption/destruction timing, unfinished GPU execution, exhaustive cancellation/metadata/cache/preflight/device-failure interleavings, physical input, photographic appearance or aggregate memory acceptance. Existing owning plans retain that genuinely separate scope.

The follow-up's `.cache/workflow-followup/before/` reconstructs the pre-follow-up state from the preserved final-cleanup baseline and delta; its `source-delta.diff` isolates only these fixes and related tests/docs. No generated transformation was applied to maintained source.

Disposable final-cleanup checkpoints: `before.tar.gz`/`before/` are the already-reviewed thumbnail baseline; `transport.tar.gz` precedes harness cleanup; `transactions.tar.gz` precedes closure construction. `source-delta.diff` and `changed-files.json` isolate milestone 8. AST generators/manifests in this cache are disposable review aids, not maintained tools or instructions to reapply old edits. All maintained source changes used exact edits and normal contributor build/native setup.
