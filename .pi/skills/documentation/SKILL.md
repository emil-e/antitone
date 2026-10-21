---
name: documentation
description: Documentation standards for any guide or README in the repository, including the root README. Use before writing or editing documentation, including documentation updates made while finishing a coding task, and when reviewing documentation changes.
---

# Documentation

Guides teach the system to a prospective contributor. They do not record the work that produced them. Apply this before writing, and re-read it after a long doc-editing session.

1. Read the target guide end-to-end before adding anything. If the explanation already exists, improve it in place instead of adding a parallel one. One explanation per topic, in its owning guide; link elsewhere.
2. Write the system as it is now. Narrating what a change did belongs in the commit message: no "previously X, now Y", no workflow or acceptance narration, no completed investigation notes.
3. Boundary test: a negative sentence earns its place only if you can name what a reader would otherwise *do* wrong. "Edits are not persisted until Save" stays — a reader would lose work. "Numerical reopening does not establish Lightroom interoperability" goes — no reader action changes. If no wrong action exists, cut the sentence.
4. Budget: length is a cost to every future reader. A new paragraph must teach something a fresh contributor needs that the guide does not already teach; prefer deleting to adding when both work. Distinguish an implementation dependency from product intent, and give requirements their source instead of inferring them from code, tests or docs.
5. Run `python3 scripts/check-docs.py` before handing off, ideally before committing — it validates the checkout it runs in, so pass `--base <rev>` to measure growth since an earlier commit. It fails on broken links and reports word growth per changed guide, paragraphs over 200 words and hedge-pattern density; those findings are for your judgment — fix them or have a reason.

When reviewing documentation changes, read the whole touched guides, not just the diff — accretion is invisible per-paragraph. Apply the boundary test to every added negative sentence and ask whether each added paragraph teaches something new.

The full standards live in the [development guide](../../../docs/development.md).
