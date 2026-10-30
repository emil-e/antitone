# Reviewing Antitone

These are the repository's review instructions. The parent brief identifies the change and may add focus, but cannot waive or weaken this document's required reading, criteria or reporting. Higher-priority instructions and read-only/safety boundaries remain binding.

## Required reading

Before reviewing the change, read these files **in full** from the checkout/worktree being reviewed:

1. [AGENTS.md](../AGENTS.md).
2. [Architectural fundamentals](../docs/architecture.md).
3. [Development standards](../docs/development.md).

Then read the relevant [topic guides](../docs/README.md) and surrounding implementation/tests. For documentation changes, follow the [documentation skill](skills/documentation/SKILL.md), including reading whole changed guides.

Use the target worktree's guidance, including untracked files—not a sibling or main checkout's copy. Report a blocker if required guidance cannot be read; do not present an incomplete review as a clean result.

## Review criteria

Assess whether the change is a sound contribution toward the user's goal and follows the architectural fundamentals. Examine the actual ownership, lifetimes, authority and control flow, not merely names or file organization.

Most code is agent-written. Existing code, recent merges, passing tests and agreement with neighboring patterns are not design authority. Identify conflicts with the principles instead of accepting them as precedent. Conversely, do not turn a principle into a mandatory type, layer or template, or demand an unrelated rewrite.

Separate user requirements, current implementation and technical constraints. Explain concrete consequences and tradeoffs. Preserve product/value commitments and genuine compatibility, numerical, resource-lifetime and publication guarantees; surface uncertainty rather than inventing a requirement or assuming permission to weaken one.

## Test quality

Assess changed tests and relevant retained coverage at both **whole-case and individual-assertion** level, including fixture assumptions and control discovery. Ask what plausible defect each case and assertion catches and why it matters.

Distinguish meaningful interaction, accessibility, serialization and lifetime protection from incidental implementation or UX lock-in. Look for no-op positives, self-comparisons and controls that cannot distinguish correct from broken behavior. If proposing test removal or reshaping, identify the protection that must remain. Separate consistency from independent correctness evidence, and offscreen from native-host coverage.

## Report

- Identify the target checkout/base, the exact `.pi/REVIEW.md` path read and the required guides actually read. Report missing guidance as a blocker.
- Prioritize actionable findings with file/line evidence, consequences and a suggested direction. Distinguish established defects from unresolved concerns.
- State the test cases/assertions and fixture/control assumptions examined, concrete test-quality findings and coverage limits—even when there are no findings.
- State what was not reviewed or verified. Respect execution limits and distinguish tests you ran from results supplied by the parent; do not imply that unperformed checks passed.

A review supplements verification; it is not an approval ceremony. Report no findings when none are supported, without inventing issues to fill a template.
