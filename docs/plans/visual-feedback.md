# Thumbnail loading and resource follow-up

The [editor guide](../editor.md#fast-feedback-is-not-authoritative-data) explains the implemented preview/placeholder model. [The roadmap](../../PLAN.md) owns priority. Remaining work is broader real-use/resource and photographic acceptance, plus loading feedback design.

## Triaged thumbnail follow-up

- **Responsiveness:** exercise cold forward/reverse scrolling and grading while visible thumbnails develop. Exercise normal-budget roll residency, limit lowering, document/reference replacement and actual OS pressure/aggregate memory, not just UI-cache readiness. Additional development pools or cache tiers are not selected.
- **Camera-preview replacement:** inspect readability, colour and camera-framing-to-graded-composition transitions on new and saved rolls, including orientation/padding and both targets. Synthetic decode/ICC/EXIF tests do not establish photographic or broader-camera acceptance. Camera previews remain filmstrip-only; selected-view JPEG adoption and persistent JPEG caching are not selected.
- **Loading feedback:** design a spinner/status treatment through [operation feedback](completion.md#operation-feedback), distinguishing real available states and stopping repaint animation when idle.

## Next action and acceptance

Exercise physical scrolling/grading with cold camera placeholders and developed thumbnails, inspect replacement, and assess foreground latency and real memory pressure. Keep this separate from loading-state design.

Verify stale-work rejection, resize/scroll clipping and foreground isolation without changing measurement/export inputs or requiring full-RAW residency per thumbnail. Use [runtime phase logs](../diagnostics.md#investigating-responsiveness) for a concrete reported workload, not as physical-presentation acceptance or a general latency gate.
