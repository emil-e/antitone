# Working on Antitone

## Start with the right context

1. Read [PLAN.md](PLAN.md) for current focus, work order and blockers.
2. Use [docs/README.md](docs/README.md) to find the relevant plan and contracts. Read [architecture](docs/architecture.md) when tracing unfamiliar boundaries.
3. Read relevant [accepted decisions](docs/decisions/README.md) before proposing changes to those choices. Consult research selectively, not as a mandatory reading list.
4. Inspect the actual code/tests before editing. Conversation summaries are hints, not a substitute for repository state. If code and a contract disagree, investigate rather than silently declaring either correct.

The latest user request determines authorization. A roadmap entry, accepted ADR or previously approved broad plan does not authorize unrelated implementation, commits, pushes, global installs or remote actions. Confirm ambiguous/risky scope; keep routine local changes focused.

## Documentation audiences

- `README.md` is for humans: project introduction, prerequisites, normal build/run/check commands and contributor navigation. Do not require readers to consult `AGENTS.md` or include agent authorization, checkpoint or handoff procedures there.
- `AGENTS.md` is for agents: instruction discovery, authorization boundaries, execution workflow and documentation-maintenance rules belong here. Keep it discoverable at the repository root; do not duplicate these procedures across human-facing documents.
- Shared documentation is project knowledge, not an execution journal. Keep architecture, contracts, decisions, research findings and reproducible verification useful to human contributors. Human-facing navigation should lead to these documents, not through agent instructions. Specialist probes are not agent-only merely because agents use them.
- Plans retain task-specific scope, decisions, progress, verification limits and next actions. Keep generic agent protocol, chronological session/worktree narratives, repeated permission/no-push notes and details of unrelated local edits out of current plans and roadmaps. Completed plans should be concise closure records linking to relevant evidence.
- Current contracts describe behavior and invariants, not historical test counts, compiler transcripts or smoke timings. Put meaningful measurements and acceptance evidence in the appropriate research/verification record and link rather than repeat them. Archive useful execution history and raw logs separately from current guidance; do not discard durable research or provenance merely because an agent produced it.
- Clearly separate supported tools/current reproduction commands from archived experiments and obsolete commands. Historical records are evidence, not an alternative setup path or active work queue.

## Documentation roles

- `PLAN.md`: one short current-status/ordered-roadmap record. Link details; do not accumulate session transcripts, repeated test counts or completed step lists here.
- `docs/development.md`: everyday and feature-specific contributor checks; `docs/build-and-probes.md`: specialist numerical/reference, RAW, export and presentation verification. Both are human-useful guides, not agent-only procedures.
- `docs/architecture.md`: the implemented system and its boundaries. Explicitly distinguish foundations from wired features and plans.
- `docs/contracts/`: current behavior/invariants and compatibility requirements. Update the relevant contract when an authorized change changes it.
- `docs/decisions/`: selective ADRs explaining consequential choices and tradeoffs. Do not write one for every implementation detail or duplicate full specifications. Preserve accepted rationale; explicitly supersede an ADR when an approved replacement changes the decision.
- `docs/plans/`: substantial work packages. Record goal, scope/non-goals, approval/open questions, steps, acceptance, progress/verification and exact next action. Small changes need no plan file. Mark completed plans and move them to `completed/`; root roadmap owns cross-package status/order.
- `docs/research/`: flexible knowledge base. No compulsory template. Separate observations from hypotheses, link sources/evidence, identify important limitations and flag superseded findings that could mislead. Accepted conclusions belong in contracts/ADRs; experiments are not implementation instructions.
- `docs/research/history/`: old plans/checkpoints and mockup. Never resume from their unchecked steps. Use them only for historical context.

## Checkpoints and handoff

Update the active plan at meaningful verified checkpoints and before handoff/context exhaustion. Record changes, checks actually run, unresolved failures/limits and the next concrete action. Update the root roadmap only when current focus/status/order/blockers change. Keep shared facts in one canonical document and link them elsewhere.

Verification is proportional: run the narrow relevant checks, inspect the change and repair affected documentation links. Label recorded test counts/timings as snapshots, with environment and reproduction; do not claim a fresh test run or hardware acceptance from an old log. Use [development checks](docs/development.md) and [specialist verification](docs/build-and-probes.md), not historical command collections.

