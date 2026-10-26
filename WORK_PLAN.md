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

## Limits / handoff
The whole editor-feature library debug attempt reached unrelated spatial-math tests before a five-minute timeout. Its one editor cancellation-fixture failure was fixed and included in the completed final editor-domain suite; no full-library pass is claimed. No Windows runtime, private throughput workload or native-window automation was performed.

Implementation and local handoff are complete. Six coherent local commits cover foundation, consumer migration, global admission removal, embedded-preview simplification, reviewed retirement fixes and owning documentation. No push or PR performed. Await an explicit integration request before remote delivery.
