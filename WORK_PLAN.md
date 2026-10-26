# Async source acquisition and execution simplification

## Agreed scope
- Async source acquisition at the root, returning existing immutable developed-source leases.
- Cache as a wrapper above the loader, reusing SourceStore's weak/live and optional strong ownership; no parallel or derived-artifact cache.
- Remove BlockingJobs and speculative cross-feature admission/thumbnail idleness policies where possible, preserving cancellation acknowledgement, stale-result rejection and owned inputs through physical completion.
- Keep bounded native execution and necessary SDK ownership/synchronization behind acquisition. Leave GPU submission/configuration synchronization and GPU internals untouched.
- Respond to observed responsiveness issues specifically rather than adding replacement throttles.
- Work in this separate worktree; create coherent local commits. No push/PR/integration authorized.

## Acceptance
Consumers acquire sources asynchronously without constructing native/cache-policy requests. Cache retention is determined in the cache layer. Analysis owns its own intermediate data. CPU blocking work runs off the UI thread without a global foreground semaphore. Existing native/GPU safety and source/output correctness remain covered by focused tests.

## Intended commit sequence
1. Introduce the async source-provider/loader and cache boundary with focused ownership/reuse tests.
2. Route editor/source/analysis/thumbnail consumers through the provider; remove producer cache policy coupling.
3. Remove BlockingJobs and its cross-feature admission/idleness dependencies; update tests to protect actual lifecycle behavior instead of serialization policy.
4. Update owning documentation and complete focused/broader verification and independent review proportionate to this cross-cutting change.

Keep each implementation commit buildable when feasible; update this plan in checkpoints. Adjust boundaries to actual code rather than adding machinery for this sequence.

## Current state / next
Foundation implemented on fetched main 6cd71b3: public async SourceProvider, native SourceLoader and SourceCache wrapper over the existing SourceStore. SourceKey can describe authoritative loads without usable file stats. Cache hits/publication run off the UI thread. Native key-only demands join independently of the existing document-bound legacy path. Existing editor consumers are not migrated yet; legacy producer cache policy is intentionally still present until the next slice.

Prior architecture diagrams remain isolated in their existing docs worktree. Next: migrate source, analysis and thumbnail consumers onto the provider boundary, then remove native producer cache/store coupling.

## Verification / limits
Foundation checks: cargo check --locked --offline --features editor passed. Seven focused source_provider tests passed, including independent waiter cancellation, zero-retention live reuse, retained keepalive, candidate parent reuse, cache closure and requested development verification. An initial test incorrectly asked the SDK's continuous camera source for integer-only codes; corrected it to inspect camera samples, preserving the representation-independent comparison. git diff --check passes.

Use shared native build setup and existing format tools; no downloads/system installs. Keep artifacts under this worktree's .cache. Native/Metal checks will be run locally, not inferred from hosted CI.
