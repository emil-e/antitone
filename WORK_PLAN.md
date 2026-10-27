# Async source acquisition and execution simplification

Current status: implementation and requested rebase complete on main e22f216; clean local branch, no push/PR. Earlier sections record preceding checkpoints; the final rebase results below supersede their integration warnings.

## Agreed scope
- Async source acquisition returns existing immutable developed-source leases.
- Cache wraps loading, reusing SourceStore's weak/live and optional strong ownership; no parallel pixel or derived-artifact cache.
- Remove BlockingJobs and speculative cross-feature admission/thumbnail-idleness policy while preserving cancellation acknowledgement, stale-result rejection and worker-owned inputs through physical completion.
- Preserve native ownership and SDK synchronization. Leave GPU submission/configuration synchronization and GPU internals untouched; add no replacement throttles/frameworks.
- Separate worktree and coherent local commits. No push, PR or integration authorized.

## Acceptance / outcome
Implemented on fetched main 6cd71b3; refreshed upstream/guidance at the execution and final-review milestones with no relevant upstream changes.

SourceProvider is async at the root. SourceCache wraps SourceLoader using the existing SourceStore. Editor/source, analysis, thumbnails and export acquire existing Arc leases; native requests contain no stores, parent caches, retention choices or admission permits. Weak completed-result discovery covers the native-completion/publication handoff without retaining pixels strongly. Analysis owns its measured samples.

BlockingJobs and global service plumbing are removed. Synchronous CPU jobs use the shared blocking pool with panic reporting; thumbnail eligibility no longer reads foreground-pool idleness or polls for its permit. The embedded-preview worker uses its serial execution directly, removing an extra semaphore and the async-lock dependency. GPU implementation files are unchanged.

Owning guides explain acquisition, retention and execution, including one compact provider/cache diagram. The older architecture-walkthrough worktree remains untouched.

## Review follow-up
Agreed: remove machinery for detecting external RAW modification/replacement during an open session. Treat open RAWs as immutable; remove repeated filesystem freshness checks from acquisition, cache access, joining, publication and other guards serving that purpose, plus offloads needed only for those checks. Opening/reopening can capture identity for persisted-cache validity; source/development compatibility validation remains a separate concern. Implementation is now authorized; verification below still covers the initial implementation.

Agreed: make the existing backend itself SourceLoader in source_loader.rs, with SourceProvider implemented directly. Remove the adapter and editor_development.rs naming. Priority is an acquisition argument passed through the cache on misses, not a property of a leaf-loader wrapper. Preserve existing worker ownership, joining, independent cancellation and SDK synchronization.

Follow-up implemented: source_loader.rs owns the backend and implements SourceProvider directly; source_metadata.rs holds shared header/development validation with no editor dependency. Priority travels per acquisition. Cache hits, publication and submission are synchronous in-memory operations inside the async interface, with no pool hops or RAW re-stat checks. Document captures file identity at Import/Open; clones and Save As preserve it, including history-only sources. Processing/document/request validation remains. Opt-in private diagnostic checks still protect their explicitly bounded/read-only inputs rather than application cache freshness.

Final follow-up verification: 778 debug domain tests passed (9 existing opt-in diagnostics ignored); 27 composed debug UI tests passed; 65 focused release native/UI tests passed (1 opt-in diagnostic ignored); authored source-store/DNG integrations passed 27 tests (2 opt-in checks ignored). Core tests, strict UI-feature clippy, separate production editor check, formatting, documentation links and diff checks passed. A fresh-context review of this materially new follow-up found two identity-capture gaps (history-only records across Save As/Undo, and explicit Import of an unavailable reference restored since Open). Both were fixed; both regression tests fail with the fixes disabled and pass when restored. The document/history/native/provider suite passed 74 tests, 1 private diagnostic ignored.

An additional raw,project-only clippy run with warnings denied is blocked by two unchanged raw.rs helpers (uses_sdk and photo_orientation_from_tiff) being unused without editor/DNG-SDK features. The introduced feature-specific warnings were fixed; unrelated raw helpers were not changed. Windows runtime, full spatial-math library completion and opt-in private workloads remain unverified.

