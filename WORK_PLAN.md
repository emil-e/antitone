# Comment, test and prose hygiene

## Agreed scope and acceptance

Comprehensively audit tracked authored source, tests, maintained tools and prose under `docs/code-guidelines.md`. Remove change narration, unnecessary negatives, stale prose and duplicated inventories; preserve useful rationale as current contracts. Remove or replace tests only where they protect incidental details or provide false assurance. Preserve runtime behavior, numerical gates, compatibility knowledge and meaningful coverage.

Exclude generated/vendor/build output, private samples, other worktrees and unrelated work. Preserve consequential decision history, scientific provenance and research evidence. Guidance transferred from main belongs to this task. The user authorizes local integration, including applying this audit to work added on main since the task branched. No push is authorized.

## State and next action

The repository-wide candidate audit and contextual cleanup are ready for review. Changes include shared/component/composed UI tests, the held-out generator check, production comments and contributor/topic/plan/research prose. Runtime Rust/WGSL tokens are unchanged; hashed asset definitions and fixture binaries are untouched.

Permanent removed-label assertions and copied UI painters are replaced by or left to meaningful routing, transaction, layout, visible-feedback, input and accessibility checks. Static shader/asset constraints, ABI checks, reproducibility and mathematical identity properties remain where they protect real contracts.

## Remaining integration work

- Checkpoint this cleanup and rebase onto current main, preserving the new bounded export-forecast implementation and unrelated dirty main files.
- Audit incoming production/test comments and documentation under the same guidelines; resolve overlapping prose semantically rather than restoring inventories or obsolete statements.
- Verify the integrated result, transfer lasting observations to their owning guide/plan, and remove this task plan.
- Commit, fast-forward main and safely remove this task worktree after inspecting tracked, untracked and ignored artifacts. Preserve other active worktrees.

## Verification and limits

- Core: 139 passing tests. Presentation: 55. Viewer: 17. Structured controls: 1. Shader composition: 1.
- Affected framework adjustments/mask checks: 11 passing in both debug and release.
- Python: 2 held-out generation and 5 formatter-tool tests pass. A disposable name-independent-geometry mutation is rejected by the strengthened assertion. The broader numerical Python suite was not run; the available interpreter lacks NumPy.
- Thumbnail suite: 75 pass, one opt-in case is ignored, and the unchanged native-priority test times out at `editor_source_cache_tests.rs:559`. That timeout reproduces on a rebuilt, unmodified HEAD checkout. The changed embedded-preview, selection and style cases pass. Baseline diagnostic logs remain in `.cache/`; the disposable baseline checkout is removed.
- Strict production editor Clippy and strict `ui-tests` Clippy pass. Editor-only **test** Clippy is blocked by the pre-existing unused `ScopeInput::frame` method at `editor_scopes.rs:186`; no lint rule or feature gate is weakened.
- Formatting, whitespace, changed-document local links/anchors and production-token comparison pass. Full native/photographic/device acceptance is outside this behavior-preserving cleanup.

The initial card cleanup has independent review; expanded changes have targeted checks and manual lost-meaning/coverage inspection, without repeating that review. Disposable searches/logs stay under `.cache/`; native dependencies use the documented shared-cache link.

## Tangential finding

Busy tool identity shifts Apply/Cancel auto IDs: `ToolIdentity::icon_warned` inserts a spinner with `ui.put` while idle identity paints directly (`src/editor_ui.rs:417–421`). This is an existing runtime behavior, not changed by the cleanup. Any correction needs separately agreed runtime scope.
