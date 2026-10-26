# Async source acquisition and execution simplification

## Agreed scope
- Async source acquisition returns existing immutable developed-source leases.
- Cache wraps loading, reusing SourceStore's weak/live and optional strong ownership; no parallel pixel or derived-artifact cache.
- Remove BlockingJobs and speculative cross-feature admission/thumbnail-idleness policy, preserving cancellation acknowledgement, stale-result rejection and worker-owned inputs through physical completion.
- Preserve bounded native execution and SDK ownership/synchronization. Leave GPU submission/configuration synchronization and GPU internals untouched.
- Address demonstrated responsiveness issues specifically; do not add replacement throttles.
- Separate worktree and coherent local commits. No push, PR or integration authorized.

## Acceptance
Consumers acquire sources asynchronously without native/cache-policy requests. Cache owns retention; analysis owns its intermediate measurements. Synchronous CPU work runs off the UI thread without a global foreground semaphore. Source/output correctness and actual cancellation/resource lifetimes remain protected.

## Current state
Based on fetched main 6cd71b3; refreshed upstream and guidance at the CPU-execution milestone, with no relevant upstream changes.

Completed commits:
1. Async SourceProvider, SourceLoader and SourceCache boundary, with ownership/reuse tests.
2. Editor/source/analysis/thumbnail consumers use providers. Native requests contain no stores, parent caches, retention choices or admission permits. Weak completed-result discovery covers the native-completion/cache-publication handoff without retaining pixels strongly.
3. BlockingJobs and editor/service plumbing removed. Shared blocking-pool execution captures panics without global admission. Thumbnail scheduling no longer reads foreground-pool idleness or polls for that permit. Lifecycle tests use explicit native/GPU barriers or a test-only pre-dispatch pause; serialization-policy assertions were removed or replaced with actual resource/publication checks.

4. Embedded-preview requests use their existing single worker directly; its extra semaphore was redundant with serial execution. Cancelled queued work checks its stop flag before IO. Removed the unused async-lock dependency; preview lifecycle tests protect cancellation and stale publication rather than a producer permit.

Prior architecture diagrams remain isolated in the architecture-walkthrough worktree.

## Verification and limits
- Editor production check and editor library/test clippy with warnings denied passed.
- Complete affected editor domain, native development, provider and blocking-execution tests: 766 passed, 9 existing opt-in diagnostics ignored.
- UI-test build passed; 17 composed activity, Analyze, export, dust-persistence, source-working and underlay tests passed on Metal.
- Embedded-preview worker checks: production editor check and all 18 embedded-preview unit/workflow tests passed after removing its redundant semaphore.
- Earlier focused acquisition, joining, cache/thumbnail and cancellation suites passed.
- Full editor-feature library attempt reached unrelated spatial-math tests before a five-minute timeout. Its one editor cancellation-fixture failure was corrected; the completed editor-domain run includes that case. No full-library pass claimed.
- No Windows runtime or native-window automation performed. GPU implementation files remain unchanged.
- Shared checkout-local native dependencies; offline builds, no downloads/system installs. Disposable artifacts are in .cache/.

## Review and finishing state
One independent fresh-context review examined the complete change, owning guides and test quality at case/assertion/fixture level. Its three actionable findings were confirmed and addressed:
- Native retirement now removes unwanted tasks from join discovery atomically before worker-local cleanup; a late replacement starts fresh.
- Thumbnail consumers drop pending provider futures when their document store closes, without using GPU-ticket changes to retire useful CPU/disk work.
- The private native throughput diagnostic clears completed-result discovery between measured schedules and asserts actual native-call counts, keeping its oracle leases from turning execution measurements into cache-hit timing.

Both retirement regressions fail with their respective fixes disabled and pass after restoration. Combined native/provider/thumbnail workflow checks: 37 passed, 1 opt-in diagnostic ignored. No private diagnostic workload was run. Owning documentation now explains the provider/cache boundary with one compact diagram; format/link checks pass. UI-feature clippy with warnings denied, separate production editor check, core tests and authored source/DNG integrations also passed before the final retirement fixes.

The final editor/provider/store/revision/cancellation/application regression sweep after these fixes passed: 784 tests, 9 opt-in diagnostics ignored. UI-feature clippy with warnings denied and the separate production editor check also passed.

## Remaining / next actions
1. Run the focused composed UI/source lifecycle checks in release; finish final format/documentation checks and record limits.
2. Commit owning documentation and the final plan state. Do not push or open a PR.
