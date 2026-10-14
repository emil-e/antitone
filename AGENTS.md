# Working on Antitone

## Find your bearings

Read [PLAN.md](PLAN.md) for current priorities and blockers. Use [docs/README.md](docs/README.md) to find the guides relevant to your task—not to read everything.

Consult [architecture](docs/architecture.md) and [decision rationale](docs/decisions/README.md) when changing unfamiliar boundaries or consequential choices. Investigate disagreements between documentation and implementation.

## Work independently, within scope

Follow the global collaboration instructions. Roadmaps and plans provide context, not additional authorization.

Use a separate worktree for substantive changes. Keep work isolated from other agents and preserve unrelated changes. At substantial milestones and before integration, refresh current guidance and inspect relevant changes on main.

Within delegated work, continue through routine implementation and verification checkpoints. Stop when you need a decision or have meaningful results to demonstrate—not merely because a batch finished.

Tie experiments to the requested outcome. Prefer a simple useful baseline, and add sophistication only when observed failures justify it.

## Preserve the product's guarantees

Follow the relevant [processing](docs/processing.md), [colour/output](docs/colour-and-output.md), [project compatibility](docs/projects.md) and decision guides. Code and tests own exact defaults, formats and numerical gates.

Do not weaken checks or alter development behavior to improve an experiment's appearance. Software agreement is not proof of photographic quality, physical interaction or monitor correctness.

Distinguish established requirements from implementation choices and experimental hypotheses. Keep source storage compact; judge additional caching by measured benefit, memory cost and lifetime rather than assuming all retained intermediates are forbidden.

## Build on the shared UI

Follow [the editor guide](docs/editor.md#build-ui-from-shared-presentation) and existing components for layout, controls, viewer tools, dialogs and task feedback. Reuse their visual and interaction language.

Presentation receives state and returns actions; domain semantics and transactions remain with their owners. Extend shared components for real supported variants. Keep genuinely one-off UI local without copying an existing family or creating speculative abstractions.

Verify both the component and its composed feature.

## Keep documentation useful

Write for prospective contributors: explain the problem, mental model, rationale and important guarantees. Leave implementation inventories and exhaustive detail to code and tests.

Maintain one explanation per topic. Extend the owning guide before adding a page. Keep workflow instructions here, not in the human README.

Plans describe unfinished scope, acceptance and next actions—not execution history. Remove completed steps and finished plans after transferring useful current knowledge. Delete obsolete tools and superseded material; do not create archives or resurrect old designs without a historical-investigation request.

Update affected guides and links when behavior changes. Update the roadmap only when priorities, status or blockers change.

## Build and deliver

Use the normal [contributor build workflow](README.md#build-and-run), including shared native setup. Do not maintain an agent-only bootstrap or Cargo wrapper.

Use `.cache/` for disposable experiments, dependencies and generated outputs. Maintained tools belong in `scripts/` or `native/`. Do not automatically promote downloaded material into maintained source.

Samples are private, read-only, ignored and never committed. System installations and automation of other applications require explicit authorization; respect environment approval requirements for setup and downloads.

An explicit wrap-up request authorizes local delivery of agreed work:

1. Checkpoint-commit as needed and rebase onto current main, preserving unrelated work.
2. Inspect semantic interactions, update affected documentation and verify the integrated result proportionally.
3. Commit the agreed result and fast-forward main. If already on main, commit there; do not rewrite unrelated history.
4. Report the main commit, checks and remaining limits.

Push only when separately requested.
