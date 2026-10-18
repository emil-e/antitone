# Cache freshness is not photograph identity

A RAW can be intentionally edited without becoming a different photograph. Using full-file hashes to reject changed sources would add I/O and enforce byte immutability, rather than the development-compatibility check used here.

We use stable IDs for settings ownership, paths for loading and camera/development compatibility for deciding whether a grade can be processed. None proves capture identity. Source relinking or stronger identity policy needs its own workflow requirement.

Disposable thumbnail caches ask a narrower question: might these generated pixels be stale? Cheap file revisions can invalidate reuse without invalidating calibration. A revision must be captured before development; attaching later file stats to old pixels would falsely describe their freshness.

File stats can miss changes when preserved, so this is not immutable-byte proof. Generated-pixel checksums protect cache data, not RAW identity. Saved correction maps remain accepted results rather than being regenerated because a reference's bytes changed.

[Projects](../projects.md#compatibility-is-about-the-processing-basis) and [editor caches](../editor.md#responsiveness-and-optional-caches) explain the boundaries. [Disk proxies](../../src/thumbnail_disk.rs) own key and corruption checks.
