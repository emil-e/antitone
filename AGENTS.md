# Working on Antitone

## Find your bearings

Use [docs/README.md](docs/README.md) to find the guides relevant to your task—not to read everything.

Consult [architecture](docs/architecture.md) when changing unfamiliar boundaries or consequential choices. Investigate disagreements between documentation and implementation.

## Requirements and design choices

Only an explicit requirement from the user establishes a requirement. Suggestions, illustrative examples, passing agreement, existing code, tests, documentation and previous agent decisions do not establish one. Do not describe a choice as user-required without that provenance.

Distinguish product intent, current implementation and technical constraints. A product goal does not by itself require a particular algorithm, precision, library or architecture. Explain constraints in terms of the design they depend on, not as user mandates. A documented or tested implementation is not frozen: reconsider it when its drawbacks warrant, within the delegated scope, and explain the tradeoffs rather than invoking “policy” as a veto.

Ask before removing or weakening a product-, value- or philosophy-level commitment, including one whose user provenance is uncertain. Preserve it pending clarification; uncertainty is not permission to delete it. This does not make implementation prescriptions such as a numeric representation or fixed algorithm into product requirements.

When writing docs and comments, describe current choices and their rationale as such. Do not turn proposals, experimental settings or task-local restrictions into standing requirements. Tests and decision records are evidence about the current design, not independent authority for it.

## Work independently, within scope

Follow the global collaboration instructions. Roadmaps and plans provide context, not additional authorization.

For substantive changes, fetch first and branch from the latest `origin/main` — merges happen on GitHub, so a local `main` ref may be behind — and work in a separate worktree under `.worktrees`, or resume the existing worktree for that task. Keep task edits and verification artifacts in that worktree, isolated from other agents, and preserve unrelated changes. Small one-off edits and read-only investigations do not require a new worktree or task plan.

Maintain the task's implementation plan in `WORK_PLAN.md` at the worktree root. Record agreed scope, acceptance, remaining steps, blockers and next actions; keep it current and include it in checkpoint commits so another session can resume. When a task runs directly in main rather than a task worktree, keep `WORK_PLAN.md` untracked instead: checkpoint commits there become shared history, and a session plan is not part of it.

At substantial milestones and before integration, refresh current guidance and inspect relevant changes on main.

Within delegated work, continue through routine implementation and verification checkpoints. Stop when you need a decision or have meaningful results to demonstrate—not merely because a batch finished.

Tie experiments to the requested outcome. Prefer a simple useful baseline, and add sophistication only when observed failures justify it.

## Follow contributor guidance

Follow the [development guide](docs/development.md) for engineering standards, documentation conventions and proportionate checks before handoff. Use the relevant [processing](docs/processing.md), [colour/output](docs/colour-and-output.md) and [project compatibility](docs/projects.md) guides to understand the boundaries being changed, not to treat their implementation choices as immutable requirements.

Before editing any guide or README, including the root `README.md`, follow the [documentation skill](.pi/skills/documentation/SKILL.md) — including documentation updates made while finishing a coding task.

For editor work, follow [shared presentation](docs/editor.md#build-ui-from-shared-presentation) and the [tooltip/contextual-help guidance](docs/editor.md#tooltips-and-contextual-help). Verify the component and its composed feature.

Write `docs/` for human contributors. Keep agent-only execution instructions here or in skills.

## Build and deliver

Use the normal [contributor build workflow](README.md#build-and-run), including shared native setup. Do not maintain an agent-only bootstrap or Cargo wrapper.

Use `.cache/` for disposable experiments, dependencies and generated outputs. Maintained tools belong in `scripts/` or `native/`. Do not automatically promote downloaded material into maintained source.

Samples are private, read-only, ignored and never committed. System installations and automation of other applications require explicit authorization; respect environment approval requirements for setup and downloads.

An independent, fresh-context agent review against the [development guide](docs/development.md) and relevant topic guides is required before integrating substantial changes. Keep its depth proportionate to the change's breadth and risk. Review supplements verification; assess findings with evidence rather than treating the review as an approval gate.

Explicitly include [test quality](docs/development.md#meaningful-tests) in the review request at both whole-case and individual-assertion level, including fixture/control-discovery assumptions. Ask reviewers to distinguish meaningful protection from incidental implementation or UX lock-in, not merely confirm passing tests or agreement with current code/docs. Review reports should identify the test scope examined, concrete findings and coverage limits.

An explicit wrap-up/integration request authorizes delivery of agreed work as a pull request: push the rebased task branch, open or update the PR, and watch [CI](docs/build-and-delivery.md#continuous-integration) to green. The Metal-dependent suites are agent/local checks and still run before delivery; hosted CI cannot gate them. Merging the PR is the user's action on GitHub, and pushing `origin/main` directly remains a separate request.
