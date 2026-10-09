# Working on Antitone

## Start with the right context

1. Read [PLAN.md](PLAN.md) for current focus, work order and blockers.
2. Use [docs/README.md](docs/README.md) to find the relevant plan and contracts. Read [architecture](docs/architecture.md) when tracing unfamiliar boundaries.
3. Read relevant [accepted decisions](docs/decisions/README.md) before proposing changes to those choices. Consult research selectively, not as a mandatory reading list.
4. Inspect the actual code/tests before editing. Conversation summaries are hints, not a substitute for repository state. If code and a contract disagree, investigate rather than silently declaring either correct.

The latest user request determines authorization. A roadmap entry, accepted ADR or previously approved broad plan does not authorize unrelated implementation, commits, pushes, global installs or remote actions. Confirm ambiguous/risky scope; keep routine local changes focused.

## Documentation roles

- `PLAN.md`: one short current-status/ordered-roadmap record. Link details; do not accumulate session transcripts, repeated test counts or completed step lists here.
- `docs/architecture.md`: the implemented system and its boundaries. Explicitly distinguish foundations from wired features and plans.
- `docs/contracts/`: current behavior/invariants and compatibility requirements. Update the relevant contract when an authorized change changes it.
- `docs/decisions/`: selective ADRs explaining consequential choices and tradeoffs. Do not write one for every implementation detail or duplicate full specifications. Preserve accepted rationale; explicitly supersede an ADR when an approved replacement changes the decision.
- `docs/plans/`: substantial work packages. Record goal, scope/non-goals, approval/open questions, steps, acceptance, progress/verification and exact next action. Small changes need no plan file. Mark completed plans and move them to `completed/`; root roadmap owns cross-package status/order.
- `docs/research/`: flexible knowledge base. No compulsory template. Separate observations from hypotheses, link sources/evidence, identify important limitations and flag superseded findings that could mislead. Accepted conclusions belong in contracts/ADRs; experiments are not implementation instructions.
- `docs/research/history/`: old plans/checkpoints and mockup. Never resume from their unchecked steps. Use them only for historical context.

## Checkpoints and handoff

Update the active plan at meaningful verified checkpoints and before handoff/context exhaustion. Record changes, checks actually run, unresolved failures/limits and the next concrete action. Update the root roadmap only when current focus/status/order/blockers change. Keep shared facts in one canonical document and link them elsewhere.

Verification is proportional: run the narrow relevant checks, inspect the change and repair affected documentation links. Label recorded test counts/timings as snapshots, with environment and reproduction; do not claim a fresh test run or hardware acceptance from an old log. Use [the runbook](docs/build-and-probes.md), not historical command collections.

## Engineering boundaries

- macOS/Apple Silicon first; M1 Pro/16 GB is the conservative baseline, not established by M5 results. macOS 14 and physical-monitor acceptance remain separate gates.
- Reuse pinned LibRaw development. No custom demosaicer, camera-specific repair, CPU interactive fallback, approximate ACES replacement or weakened numerical gate.
- Preserve signed/HDR AP1 working data, AP0 calibration/inversion and AP0 EXR; ACES mathematics are unchanged. Preview/TIFF targets are matching 100-nit sRGB and Display P3, with managed extended-sRGB SDR presentation.
- Dmin/Dmax are RGB-only values. Picker context is transient; source fingerprints and development compatibility are separate concerns. Invalid/cancelled picks and rejected edits change no processing state.
- Existing model-2 grades retain original behavior. New grades use model 3 toe 2.5/shoulder off; changing an existing grade requires explicit opt-in.
- One production RGB16 source path: 8 bytes/pixel, extra precision in temporaries/small constants, not a wide retained source cache. Historical research snapshots are not supported upload paths or fallbacks.
- Samples are read-only, ignored and not committed. Do not automate Lightroom/Resolve or pursue flat-field correction without a new request. Do not silently add a pedestal/clamp or change development to improve inspection.

## Cache and durable research

`.cache/` is disposable storage, not the only home for authored source, decisions or irreplaceable evidence. Keep reusable tools in `scripts/`/`native/` or research tools alongside their guide; preserve selected compact results in research evidence. Put dependencies, build products, generated/private photographic outputs and downloaded third-party inputs in ignored local storage. Never automatically promote user photographs, copied upstream code or copyrighted documents. See [research tools/cache policy](docs/research/tools/README.md).

Run commands from the repository root through `./scripts/cargo.sh`. No system/Homebrew installations, sample edits, commits or pushes are authorized by this file.
