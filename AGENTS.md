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

## Preserve the product's guarantees

Follow the relevant [processing](docs/processing.md), [colour/output](docs/colour-and-output.md), [project compatibility](docs/projects.md) and decision guides. Code and tests own exact defaults, formats and numerical gates.

Do not weaken checks or alter development behavior to improve an experiment's appearance. Software agreement is not proof of photographic quality, physical interaction or monitor correctness.

Distinguish established requirements from implementation choices and experimental hypotheses. Keep source storage compact; judge additional caching by measured benefit, memory cost and lifetime rather than assuming all retained intermediates are forbidden.

## Build on the shared UI

Follow [the editor guide](docs/editor.md#build-ui-from-shared-presentation) and existing components for layout, controls, viewer tools, dialogs and task feedback. Reuse their visual and interaction language.

Follow the editor's [tooltip and contextual-help policy](docs/editor.md#tooltips-and-contextual-help): no tooltip is the default for self-explanatory labelled controls. Keep useful help brief and local; do not put pipeline explanations or unrelated gestures in hover text.

Presentation receives state and returns actions; domain semantics and transactions remain with their owners. Extend shared components for real supported variants. Keep genuinely one-off UI local without copying an existing family or creating speculative abstractions.

Verify both the component and its composed feature.

## Keep source readable

Follow the neighboring code: separate declarations and logical phases with blank lines, name values by role outside conventional mathematics, and expand dense control flow or macros even when the formatter accepts them. Inspect complete touched functions and run `python3 scripts/check-format.py` before finishing; formatter success is not a readability review.

Follow the [code guidelines](docs/code-guidelines.md) for comments, tests and prose: preserve current contracts and useful rationale, not change narration or obvious restatements. Tests should protect meaningful requirements and invariants, not incidental implementation choices or facts guaranteed by their own setup.

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
2. Inspect semantic interactions, update affected documentation and verify the integrated result proportionally. Transfer lasting knowledge and any remaining scope from `WORK_PLAN.md` into the owning guides or topic plans, then remove `WORK_PLAN.md` before final delivery.
3. Commit the agreed result and fast-forward main. If already on main, commit there; do not rewrite unrelated history.
4. Remove the task's worktree once its work is integrated and it is no longer needed, including disposable checkout-local caches and generated outputs. Before removal, verify integration and check tracked, untracked and ignored files for unique work or retained artifacts. Preserve active worktrees and other agents' work; ask before discarding anything not integrated or explicitly abandoned. Do not force removal merely because the branch is merged.
5. Report the main commit, checks, worktree cleanup and remaining limits.

Push only when separately requested.