Async-consumer audit: no production caller requires a blocking source-acquisition interface. Selected/export and analysis await SourceProvider; the thumbnail resource worker manually polls pending provider futures so its synchronous loop can serve several demands. SourceLoader::load/Ticket::wait and source::Request::load synchronous facades are cfg(test), including opt-in diagnostics; they reflect fixture organization, not an inherent inability to use async. Native C++ calls execute synchronously inside loader workers, not in consumers.

Follow-up implementation, review and local verification are complete in one coherent local change covering the linked loader/interface/freshness simplification. No push/PR; integration and adaptation to newer main remain separately authorized work.

Upstream refresh now includes 1e26218 (Digital source brightness), touching source/document/store boundaries. This resumed task remains on its existing base; inspect/adapt those changes during integration rather than silently mixing unrelated product work into this follow-up.

## Independent review
One fresh-context review covered the full change, owning guides and test quality at whole-case, assertion and fixture/control-discovery levels. Three findings were confirmed and fixed:
- Native retirement atomically excludes unwanted tasks from joining before worker-local cleanup; a late replacement starts fresh.
- Closed-document thumbnail consumers drop pending provider futures; disposable GPU-ticket changes still preserve useful CPU/disk work.
- The private throughput diagnostic isolates measured schedules from completed-result discovery and asserts actual native-call counts, preventing oracle leases from turning native timings into cache-hit timings.

Both retirement regression tests fail with their fixes disabled and pass when restored. No second review was requested after addressing findings.

## Verification
- Final debug editor/provider/store/revision/cancellation/application suite: 784 passed, 9 existing opt-in diagnostics ignored.
- Release native/provider/thumbnail workflows plus composed activity, Analyze, export, dust-persistence, source-working and underlay: 54 passed, 1 opt-in diagnostic ignored. The 17 composed UI cases also passed in debug on Metal.
- Authored source-store/DNG integration: 27 passed, 2 existing opt-in checks ignored.
- Core no-default-features suite passed.
- UI-feature library/test clippy with warnings denied and separate production editor check passed after lifecycle fixes.
- Focused native/provider/thumbnail lifecycle suite: 37 passed, 1 ignored. Negative controls: both new retirement regressions fail as intended without their fixes.
- Final formatting, documentation links and diff checks passed. Shared checkout-local native dependencies; offline builds, no downloads/system installs. Disposable logs/artifacts are under .cache/.

## Pressure-independent development follow-up (#37)
User explicitly authorized removing pressure-driven RAW thread-count reductions and second-owner collapse. Keep optional cache backoff/prefetch suspension, priority, CPU reservation, shared native thread budgeting and SDK ownership/exclusivity unchanged. Do not add replacement throttles or a speculative benchmark project. GPU internals remain out of scope.

Completed: RAW thread selection and loader eligibility no longer read or accept memory pressure; removed loader pressure-injection constructors and fixture plumbing. Cap/reservation/sharing and both-owner priority/pinning/SDK exclusivity tests remain. Cache/prefetch and GPU implementation files are unchanged. Owning guides reflect the separation. No replacement policy or benchmark added.

Verified: 154 focused debug tests passed (2 existing opt-in diagnostics ignored); 166 release tests passed (6 opt-in diagnostics ignored), including RAW/native/provider/source workflows and thumbnail/cache/prefetch behavior. Strict editor and UI-feature clippy, separate production check, repository formatting, documentation links and diff checks passed. No private workload, Windows-runtime check or full-library rerun for this narrow follow-up. Ticket/thumbnail-loop discussion does not independently authorize those separate changes.

Latest upstream refresh is 727c70d; guidance remains unchanged, while main's resolved-input/photographic work requires later integration adaptation. Continue in this task worktree without mixing that unrelated implementation into this follow-up.

## Shared document cache design discussion
Agreed direction: DocumentScope owns one SourceCache across same-document operations; storage moves into SourceCache, replacing separate SourceStore/fork/parent/transfer machinery. Joining stays in SourceLoader alongside native execution and priority promotion, avoiding another cross-layer priority/cancellation protocol. The user has now authorized implementing this cache lifetime/storage simplification. SourceCache will own storage directly; its async load method receives the shared provider, so a document cache need not own or create native workers. Document snapshots retain source identity but no cache handles. Same-document preparation shares the scope cache; New/Open prepares independently and seeds the new scope only after acceptance.

