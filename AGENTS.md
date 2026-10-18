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

An independent, fresh-context agent review against the [development guide](docs/development.md) and relevant topic guides is required before integrating substantial changes. Keep its depth proportionate to the change's breadth and risk. Review supplements verification; assess findings with evidence rather than treating the review as an approval gate.

An explicit wrap-up/integration request authorizes local delivery of agreed work. Push only when separately requested.
