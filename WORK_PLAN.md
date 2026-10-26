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

Prior architecture diagrams remain isolated in the architecture-walkthrough worktree.

## Verification and limits
- Editor production check and editor library/test clippy with warnings denied passed.
- Complete affected editor domain, native development, provider and blocking-execution tests: 766 passed, 9 existing opt-in diagnostics ignored.
- UI-test build passed; 17 composed activity, Analyze, export, dust-persistence, source-working and underlay tests passed on Metal.
- Earlier focused acquisition, joining, cache/thumbnail and cancellation suites passed.
- Full editor-feature library attempt reached unrelated spatial-math tests before a five-minute timeout. Its one editor cancellation-fixture failure was corrected; the completed editor-domain run includes that case. No full-library pass claimed.
- No Windows runtime or native-window automation performed. GPU implementation files remain unchanged.
- Shared checkout-local native dependencies; offline builds, no downloads/system installs. Disposable artifacts are in .cache/.

## Remaining / next actions
1. Inspect remaining execution/provider boundaries for stale coupling or clearly redundant admission, without expanding into GPU work or adding frameworks.
2. Update owning architecture/native/editor/development navigation documentation to the resulting design; keep it concise.
3. Run source-store/integration and final format/documentation checks, and any targeted verification needed by finishing edits.
4. Obtain one independent fresh-context review of the full change, explicitly covering test quality and fixture/control assumptions; assess findings with evidence and fix relevant defects.
5. Record final verification/limits and finish a coherent local commit history. Do not push or open a PR.
