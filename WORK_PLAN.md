# Scoped source/cache and ownership follow-up

## Status

The approved implementation is complete and locally verified. The user has now explicitly requested integration as a pull request. Main and other worktrees remain untouched. Latest fetched origin/main is still 63d1f6d, so no code reconciliation is needed. Delivery steps: checkpoint agreed work, transfer remaining state-model follow-ups into EDITOR_STATE_PLAN.md, remove this session plan, run final delivery checks, push this branch, create PR and watch CI. Existing independent reviews cover the material change set; do not repeat them solely for delivery.

- Worktree: `.worktrees/scoped-source-cache`
- Branch: `refactor/scoped-source-cache`
- Base/upstream: fetched `refs/remotes/pi-github/origin/main` at `63d1f6d`, after PR #90 and #91. Refreshed before the ownership review; no intervening upstream changes.

## Agreed scope and provenance

The user authorized async thumbnail/embedded-preview coordination, ordinary document-scoped cache destruction instead of close/is_closed, and removal of cache-based identity/authority. They subsequently authorized SourceProvider encapsulation and the broader ownership alignment below.

Document-specific logical activity generally ends with its document. Uncancellable physical cleanup is an implementation tail, not a surviving product operation. The user establishes document-owned RAM retention as the default; hypothetical cross-document reuse does not justify retaining document artifacts in memory. Shared infrastructure and existing disk reuse remain separate.

The approved delivery split includes lifetime ownership, coherent feature sessions, and removal of compensating reset/rebind/identity machinery. Preserve tool/navigation/display behavior, immutable leases, recorded exposure, native joining/promotion and independent cancellation, physical input ownership, native scheduling, serial thumbnail processing, cache/prefetch pressure backoff and existing GPU processing/synchronization.

Explicit follow-ups, not solved by moving fields:
1. Accepted-versus-editable state/source-of-truth, copy-back, history and persistence.
2. Authoritative operations with derived activity/busy presentation.
3. Further cache optimization only when measured.

## Implemented ownership

- DocumentScope owns thumbnail/image and per-source GPU-grade retention, correction/flat-field session, filmstrip selection, picker session, export continuation/draft/summary and existing project control mirrors. Replacement constructs empty retention while sharing worker/device/kernel infrastructure. In-memory thumbnails do not survive replacement; disk camera-proxy reuse remains.
- Removed weak document identity, thumbnail rebinding, the auto-crop weak containment token and DocumentScope's enumerated child-task Drop teardown. SourceCache has no close/closed/is_closed or native acquisition wrapper.
- PhotoScope owns working images/mailbox, selected previews/timing, photo control/viewport snapshots, pointer drags, Dust gestures/GPU patches/healing continuation and the single-photo spatial consumer. Ordinary replacement drops those resources and task handles together. Tiny viewport/control snapshots still hand off for reference/history behavior without retaining old image/operation owners.
- GradeScope owns original reconstruction state/results/generation alongside its continuation. Dropping OriginalView signals its captured physical generation. Revisions are local to that owner, not globally monotonic.
- Picker kind/return/view/support form one document Session because reference picking spans visits; drags remain visit-owned. Picker navigation explicitly uses temporary-source loading, preserving the exclusion of pending working previews.
- Dust document state retains source-keyed completed/failed CPU results, selections and released donor searches. Viewport/downstream grades preserve active healing; visit retirement cancels it. Source-keyed cache freshness and same-visit admission remain intact. Released searches still survive browsing.
- Shared spatial preparation stays document-owned; Photo auto-crop continuation is visit-owned. Unsuccessful terminal paths retire that consumer so retries remain available; successful Save/failure-ack deferral remains intact. Fresh visits hide Dust artwork until entering the tool.
- Export continuation is document-owned. Current normal UI still blocks replacement/closing during export. A forced owner retirement cancels logical work while the physical call retains inputs through cleanup; no close-while-export feature is added. Captured recipe/source data remain stable if live document data changes within an owner.

## Acquisition and physical execution

- Internal object-safe SourceProvider exposes native/proxy/preview acquisition and optional immediately available proxy data, not budgets, publication or recorded exposure. Cache-backed and cacheless implementations share the contract. Removed unused weak-provider construction/retention enum; legitimate weak pixel/preview discovery remains.
- SourceCache stays at scope/provider construction and retention management, not on source/analysis/candidate/reference/embedded/thumbnail worker requests. Photo preparation consumes the provider and separately resolves the captured document exposure slot.
- Physical header/candidate/estimation/processing jobs receive provider-free data and pixels. Thumbnail finishing returns proxy products for scope-owned publication. The shared serial worker has no cache or document generation field; GPU ticket generation travels with each job.
- PreviewLoader owns cache-independent shared flights and weak live discovery. Async waiters suspend normally; background work owns decode inputs through completion. Thumbnail coordination uses normal async acquisition and one short-lived priority-ordered ready drain, not dedicated polling threads/no-op wakers.
- No native scheduling, RAW development or GPU processing/synchronization implementation changes. Global operation/activity/busy semantics and editing mirror authority/copy-back remain unchanged.

## Final verification

Offline contributor workflow with existing shared native dependencies and CARGO_INCREMENTAL=0:

- Selected broad source/editor/photo-preparation/exposure/source-scale/cold-export suite: **945 passed, 13 opt-in ignored in both debug and release**, including **119 composed editor kittest cases per profile** and Metal-dependent rendering/numerical/lifecycle checks. Not a full unfiltered library claim.
- Selected integrations: **100 passed, 2 ignored** across source-cache, DNG, camera/dust/film/flat-field/image-recipe/project persistence, adjustment-layer consumers, viewer and scopes.
- No-default-feature numerical-core suite passed. Raw-only, project-only and raw+project library/test compilation passed. Off-editor raw configurations retain existing unused-private-helper/import warnings; no strict off-editor clippy claim.
- Production editor all-target clippy and UI-feature library/test clippy passed with warnings denied.
- Authored formatting, documentation links and git whitespace checks passed. Architecture grows 39 words; editor guide replaces obsolete cross-document RAM reuse prose. Completed SOURCE_LOADING_PLAN.md is removed.
- Final inspection includes new/untracked files and confirms excluded native/GPU implementations unchanged.

### Meaningful tests and controls

- Identical-data document replacement releases actual thumbnail images and grade allocations, while shared kernels/device infrastructure remain usable.
- Gated thumbnail/export retirement proves physical inputs survive and are released after completion without retaining the document cache or publishing into the replacement owner.
- Single-photo spatial consumer test gates real native acquisition, injects declared compatible geometry and proves a replacement visit's crop is not retargeted. Its usable-proposal positive control avoids a blind-detector abstention making the case vacuous.
- New error/rejected-publication retry case proves a subsequent Photo request is admitted in the same visit and consumes valid geometry. Rejected publication uses an invalidated store epoch with otherwise compatible inputs.
- New composed Dust navigation case uses real UI tool entry, a released repair, navigation away/back and isolated production artwork painting. It checks absence/presence without selecting controls by colour or freezing line styling; re-entering Dust is the positive control. Isolated paint output intentionally clears its unused egui texture delta.
- Five runtime controls failed at intended assertions and were restored: cross-document thumbnail RAM carryover, missing original-generation retirement, departed Photo consumer retained in document, provider/cache owner inside physical Work, and demoted provider priority.
- Review-fix controls also failed meaningfully and were restored: missing failed-preparation cleanup, independently missing rejected-publication cleanup, and fresh visits enabling inactive Dust artwork.

## Independent review and investigations

This NEW ownership/acquisition delta received one fresh-context review against `.cache/pre-ownership/{src,docs,tests,native}` (saved from the verified prior provider candidate). The reviewer examined complete touched guides, physical captures, destruction/cancellation, retention and composed workflows, plus test quality at whole-case/assertion/fixture/control-discovery levels. Report `.cache/ownership-review.md`.

Two findings were assessed and fixed: stranded Photo spatial consumers on unsuccessful terminal paths, and inactive Dust overlays restored on navigation. Focused cases and runtime negative controls protect both; final debug/release/strict checks pass afterward. No repeat review after fixes was requested or performed. The final subsequent cleanup only simplifies default construction, updates a scope comment and removes an unused optional-cache branch from the preview helper; checks were rerun.

Previous materially different candidates were separately reviewed, not repeatedly reviewed as one change set: `.cache/scoped-source-review.md` (original weak-handle candidate) and `.cache/provider-review.md` (native provider revision). Their earlier verification is not substituted for the current results.

Investigations preserved behavior rather than weakening checks: viewport/history handoff after moving view state; picker navigation must remain temporary-source demand; zero-limit fixtures must bind their own scoped cache at construction; original-generation tests now verify retirement of the actual old owner rather than cross-owner numeric ordering; original export-survival tests no longer assert an unsupported product exception. A blind spatial detector abstention was replaced with declared geometry plus a usable-proposal control. SDK/GPU algorithms and numerical tolerances were unchanged.

## Reproduction and artifacts

    CARGO_INCREMENTAL=0
    LIBRAW_SOURCE=/Users/emilsahlen/src/antitone/.cache/native-deps/LibRaw-0.22.0-openmp
    OCIO_PREFIX=/Users/emilsahlen/src/antitone/.cache/deps/ocio-install

- `.cache/ownership-domain-ui.log`, `.cache/ownership-release-domain-ui.log`
- `.cache/ownership-integrations.log`, `.cache/ownership-core.log`
- `.cache/ownership-production-clippy.log`, `.cache/ownership-clippy.log`
- `.cache/ownership-features-{raw,project,raw-project}.log`
- `.cache/ownership-format.log`, `.cache/ownership-docs.log`
- `.cache/ownership-negative-controls.log`, `.cache/ownership-review-negative-controls.log`, `.cache/ownership-review-rejection-control.log`
- `.cache/ownership-retirement-tests.log`, `.cache/ownership-review-regressions.log`, `.cache/ownership-export-slice.log`
- `.cache/pre-ownership.patch`, `.cache/pre-ownership-source.tar.gz`, `.cache/pre-ownership/`

The target was APFS-cloned from the previous task; path-bound generated DNG CMake output was regenerated normally. Earlier ENOSPC was resolved by removing only this worktree's disposable incremental/package artifacts. No other agents' caches were cleaned. Shared disk space remains tight.

## Limits and next action

Windows runtime, native-window automation, private throughput workloads and exhaustive cancellation/GPU/panic schedules remain unverified. No speedup claim is made. Review was static, not an independent rerun of reported checks/controls.

Ready for user assessment. Follow-ups above remain separate work. Do not integrate without a new explicit request. If delivery is requested, refresh/reconcile upstream and guidance, preserve unrelated main/other-agent edits, checkpoint agreed work as appropriate, transfer/remove this session plan before delivery, push the task branch, open/update a PR and watch CI. Never push main.
