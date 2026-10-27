# Async thumbnail and embedded-preview coordination

**Status: deferred follow-up.** Source acquisition already separates shared physical pixels, recorded photo exposure and operation ownership. This plan concerns the remaining thumbnail and embedded-preview orchestration.

## Current boundary

The [thumbnail worker](src/editor/thumbnails/production_worker.rs) performs disk/cache work and serial GPU processing on a dedicated thread. It manually polls pending source futures with a no-op waker and a 20 ms receive timeout, selecting ready jobs by priority. Those roles do not themselves establish a thread-affinity requirement.

Embedded-preview display and [exposure estimation](src/photo_preparation.rs) share extraction and JPEG/RGB decoding through [SourceCache](src/source/cache.rs). One worker initializes a per-file `OnceLock`; concurrent callers wait on that slot, occupying their worker threads. The cache mutex is released before extraction/decoding, and optional pixel retention is separate from the in-flight slot.

## Direction

Prefer ordinary async coordination with blocking disk, decoding, CPU work and current GPU waits dispatched through the existing [background pool](src/background.rs). Acquisition waits should suspend futures rather than occupy pool threads. Keep short cache lookup/publication locks synchronous.

For previews, run one extraction/decode job and let its consumers await the shared result. Shared work must remain owned independently of any one waiter: cancelling one consumer cannot disrupt another. Running native or blocking calls retain their inputs until completion.

Keep source acquisition concurrent so an eager waiter cannot prevent newly visible demand from joining and promoting queued native work. Preserve ready-job priorities, current serial GPU processing, native owner scheduling, cache-pressure backoff and prefetch suspension while changing coordination. No replacement global admission controller or general async-cache framework is needed for this direction.

Document ownership includes CPU/disk thumbnail demand even after its disposable GPU ticket retires. Document retirement drops that demand; independent exports keep their own owners and captured inputs. Avoid using cache presence or closure as an operation-lifetime signal.

## Checks to carry forward

Exercise completion, document retirement and shutdown with pending and running work. Verify selection promotes an existing eager load without duplicate development; zero optional retention preserves live leases; one preview consumer can leave while another completes. Check both the component and composed editor workflow without continuous painting. Assert lifecycle and publication outcomes rather than a particular polling interval or timer-speed improvement.
