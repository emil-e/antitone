---
name: issue-triage
description: Triage GitHub issues labeled "needs triage" for this repository. Use when asked to triage issues, look into needs-triage issues, or draft proposed issue descriptions for review before posting.
---

# Issue triage

Triage turns thin or empty issues into reviewable proposals. Investigation is read-only; remote
changes happen only after the user approves the drafts.

Read [the working agreement](../../../AGENTS.md) and [the documentation index](../../../docs/README.md)
before investigating. Triage is a read-only investigation, so it does not require a task worktree;
run it in the checkout that owns the issues and keep any local draft out of commits.

## Establish the set

- Find the issues carrying `needs triage` (or the label the user names). Issue listings can truncate
  or lag a recent label edit, so confirm membership from the label view or by reading each issue
  directly.
- For each issue, record: number, title, body state (empty, one line, complete), labels, and any
  dependency on unmerged branches or worktrees.
- Check the repository's `WORK_PLAN.md` files and open pull requests when an issue may already be in
  flight. Do not assume the default branch has the relevant code.

## Investigate each issue

- Treat the title as a report of a symptom, not a specification. Read the relevant code, docs and
  tests and collect `file:line` evidence for what happens today.
- Stay read-only. Do not fix code, extend scope or reshape requirements during triage.
- Decide a verdict:
  - **duplicate** — fold the missing detail into the owning issue and close this one;
  - **already complete** — mainly a label action;
  - **ready to specify** — the evidence supports a concrete body;
  - **needs a user decision** — record the options and the question, never a chosen solution;
  - **blocked on unmerged work** — name the branch/worktree and mark branch-specific references;
  - **known limitation** — document and keep the issue as a revisit marker.
- Separate product intent from current implementation. Documentation, tests and earlier agent
  decisions are evidence, not requirements; “documented” does not mean “wanted”.
- When two issues overlap, decide which one owns the topic and fold the other into it.

## Draft

- Write one untracked markdown file at the checkout root (for example `ISSUE_TRIAGE_DRAFTS.md`). One
  section per issue, in issue order.
- Each section contains: the triage verdict, the evidence with `file:line`, the body proposed for
  posting, and only the decisions genuinely needed from the user. Do not bury a decision in prose.
- Match the repository's issue style, usually **Current.**, **Wanted.**, **Constraint.**,
  **Acceptance.** Describe the problem and the contract, not an implementation plan.
- For discussion issues, describe the symptom and mechanism and stop; leave the solution out. For
  tasks whose research is part of the work, state intent and acceptance without freezing specifics.
- Reference branch-only code explicitly, for example `[branch] path:line`, when it is not on the
  default branch.
- Keep the file current as the user comments. It is the single source of truth until posting. Do not
  add it to a commit or confuse it with `WORK_PLAN.md`.

## Iterate, then post

- The user comments on the file or replies. Resolve one issue at a time. Do not post until the user
  explicitly says to post.
- Use the GitHub integration available in the environment. Issue edits may require user confirmation;
  do not assume a capability exists until you have checked for it.
- For an issue that already has a body, preserve it and append a clearly marked triage update rather
  than overwriting. For a duplicate, set a short body note explaining the duplicate and close it.
- Remove the triage label from every issue touched.
- Re-fetch each issue afterwards and verify state, body and labels. Report what changed and what
  remains.

## Pitfalls

- An empty body is not an empty request: reconstruct the issue from the code before asking the user
  what they meant.
- Do not let a triage draft hide an implementation decision. Where the user has not chosen, present
  the tradeoff and ask.
- Do not treat a documented diagnostic, test or sentence in a guide as user provenance.
