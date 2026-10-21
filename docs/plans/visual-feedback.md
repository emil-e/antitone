# Thumbnail loading and resource follow-up

The [editor guide](../editor.md#fast-feedback-is-not-authoritative-data) explains the implemented preview/placeholder model. [The roadmap](../../PLAN.md) owns priority. Remaining work is broader real-use/resource and photographic acceptance. Available camera previews and previous accepted grades remain visible during replacement; recent JPEG retention and processed-card versions share bounded image-cache ownership.

## Triaged thumbnail follow-up

- **Responsiveness:** exercise cold forward/reverse scrolling and grading while visible thumbnails develop. Exercise normal-budget roll residency, limit lowering, document/reference replacement and actual OS pressure/aggregate memory, not just UI-cache readiness. Additional development pools or cache tiers are not selected.
- **Camera-preview replacement:** inspect readability, colour and camera-framing-to-graded-composition transitions on new and saved rolls, including orientation/padding and both targets. The reported Sony source's padded tiny thumbnail is avoided by preferring a sufficient-resolution bounded preview; cameras with only padded thumbnails or genuinely different preview framing still need separate evaluation. Synthetic decode/ICC/EXIF tests do not establish photographic or broader-camera acceptance. Camera previews remain filmstrip-only; selected-view JPEG adoption and persistent JPEG caching are not selected.
- **Loading feedback:** inspect the shared static status treatment and camera badge during cold loading, grade changes and scroll reversal. Available camera previews should not look like empty loading cards; previous accepted grades must remain distinguishable from completed replacements. Further feedback design belongs with [operation feedback](completion.md#operation-feedback).

## Next action and acceptance

Exercise physical scrolling/grading with cold camera placeholders and developed thumbnails, inspect replacement, and assess foreground latency and real memory pressure. Keep this separate from loading-state design.

Verify stale-work rejection, resize/scroll clipping and foreground isolation without changing measurement/export inputs or requiring full-RAW residency per thumbnail. Use [runtime phase logs](../diagnostics.md#investigating-responsiveness) for a concrete reported workload, not as physical-presentation acceptance or a general latency gate.
