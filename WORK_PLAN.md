# Async source acquisition and execution simplification

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
Agreed direction: DocumentScope owns one SourceCache across same-document operations; storage moves into SourceCache, replacing separate SourceStore/fork/parent/transfer machinery. Joining stays in SourceLoader alongside native execution and priority promotion, avoiding another cross-layer priority/cancellation protocol. The cache lifetime/storage simplification remains pending.

Explicit user requirement: when a source queued by eager thumbnail loading is selected, promote that existing joined load to foreground priority so selection does not wait behind the remaining low-priority queue. Preserve highest-active-consumer priority and independent waiter cancellation; do not create duplicate foreground development. Native calls already executing remain non-preemptible. The current loader-level joining already supports promotion and will stay there.

## Source module containment
Current user request: put core source acquisition in one module. Consolidate loader, provider/cache interface, storage, keys, persisted identity and metadata validation under src/source/ with a small re-exported API; private implementation files are not separate public module APIs. Keep editor-specific view/correction preparation outside. Preserve feature boundaries, source behavior and loader-owned joining/priority. This containment change does not implement the separate pending DocumentScope/cache-storage redesign.

Completed containment: src/source/mod.rs exports the provider, cache, loader, keys and storage types; implementation files are private. SourceKey is separate from LRU storage. Imports, diagnostics, integration tests and guide links use the unified module. Native loader scheduling and storage bodies are unchanged (apart from visibility restored after nesting). No GPU/prefetch implementation changed.

Verification: 216 focused debug and 216 release tests passed (2 opt-in diagnostics ignored in each); 26 source/DNG integration tests passed (2 opt-in diagnostics ignored), and 39 project persistence integrations passed; the no-default-feature core suite passed. Strict editor and UI-feature lib/test clippy, separate production editor check and project-only check passed; raw,project tests compile with the existing off-editor unused helpers. The added queued-selection regression runs through SourceCache with independent per-source fixture developments: selecting the last eager entry executes it ahead of the middle entry, once only. It fails when joined promotion is disabled (first-waiter priority only) and passes after restoration. Logs: .cache/source-module-*.log. Formatting/docs/diff checks and final inspection passed. This containment change is complete; next is the separate DocumentScope/cache-storage simplification. No push/PR.

## Limits / handoff
The whole editor-feature library debug attempt reached unrelated spatial-math tests before a five-minute timeout. Its one editor cancellation-fixture failure was fixed and included in the completed final editor-domain suite; no full-library pass is claimed. No Windows runtime, private throughput workload or native-window automation was performed.

The initial implementation is in six coherent local commits; the completed follow-up adds the actual root loader and immutable-session acquisition model. No push or PR performed; remote delivery requires an explicit integration request.