Explicit user requirement: when a source queued by eager thumbnail loading is selected, promote that existing joined load to foreground priority so selection does not wait behind the remaining low-priority queue. Preserve highest-active-consumer priority and independent waiter cancellation; do not create duplicate foreground development. Native calls already executing remain non-preemptible. The current loader-level joining already supports promotion and will stay there.

## Source module containment
Current user request: put core source acquisition in one module. Consolidate loader, provider/cache interface, storage, keys, persisted identity and metadata validation under src/source/ with a small re-exported API; private implementation files are not separate public module APIs. Keep editor-specific view/correction preparation outside. Preserve feature boundaries, source behavior and loader-owned joining/priority. This containment checkpoint preceded the DocumentScope/cache-storage redesign completed below.

Completed containment: src/source/mod.rs exports the provider, cache, loader, keys and storage types; implementation files are private. SourceKey is separate from LRU storage. Imports, diagnostics, integration tests and guide links use the unified module. Native loader scheduling and storage bodies are unchanged (apart from visibility restored after nesting). No GPU/prefetch implementation changed.

Verification: 216 focused debug and 216 release tests passed (2 opt-in diagnostics ignored in each); 26 source/DNG integration tests passed (2 opt-in diagnostics ignored), and 39 project persistence integrations passed; the no-default-feature core suite passed. Strict editor and UI-feature lib/test clippy, separate production editor check and project-only check passed; raw,project tests compile with the existing off-editor unused helpers. The added queued-selection regression runs through SourceCache with independent per-source fixture developments: selecting the last eager entry executes it ahead of the middle entry, once only. It fails when joined promotion is disabled (first-waiter priority only) and passes after restoration. Logs: .cache/source-module-*.log. Formatting/docs/diff checks and final inspection passed. This containment change is complete; its subsequent DocumentScope/cache-storage simplification is recorded below. No push/PR.

## Document cache simplification (complete)
Acceptance: one SourceCache per DocumentScope, persistent across same-document edits/Save As/Undo and discarded on document replacement; no SourceStore, candidate cache forks, parents or acceptance transfers. Loading, proxy retention, budgets/pressure and live leases keep existing behavior. SourceLoader retains joining/priority promotion. Closed-scope consumers retire while independent export demand remains valid. Capture file identity explicitly with source requests, separately from cache handles.

Implementation is in place: SourceStore and editor source-cache descriptor removed; SourceCache directly owns storage; DocumentScope closes it; source requests carry captured file identity explicitly. Analysis/reference/export services receive captured scope handles separately from Document data. Same-document candidate preparation shares the cache; accepted New/Open seeds native/proxy entries from Ready. Retention and native joining algorithms remain unchanged.

Initial domain run: 762 passed, 9 ignored, one obsolete unknown-reference cache-presence assertion failed and was updated to protect its actual saved-record/WB invariant. Composed workflow/history/export/correction/project/Analyze run: 33 passed, one pre-existing correction test fails at editor_kittest_correction.rs:534 after re-adding a removed reference. Reproduced unchanged at b03c168: actual correctly adds source id 3 and advances next_source_id to 4; expected merely sets the flat-field and omits the re-registered reference (next_source_id 3). Leave that unrelated test untouched; do not claim that case or its later reopen assertions passed. Baseline logs: .cache/scope-cache-baseline-correction.log. Baseline source-tree experiment restored all task sources; the detached baseline worktree was removed.

Fresh independent review examined ownership, scope/source/cache/workflow tests, renamed integrations, priority fixtures and export retirement; one actionable finding: first-time FileKey::new had moved into UI source/reference capture. Fixed at the existing background header preparation boundary; registered sources still reuse captured identity. Added unregistered_source_requests_defer_file_identity_until_preparation; it passes and fails with UI-thread capture restored. Review found no other ownership issue; review supplements runtime checks. Detached baseline worktree disposed and source-tree experiment fully restored.

Latest broad debug domain run: 764 passed, 9 ignored (before the new deferred-identity regression, which passes separately). New accepted-Open regression proves the new scope has the prepared native and proxy leases and the old cache closes despite captured document data remaining alive.

