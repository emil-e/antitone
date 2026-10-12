# Work plans

[The roadmap](../../PLAN.md) records current priorities. These work packages describe scope, decisions, acceptance and remaining implementation; completed packages summarize results and verification limits.

## Active packages

- [Source inspection and interaction follow-up](inspection-and-interaction.md): remaining cursor/New-tab/roll-workflow/layout/colour questions; the initial navigation/source-inspection slice is implemented.
- [Thumbnail follow-up](visual-feedback.md): reported clipping/invalidation/latency concerns; bounded thumbnails and responsive scopes are implemented.
- [Durable projects](durable-projects.md): recovery, source-change/relink UI and undo/redo; basic Save/Open is complete.
- [Later completion work](completion.md): post-implementation photographic acceptance, deferred dense guides, roll assistance, photographic controls/B&W, operation feedback, readability and release/export work. Alternate inversion bases are superseded, not an active implementation choice.

## Completed implementations

- [Remove full RAW hashing](completed/remove-raw-hashing.md): schema-5 removal, unchanged processing model 4 and cache-only path/stat freshness; metadata identity/relinking deferred, no old-project compatibility.
- [Documentation audiences](completed/documentation-audiences.md): human-first guides/current closures separated from unchanged archival checkpoint records.

- [Responsive rendering and thumbnail caching](completed/rendering-performance.md): shared GPU resources, pressure-responsive/persistent caches, bounded OpenMP/lazy diagnostics and combined geometry/scope integration; further acceptance and cache/fingerprint proposals are separate.
- [Documentation structure](completed/documentation-structure.md): workflow/cache reorganization and deliberate historical/private-input boundaries.
- [Settled-preview buffer limits](completed/preview-stability.md): adapter-supported limits and large-Fit regression; no speculative minimum chip/RAM target.
- [Basic editor Save/Open](completed/save-open.md): stable-ID ownership, transactional Open, atomic Save, File/Export menus and dirty protection.
- [Film-response integration](completed/film-response.md): historical response/source closure; model/compatibility clauses subsequently superseded by native-camera processing.
- [Roll-level flat-field correction](completed/flat-field-correction.md): user-accepted source correction, editable support, shared CPU/GPU map, corrected picks/thumbnails/exports and persistence.
- [Native-camera inversion replacement](completed/camera-inversion.md): model/schema 4 delivered on main, unchanged response/output mathematics and explicit camera identity; photographic limits retained.
- [Responsive vectorscope and waveform](completed/vectorscope-and-waveform.md): output-native positive-only proxy analysis, composition integration and bounded GPU/UI lifecycle; remaining acceptance/full-source work is separate.
- [Flip, crop and rotation](completed/flip-crop-rotate.md): shared geometry, constrained framing, coverage-aware measurements/reconstruction/exports and accepted crop UX; native-camera/flat-field integration delivered on main. Deferred acceptance/guides live in completion work.

Original execution/checkpoint narratives are preserved in [the implementation archive](../research/history/implementation/README.md); captured numerical logs are in [verification evidence](../research/evidence/README.md). Neither archive is a current implementation checklist.
