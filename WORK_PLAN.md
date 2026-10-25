# Deliver plain photographic arithmetic

## Agreed scope and authority

User requested integration of the completed plain-f32 photographic processing, shared renderer/kernel/operators, ordinary-f64 CPU sampling, lazy CPU grade endpoints and shader-formula simplifications. This authorizes reconciliation with latest origin/main, necessary correctness fixes, verification, fresh independent review, commits, task-branch push and a PR watched through CI. User merges on GitHub; do not push main.

Preserve equations, signed/HDR values, stage order, correction/repair/source coverage, validation and rejection transactions. Additional numerical machinery needs meaningful photographic evidence rather than synthetic f64 agreement. Keep root `ISSUE_TRIAGE_DRAFTS.md`, unrelated worktrees and private read-only samples untouched.

## Existing evidence

- Prior implementations/limits: `.cache/F32_VERIFICATION.md`.
- CPU summation study: `.cache/cpu-sampling-study/RESULTS.md`.
- Final branch/formula study: `.cache/grade-simplification/RESULTS.md`.
- Before reconciliation: 158 core cases and targeted Metal/consumer checks pass; all-feature/all-target Clippy, format/docs and release build pass.
- 48 real-photo GPU/scalar pairs and direct old/new comparisons across three related Negative crops have maximum one RGB16 code. CPU grade results match baseline bits. These are limited photographic evidence, not a universal corpus.
- Prior broad parallel library run had five look-workflow timeouts; all pass isolated/serialized without weakened deadlines. No clean broad rerun yet.
- No Windows Vulkan/DX12 or controlled cold shader-compilation timing. No full-pipeline speedup claim.
- Ignored cache contains unique reports and paired photographic outputs; retain worktree while PR is unmerged.

## Delivery steps

1. [in progress] Checkpoint agreed tracked/untracked source plus this plan; fetch and rebase task branch onto latest main (currently 99d91f9). Inspect semantic interactions with unity-WB stage policy (#56) and source-independent viewer painting (#57), not only conflicts.
2. [next] Refresh owning guides and necessary retained diagnostics for current source policy; keep lasting rationale/recipes in guides, execution results in PR/report.
3. [next] Run proportional integrated checks, including local Metal and composed editor consumers, broader serialized tests, formatting/docs/lints, production release and photographic repeat. Investigate failures without hiding them or extending deadlines.
4. [next] Fresh independent review of integrated delta against development/topic guides, explicitly including whole-case/assertion/fixture test quality. Assess findings, fix accepted ones and verify; no automatic repeat review.
5. [next] Inspect final diff, transfer any lasting scope/limits, remove this plan, commit, refetch/reconcile if main changed, push the rebased task branch and create/update the PR. Watch CI to green. Never merge or push main.
6. [next] Report PR/checks/review/limits. Keep worktree and unique ignored artifacts until merged and safe cleanup is established.

## Repository state

Task: `/Users/emilsahlen/src/antitone/.worktrees/review-f32`, branch `refactor/f32-photographic-arithmetic`.
Main checkout: `/Users/emilsahlen/src/antitone`, clean tracked/staged state with unrelated untracked `ISSUE_TRIAGE_DRAFTS.md`.
Remote task branch is 2141c9d; original base e31a5cd. All subsequent agreed implementation was uncommitted before delivery. Remote fetch uses pinned `refs/remotes/pi-github/origin/*`; ordinary `origin/main` may be stale.