Final verification: 765 release domain tests passed (9 opt-in ignored); final post-review targeted debug run passed 231 tests (2 ignored), supplementing the earlier 764-test broad debug pass. Source-cache/DNG integrations passed 26 (2 opt-in ignored); project integrations passed 39; no-default-feature core suite passed. Strict editor and UI-feature lib/test clippy, production editor check and raw,project test compilation passed (existing off-editor unused-helper warnings remain). Both debug and release composed suites passed 33 cases and failed only the unchanged, baseline-reproduced correction fixture at line 534. The Open-cache seeding regression fails with publication disabled and passes after restoration; deferred identity capture likewise has a failing negative control. Formatting, documentation links and diff checks passed. Logs: .cache/scope-cache-*.log. Implementation and final inspection complete; no further implementation blocker. Milestone refresh reached ab0999e; contributor instructions are unchanged. Main has reorganized editor/GPU files and renamed scene fitting since the previous refresh, in addition to the already-known resolved-input/photographic changes. No upstream implementation was merged; later integration must adapt those boundaries explicitly. No GPU changes, source-ticket/thumbnail orchestration redesign, private benchmark, push or PR.

## Rebase onto current main (complete)

User requested rebasing onto main and adapting the changed structure. Replayed all task commits onto ab0999e, then onto e22f216 after main advanced during verification. This includes the newer editor/GPU organization, resolved-input/Digital exposure and preview sharing, independent CI stages (#86), and accepted-crop Film Emulation scaling (#87). Local safety ref backup/async-sources-pre-main-rebase remains at a95dbd1. No push/PR; working exclusively in the async-sources worktree.

Preserved one SourceCache per DocumentScope, loader-owned joining/priority/cancellation, immutable session identity, pressure-driven cache/prefetch behavior, and absence of global admission or RAW pressure throttles. Added upstream preview single-flight/byte accounting and canonical exposure memos inside SourceCache without candidate forks. Document snapshots carry completed exposure data, not cache handles; cache restore/snapshot uses captured source keys without filesystem IO. New/Open prepares independently and seeds its accepted scope. GPU diff from main is only the source API rename in gpu_export/job.rs.

Adaptations also preserve main's Sony as-shot floor by delegating metadata construction to SourceDevelopment::from_metadata. Pending-preview incompatibility coverage now admits a valid geometry change rather than depending on obsolete filesystem freshness to reject invalid WB.

Fresh independent review covered the materially new source interpretation, ownership, admission and test quality. Three confirmed issues fixed: restored exposure leaking through loader-shared DevelopedSource initialization; relative Save As rebasing locators before normalized numerical validation; history missing canonical offscreen values for the restored target. Interpreted native headers now share frozen storage without mutating the original snapshot; Save validates before rebasing; data-only capture_exposure snapshots current canonical values before history/final admission. All three regression cases pass and fail when those fixes are disabled. The composed automatic Digital-import regression checks native storage identity instead of interpretation-header Arc identity and verifies the original view stays unchanged. No repeat review after fixes.

Final verification on e22f216:
- Debug and release domain/source/editor/exposure/preparation/normalization suites each passed 790 tests (9 opt-in ignored). An earlier 180s debug limit was superseded by the complete 300s-budget run; its last test also passed in isolation.
- Release composed workflow/history/export/correction/project/Analyze/exposure/Film Emulation plus the three regressions passed 45 cases. The old pre-rebase correction fixture passes on current main.
- Source-cache/DNG integrations passed 26 (2 opt-in ignored); project persistence including Film Emulation passed 41; no-default-feature core suite passed.
- Strict editor/UI-feature library/test clippy, production editor check, raw-only/project-only/raw+project test compilation passed. Existing off-editor unused-helper warnings remain.
- Formatting, documentation links and diff checks passed. Guide growth is the acquisition/ownership explanation and compact diagram; hedge matches introduce no new implementation mandates.
- Logs: .cache/rebase-latest-*.log; review regressions/negative controls: .cache/rebase-review-*.log.

No remaining implementation blocker. No ticket/thumbnail orchestration redesign, private benchmark, native-window automation, Windows runtime, push or PR. Full unfiltered library coverage is not claimed.

## Limits / handoff
The whole editor-feature library debug attempt reached unrelated spatial-math tests before a five-minute timeout. Its one editor cancellation-fixture failure was fixed and included in the completed final editor-domain suite; no full-library pass is claimed. No Windows runtime, private throughput workload or native-window automation was performed.

The initial implementation is in six coherent local commits; the completed follow-up adds the actual root loader and immutable-session acquisition model. No push or PR performed; remote delivery requires an explicit integration request.
