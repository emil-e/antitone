# Working on Antitone

## Find your bearings

Read [PLAN.md](PLAN.md) for current priorities and blockers. Use [docs/README.md](docs/README.md) to find the guides relevant to your task—not to read everything.

Consult [architecture](docs/architecture.md) and [decision rationale](docs/decisions/README.md) when changing unfamiliar boundaries or consequential choices. Investigate disagreements between documentation and implementation.

## Work independently, within scope

Follow the global collaboration instructions. Roadmaps and plans provide context, not additional authorization.

For substantive changes, create a task branch and separate worktree from current main, or resume the existing worktree for that task. Keep task edits and verification artifacts in that worktree, isolated from other agents, and preserve unrelated changes. Small one-off edits and read-only investigations do not require a new worktree or task plan.

Maintain the task's implementation plan in `WORK_PLAN.md` at the worktree root. Record agreed scope, acceptance, remaining steps, blockers and next actions; keep it current and include it in checkpoint commits so another session can resume. This is task-local bookkeeping: `PLAN.md` owns project priorities, and `docs/plans/` owns durable unfinished scope and acceptance. Do not use the task plan as an execution archive.

At substantial milestones and before integration, refresh current guidance and inspect relevant changes on main.

Within delegated work, continue through routine implementation and verification checkpoints. Stop when you need a decision or have meaningful results to demonstrate—not merely because a batch finished.

Tie experiments to the requested outcome. Prefer a simple useful baseline, and add sophistication only when observed failures justify it.

## Follow contributor guidance

Follow the [development guide](docs/development.md) for engineering standards, documentation conventions and proportionate checks before handoff. Use the relevant [processing](docs/processing.md), [colour/output](docs/colour-and-output.md), [project compatibility](docs/projects.md) and decision guides for the boundaries being changed.

For editor work, follow [shared presentation](docs/editor.md#build-ui-from-shared-presentation) and the [tooltip/contextual-help policy](docs/editor.md#tooltips-and-contextual-help). Verify the component and its composed feature.

Write `docs/` for human contributors. Keep agent-only execution instructions here or in skills.

## Build and deliver

Use the normal [contributor build workflow](README.md#build-and-run), including shared native setup. Do not maintain an agent-only bootstrap or Cargo wrapper.

Use `.cache/` for disposable experiments, dependencies and generated outputs. Maintained tools belong in `scripts/` or `native/`. Do not automatically promote downloaded material into maintained source.

Samples are private, read-only, ignored and never committed. System installations and automation of other applications require explicit authorization; respect environment approval requirements for setup and downloads.

Before integrating substantial changes, consider an independent, fresh-context agent review against the contributor standards and relevant topic guides, proportionate to the change's breadth and risk. Review supplements verification; assess findings with evidence rather than treating the review as an approval gate.

An explicit wrap-up request authorizes local delivery of agreed work:

1. Checkpoint-commit as needed and rebase onto current main, preserving unrelated work.
2. Inspect semantic interactions, update affected documentation and verify the integrated result proportionally. Transfer lasting knowledge and any remaining scope from `WORK_PLAN.md` into the owning guides or topic plans, then remove `WORK_PLAN.md` before final delivery.
3. Commit the agreed result and fast-forward main. If already on main, commit there; do not rewrite unrelated history.
4. Remove the task's worktree once its work is integrated and it is no longer needed, including disposable checkout-local caches and generated outputs. Before removal, verify integration and check tracked, untracked and ignored files for unique work or retained artifacts. Preserve active worktrees and other agents' work; ask before discarding anything not integrated or explicitly abandoned. Do not force removal merely because the branch is merged.
5. Report the main commit, checks, worktree cleanup and remaining limits.

Push only when separately requested.
