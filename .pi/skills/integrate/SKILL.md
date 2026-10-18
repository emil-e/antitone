---
name: integrate
description: Integrate agreed Antitone work into local main. Use when the user says "integrate" or "wrap up".
---

# Integrate

An explicit integration or wrap-up request authorizes local delivery of agreed work. Respect any narrower limits in the current request. Push only when separately requested.

Use the [development guide](../../../docs/development.md) and relevant [topic guides](../../../docs/README.md) for standards and checks.

Run task commands from the task worktree; perform the fast-forward from main's checkout.

1. **Establish the delivery scope.** Identify the task branch/worktree and main checkout. Inspect tracked, untracked and ignored state, including pending or staged work on main. Preserve unrelated changes and other agents' work.
2. **Reconcile with current main.** Refresh current guidance and inspect intervening main changes. Checkpoint agreed work as needed and rebase onto current main without rewriting unrelated history. Inspect semantic interactions, not just conflicts. Update affected guides, transfer lasting knowledge and remaining scope from `WORK_PLAN.md` into the owning guides or topic plans, and keep the task plan current until delivery.
3. **Verify and review.** Run proportionate checks from the development guide on the integrated result. Substantial changes require an independent, fresh-context agent review against the development and relevant topic guides, with depth proportionate to the change's breadth and risk. Review supplements verification; reviewer approval is not required. Assess findings with evidence, fix accepted findings and explain rejected ones. Verify fixes and inspect the final diff before delivery.
4. **Deliver locally.** Remove `WORK_PLAN.md` once its lasting knowledge and remaining scope are transferred. Commit only the agreed result and fast-forward main. If already on main, commit there. If main advances before delivery, reconcile and verify the interactions before retrying the fast-forward.
5. **Clean up safely.** Verify the task is integrated. Before removing its worktree, inspect tracked, untracked and ignored files for unique work or retained artifacts. Remove the worktree and its disposable checkout-local caches/generated outputs only when no longer needed. Preserve active worktrees and ask before discarding anything not integrated or explicitly abandoned; a merged branch alone does not justify forced removal.
6. **Report.** Give the main commit, checks performed, review disposition, worktree cleanup and remaining limits.
