# Code guidelines

Keep durable knowledge and meaningful checks; remove residue of the process that produced the code. Judge comments, tests and documentation by what a future contributor needs to understand or preserve, not by how thoroughly they narrate a change.

## Comments and explanatory prose

Explain current contracts, rationale and non-obvious constraints. Useful subjects include ownership, ordering, numerical assumptions, platform quirks, compatibility, security and surprising side effects. Public documentation should describe supported behavior at the reader's level rather than enumerate private helpers or call paths.

Remove obvious restatements, patch narration, completed TODOs and temporary investigation or review notes. Replace “previously X, now Y” with the reason Y matters, if that reason is still useful. Preserve consequential history where it belongs: decision records, migration guidance and changelogs.

Negative statements earn their place when they rule out a plausible, consequential misunderstanding. “Edits are not persisted until Save” describes a useful boundary; “the old toolbar is no longer rendered” usually records only a completed change. Keep prohibitions and exclusions that explain real safety, compatibility or lifecycle constraints.

Test names should describe the scenario and invariant actually exercised. Comments can explain unusual fixtures or hidden preconditions; neither names nor comments should claim a diagnosis or guarantee the assertions do not establish.

Keep one explanation per topic in its owning guide. Avoid parallel implementation inventories, speculative caveats and explanations of alternatives readers have no reason to expect. Rewrite awkward prose when it contains useful knowledge rather than deleting the knowledge with it.

## Tests that protect a contract

For each test, ask: **What plausible defect would this catch, and why would that defect matter?** Derive expectations from a requirement, a justified invariant or an independent reference. Agreement between two paths can establish consistency without establishing that either is correct.

Prefer checks that allow behavior-preserving implementation changes. Exact wording, snapshots, private helper calls, ordering, representations and operation counts need justification as contracts. They can be essential for accessibility, serialization, transactions, resource lifetime or performance; they are not requirements merely because the current implementation produces them.

Watch for assertions that:

- Restate hardcoded fixture values or compare a result with itself through equivalent logic.
- Search source text to prove a removed UI label is absent instead of exercising the relevant UI behavior.
- Mirror the implementation's calculations without an independent basis for correctness.
- Verify only a mock arrangement, or duplicate another test without covering a distinct failure.
- Capture broad snapshots without making the meaningful expectation apparent.

These are review signals, not automatic deletion rules. Structural guarantees can deserve architectural or boundary tests; source inspection can enforce a real static constraint. Distinguish those from facts guaranteed only by the test's own setup.

When an expected value changes, establish why the new value is correct before updating the assertion. A passing test or newly accepted snapshot is not that justification. Numerical tolerances need the same scrutiny.

## Applying the guidelines

Read the surrounding implementation, requirements and tests before classifying a candidate as remove, rewrite, keep or discuss. State the concrete reason; keyword matches such as “not”, “old” or “regression” are only search aids.

Test deletion needs stronger justification than prose cleanup: identify the protection being removed and whether it remains necessary or is covered elsewhere. Preserve fixture rationale, compatibility knowledge and non-obvious failure conditions. Keep uncertain cases for discussion rather than treating uncertainty as evidence of redundancy.

Keep cleanup behavior-preserving and inspect the diff for lost meaning. Use the relevant [development checks](development.md); comments and documentation can also contain executable examples or affect generated output.