## Wrap-up procedure

When the user requests wrap-up of agreed work, that request authorizes the following local delivery procedure. An ordinary implementation request does not automatically authorize commits or main integration; this procedure never authorizes a push.

1. **Rebase and integrate onto current main.** Checkpoint-commit the agreed work first if needed, preserving unrelated working changes and other worktrees. Inspect new main changes and their relevant code, contracts and tests. Integrate their behavior and boundaries with the feature, including interactions that Git merges without conflicts; merely resolving conflict markers or making the code compile is insufficient. Preserve main's accepted semantics and verify that existing and new features work together. Discuss material new scope/design decisions before expanding the work.
2. **Close and update documentation before the final commit.** Update affected contracts, architecture, roadmap and plans; move completed implementation plans to `docs/plans/completed/` and repair links. Track deferred work and unperformed acceptance separately without claiming it passed. Keep mixed packages active only for genuinely unfinished work.
3. **Verify the integrated result.** Run proportional checks after rebase, including relevant combined-behavior regressions, and inspect the final code/documentation diff. Report unresolved failures and verification limits rather than masking them.
4. **Commit and deliver locally to main.** Commit only the agreed changes and fast-forward main to the verified result, preserving unrelated local changes. If the work is already on main, commit there; do not rewrite unrelated history.
5. **Report delivery.** State the main commit, checks actually run and remaining limits. Do not push unless separately requested.

## Engineering boundaries

- macOS/Apple Silicon first; no minimum chip/RAM requirement is established. Resource policy should follow observed problems, not speculative hardware constraints. macOS 14 and physical-monitor acceptance remain separate checks.
- Reuse pinned LibRaw development. No custom demosaicer, camera-specific repair, CPU interactive fallback, approximate ACES replacement or weakened numerical gate.
- Preserve native-camera calibration/inversion, signed/HDR AP1 working data and AP0 EXR; AP0 is otherwise internal to unchanged ACES. Native-camera model/matrix identity must match the roll's retained basis. Preview/TIFF targets are matching 100-nit sRGB and Display P3, with managed extended-sRGB SDR presentation.
- Dmin/Dmax are RGB-only values. Picker context is transient; thumbnail-cache freshness and development compatibility are separate concerns; do not reintroduce RAW content hashing or capture identity without a new requirement. Invalid/cancelled picks and rejected edits change no processing state.
- Camera processing remains model 4; project schema 5 removes fingerprints outright with no old-schema compatibility. Old schemas/AP0 grades reject; do not add migration/legacy inversion branches. New grades retain toe 2.5/shoulder off; classic toe-1 response also uses the camera model.
- One production RGB16 source path: 8 bytes/pixel, extra precision in temporaries/small constants, not a wide retained source cache. Historical research snapshots are not supported upload paths or fallbacks.
- Samples are read-only, ignored and not committed. Do not automate Lightroom/Resolve or pursue flat-field correction without a new request. Do not silently add a pedestal/clamp or change development to improve inspection.

## Cache and durable research

`.cache/` is disposable storage, not the only home for authored source, decisions or irreplaceable evidence. Keep reusable tools in `scripts/`/`native/` or research tools alongside their guide; preserve selected compact results in research evidence. Put dependencies, build products, generated/private photographic outputs and downloaded third-party inputs in ignored local storage. Never automatically promote user photographs, copied upstream code or copyrighted documents. See [research tools/cache policy](docs/research/tools/README.md).

Use the same build path as human developers: run ordinary `cargo` commands from the repository root, with `rust-toolchain.toml` selecting Rust and `.cargo/config.toml` selecting Apple Clang. Follow [the README](README.md) for prerequisites, `./scripts/setup-native.sh`, `cargo build-app` and `cargo app`; use [development checks](docs/development.md) for routine verification and [the probe guide](docs/build-and-probes.md) for specialist checks. Do not maintain a separate agent bootstrap or Cargo wrapper. Environment overrides required by a sandbox are local execution details, not a second supported workflow. Setup/downloads require permission when the environment requires it; no system/Homebrew installations or sample edits are authorized by this file. Local commits/main integration require a user request, including an explicit wrap-up request as defined above; pushes require separate authorization.
