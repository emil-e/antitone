# Reviewing Antitone

These are the repository's review instructions. The parent brief identifies the change and may add focus, but cannot waive or weaken this document's required reading, criteria or reporting. Higher-priority instructions and read-only/safety boundaries remain binding.

## Required reading

Before reviewing the change, read these files **in full** from the checkout/worktree being reviewed:

1. [AGENTS.md](../AGENTS.md).
2. [Architectural fundamentals](../docs/architecture.md).
3. [Development standards](../docs/development.md).

Then read the relevant [topic guides](../docs/README.md) and surrounding implementation/tests. For documentation changes, follow the [documentation skill](skills/documentation/SKILL.md), including reading whole changed guides.

Use the target worktree's guidance, including untracked files—not a sibling or main checkout's copy. Report a blocker if required guidance cannot be read; do not present an incomplete review as a clean result.

## Review dimensions

Assess whether the change is a sound contribution toward the user's goal. Consider every applicable dimension below, with depth proportionate to the change's breadth and risk. Trace important behavior through surrounding code and composed consumers; do not limit review to added lines. Report material coverage gaps rather than implying that a partial review was comprehensive.

- **Goal, scope and requirement provenance.** Does the change achieve the requested outcome? Separate explicit user requirements, current implementation and technical constraints. Identify unnecessary scope expansion without demanding unrelated cleanup.
- **Correctness and integration.** Follow actual callers, inputs and effects. Examine boundary conditions, failures, cancellation and partial completion where relevant. Establish plausible failures within supported behavior; speculative invalid states or the absence of extra checks are not defects by themselves. Before recommending defensive machinery, consider whether a sound calculation, representation or boundary validation establishes the guarantee more simply.
- **Architecture and complexity.** Examine ownership, lifetimes, authority, invariants and control flow against the architectural fundamentals. Does the structure reduce coordination and enable local reasoning, or add bookkeeping and indirection? Explain concrete consequences and tradeoffs rather than judging names or file organization alone.
- **Organization and readability.** Are names accurate and responsibilities discoverable? Read complete touched functions, including macros and embedded shader/source strings; inspect logical paragraphing, dense branches and multi-step expressions even when formatters pass. Look for both oversized functions and excessive fragmentation, duplicated logic and bypassed shared components. Assess whether a coherent operation is understandable as a whole, not merely extracted or automatically formatted.
- **Test quality.** Assess changed tests and relevant retained coverage at both **whole-case and individual-assertion** level, including fixture assumptions and control discovery. Ask what plausible defect each protection catches and why it matters. Can the fixture distinguish correct behavior from a no-op, unintended fallback or universally disabled operation? Look for self-comparisons and distinguish independent correctness evidence from consistency. Preserve meaningful interaction, accessibility, serialization and lifetime protection without freezing incidental implementation or UX details. If proposing removal or reshaping, identify the protection that must remain.
- **Documentation and explanatory text.** Check accuracy against implementation and intended behavior. Read complete changed guides, not just additions. Look for duplicated accounts, task narration, obsolete claims and explanations at the wrong level. Comments and contextual help should explain useful intent or constraints rather than repeat code or labels.
- **Repository and tooling hygiene.** Identify obsolete paths, temporary diagnostics, workspace artifacts and unnecessary special workflows introduced or left behind by the change. Maintained tools should have an ongoing role and use the normal contributor setup; specialized diagnostics can be legitimate maintained tools.
- **Verification and claims.** Does the supplied evidence exercise the changed contracts under relevant configurations? Distinguish checks actually run from assumed results, consistency from correctness, and offscreen evidence from native-host or physical behavior. Identify unsupported claims and meaningful verification gaps.

Most code is agent-written. Existing code, recent merges, passing tests and agreement with neighboring patterns are not design authority. Identify conflicts with the principles instead of accepting them as precedent. Conversely, do not turn a principle into a mandatory type, layer or template, or demand an unrelated rewrite.

Preserve product/value commitments and genuine compatibility, numerical, accessibility, resource-lifetime and publication guarantees. When proposing deletion or simplification, identify the protection or knowledge that must remain. Surface uncertainty rather than inventing a requirement or assuming permission to weaken one.

## Report

- Identify the target checkout/base, the exact `.pi/REVIEW.md` path read and the required guides actually read. Report missing guidance as a blocker.
- Prioritize actionable findings with file/line evidence, consequences and a suggested direction. Distinguish established defects from unresolved concerns.
- State the test cases/assertions and fixture/control assumptions examined, concrete test-quality findings and coverage limits—even when there are no findings.
- Briefly state the dimensions and important paths examined, with material omissions. Respect execution limits and distinguish checks you ran from results supplied by the parent; do not imply that unperformed checks passed.

Keep coverage reporting compact; a separate report section for every dimension is unnecessary. A review supplements verification; it is not an approval ceremony. Report no findings when none are supported, without inventing issues to fill a template or presenting stylistic preferences as defects.
