# Finish feature-owned workflow initiation

## Agreed scope and constraints

The user authorized migrating all post-draw workflow starters to feature-owned sequential async initiation through combined borrowed UiContext. Keep normal mutable Editor/raw egui UI, deferred factories, short continuation transactions and standard owner-held tasks. No shared Model, projection/DI/result framework, replacement scheduler or relocated starter catalogue. Preserve photographic behavior, input/focus/history transactions, project compatibility, cancellation acknowledgement, physical admission, native priority, specialized coalescing and cache/GPU policies.

Commands stage state and register tasks atomically. Read-only algorithm/input builders are fine; do not manufacture stateful capture/launch pairs for tests. The user's clarification prioritizes understandable production code over convenient tests. Rendering-only fixtures reject execution; typed controller proposals do not establish producer/admission coverage. Stop with concrete code if the approach needs an ugly abstraction or materially changes design/risk.

Worktree: `.worktrees/editor-workflows`, branch `refactor/editor-workflows`, above checkpoint `8130cb3`. The user's explicit integration request now authorizes local checkpoint commits, rebase and fast-forward delivery to main. No push, installation, native-window or other-application automation is authorized. Root `docs/feedback.md`/`.pi/settings.json` and other worktrees are untouched. Main/guidance refreshed at the completed-initiation milestone; main remains `7a14964`.

## Current state: implementation and review complete

- Combined UiContext owns no Editor: external state outlives the local executor; factories receive context outside the mutable UI borrow. Split/compatibility names and manual context-capture plumbing are removed.
- Atomic commands cover measurement, reference/correction, Save/document/history/guarded transitions, initial opening, looks/export, Analyze preparation/fitting, source/visit and lazy support. Save directly resumes New/Open/Close without painting.
- Original/working, Dust, embedded-preview and thumbnail demand initiates at the consumer. The production post-draw catalogue and transport-only input slots are gone. Settling, working latest-demand coalescing, visible/hidden/eager queues, producer acknowledgement and physical retention remain.
- Adopted main's bounded physical-view precision-identity guard fix, with new regression coverage for tightened local precision and mismatched global/local snapshots. No duplicate grade constants/accessor, numerical preparation or shader change.
- Immutable-input/service, typed controller, live command/submission and real gated producer tests have distinct claims. Thumbnail fixtures bind the actual document store. Composed Dust fixtures settle accessibility before querying controls and keep RAW/project paths distinct. Active-photo accessibility permits contextual thumbnail readiness details.
- Human guides describe current ownership/initiation and test boundaries. 206 local documentation targets resolve. One independent fresh-context review examined production lifecycle and meaningful tests; its sole finding was addressed and rechecked. Coverage limits/evidence: `.cache/initiation-migration/final-review.md`.

## Final verification

Disposable evidence: `.cache/initiation-migration/`.

- Full release `ui-tests` lib: **1029 passed, one failed, 38 ignored** (`final-all-release.log`). The sole failure is an untouched GPU test fixture described below. All composed editor cases passed.
- Production `editor` application/execution/admission/editor selection, without OCIO unification: **683 passed, five ignored** (`final-production-workflows.log`).
- Full debug lib before final fixture repairs: 1026 passed, three failed, 38 ignored (`all-initiation-debug.log`). Both Dust failures were repaired and rechecked with identity/working/guarded-close cases: 12 passed (`final-dust-identity-working-debug.log`). Historical correction/thumbnail/local/mask failures passed. Final release covers the completed repairs and review fix.
- Production release binary build, **151 core tests**, all-targets Clippy `-D warnings`, contributor formatting and `git diff --check` pass (`final-production-build.log`, `final-core.log`, `final-clippy.log`, `final-format.log`). GPU-only compilation passes with the pre-existing unused `look_id` warning (`final-gpu-check.log`).
- Native executables compile and ordinary software-contract invocations pass without `--run`; no native windows were launched (`final-native-contracts.log`).

## Remaining independent defect and delivery boundary

`gpu::flat_tests::corrected_conversion_matches_f64_and_full_pipeline_preserves_flags_and_precision` omits camera-matrix binding 3 from its custom conversion bind group in `src/gpu_flat_field_tests.rs` (four entries for the shader's five bindings). It fails validation before numerical assertions. The test and `src/shaders/camera_conversion.wgsl` are unchanged from `58fa628`; leave this independent fixture repair as reported follow-up rather than silently expand numerical scope. The full suite is therefore not green.

Integration in progress: checkpoint agreed changes, rebase onto main `7a14964`, resolve overlapping guides without losing main's export/colour changes, and inspect semantic interactions (the GPU identity guard already matches main). Verify the combined result, transfer remaining independent fixture defect to contributor guidance, remove this task-local plan and fast-forward main. Inspect ignored artifacts before safe worktree cleanup. Photographic, physical-input/focus/chooser/window lifecycle and aggregate-resource acceptance are not established by software checks. Do not push without separate authorization.
