# Dust release-to-result latency

## Agreed scope and delivery

Reduce the wait after committing a heal; preserve the frozen search region, eligibility, exact f64 scores, one-percent/epsilon membership, tie breaks, donor metadata and repaired f32 bits. Preserve signed/HDR AP1, masked-value independence, independent repairs and existing source/mask budgets. No quality tuning, shortlist, solver change, persistence/output work or push.

The user authorized the selection-hover pointer, safe held-stroke fix, disposable GPU prototype, then editor integration. CPU/UI baseline is checkpoint `324d436` on `perf/dust-release-latency`. The user now authorizes local integration and task-worktree cleanup, without temporary tracing/debugging. No push.

## Current implementation

- Exact CPU search pruning, action-sensitive selection/brush/donor cursors and shape-independent mask admission are checkpointed. Over-budget held gestures visibly reject and discard on release rather than committing a truncated prefix.
- The existing backend optionally screens donors on the shared GPU using integer lower bounds. Exact CPU seeds and survivors retain score/ranking/reconstruction ownership. Seed minima retain their candidates, including exact-zero ranking state.
- Only a lazy immutable pipeline survives jobs. Packed feature/weight/result buffers are job-local, explicitly admitted against a conservative 64 MiB payload/device bound. Slices have a point-visit bound and shrink after long waits; submission and job-specific completion use the existing gate/owner. No new full-resolution retained photograph or worker.
- Per-request/source cancellation retires superseded work without failed-heal memoization or consuming a newer acknowledgment. Navigation and downstream grades retain compatible work. The authored-RAW backend test interleaves offscreen presentation between actual slices, retires the source and completes subsequent work bit-exactly.

## Continued-delay diagnosis

The user still saw several seconds with `cargo app` in this worktree; its alias is confirmed `--release`.

1. A supported 128-pixel wide diagonal (59,228 marked pixels) reproduced ~6 s even with GPU screening: ~2.8 s all-pairs distance weighting, ~2.1 s exact CPU verification and ~0.6 s filling. A separable integer squared-distance transform preserves the original f64 distance bits and reduces that phase to ~1.5 ms, with scalar fallback beyond the exact-integer/workspace guarantee. Exact paired weight-ceiling sums tighten the screening denominator where missing source context warrants the extra pass; otherwise it retains the cheaper full-target bound. This case is now ~1.4–1.8 s core time across repeated runs with identical metadata/pixels. These are developed-AP1 timings, not native release-to-visible measurements.
2. Dust replies previously did not wake an idle UI. Results could wait for unrelated input. Requests now carry the egui repaint context, signalled after terminal message enqueue. The backend regression fails without this wake and passes for cancellation and success without pointer input.
3. Temporary request/phase attribution established native preparation/queueing/reconstruction costs and supported the user's “much faster” feedback. It is removed for delivery, including the diagnostic module, all call sites and the opt-in instructions. Retain numerical/lifecycle regressions and their cfg(test)-only submission observer; the production slice clock controls bounded scheduling, not diagnostics.

The numerical prototype review and integration review were separate. The former caught the seeded-zero bug, now covered by a reproducer; the latter identified missing shared-device coverage, now addressed. A third, distinct review of EDT/paired-ceiling mathematics found no defects; its boundary-test gaps are addressed. Do not repeat reviews merely because their findings were fixed.

## Verification and next actions

- Checks pass: 47 default tests; 593 UI/library tests (34 explicit ignores); 48 release Dust tests; four release reconstruction tests; 77 private GPU metadata/pixel comparisons plus the wide case; 154 comparisons with the original pre-optimization CPU core; 47 Python tests; editor and UI/test-target Clippy; formatting/diff checks. Release editor and reference probe are rebuilt. The probe retains only the noted pre-existing export warning. Task binary: `target/release/antitone` (Oct 16 16:01; 21,181,584 bytes).
- User retested and reports “much faster.” Their private `.cache/dust-latency.log` records six successful strokes: commit→repaired-view selection 0.35–2.15 s (median 1.11 s), enqueue→worker <1 ms, original AP1 17–37 ms, reply→UI receipt <11 ms and regional reconstruction/acquisition 13–28 ms. Matching dominates the remaining wait; the slowest stroke has ~1.48 s of search outside screening preparation/GPU work. One support found no eligible donor on three attempts. Mask editing also produced 29 cancelled draft evaluations (~0.63 s aggregate worker time); no optimization to preview scheduling is authorized by trace inspection alone. These measurements are not physical scanout or broad native acceptance. Cold initialization, aggregate/in-flight resources and monitor acceptance remain open.
- Integration is now authorized: remove temporary tracing (done), checkpoint/rebase onto current main, resolve semantic interactions, verify the integrated result, transfer remaining acceptance to the owning plan, remove this task plan, commit/fast-forward main and audit/clean up this worktree. At delivery rebase onto current main and preserve its quiet viewer/scopes feedback, contextual-help policy and newer shared tool workflows; do not restore removed `dust_updating` presentation behavior. The tested branch base is `98e1e95` plus checkpoint `324d436`, not newer main. Current guidance/main interactions were refreshed through `e2f6912`: Auto-crop changes tool mutual exclusion; bounded original-tile batching changes shared flat-field/local acquisition. Inspect both at eventual integration; neither was merged into this branch.

## Continuity and limits

The owning guides document numerical rationale, workflow and tracing. Disposable probes/corpora are under `.cache/gpu-match-03/`, `.cache/bench/`, `.cache/integration-large-masks/` and `.cache/native-delay-*`; original photographic evidence remains in the separate dust-experiments worktree. Keep all private inputs read-only/ignored and audit unique evidence before any cleanup. Main's unrelated dirty guides and `.pi/` have not been touched.

The non-editor reference probe retains main's existing unused `BlockOrder::Spatial2x2` warning in unchanged `src/export.rs`; do not suppress it or expand into export changes.
