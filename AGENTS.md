# Working on Antitone

## Context and authorization

1. Read [PLAN.md](PLAN.md) for current focus, priority and blockers.
2. Use [docs/README.md](docs/README.md) to find topic guides, important behavioral boundaries and unfinished work. Read [architecture](docs/architecture.md) when tracing unfamiliar boundaries.
3. Read relevant [decision rationale](docs/decisions/README.md) before changing those choices.
4. Inspect actual code/tests before editing. If code and contract disagree, investigate rather than silently declaring either correct.

The latest user request determines authorization. Roadmaps, decisions and broad plans do not authorize unrelated implementation, commits, pushes, global installs or remote actions. Confirm ambiguous/risky scope; keep routine changes focused. Use a separate worktree for substantive changes and preserve unrelated working changes.

## Documentation policy

The checkout describes the current system and unfinished work, not how the project got here.

- `README.md` is for humans: introduction, prerequisites, normal build/run/check commands and contributor navigation. Agent authorization and handoff procedures belong only in root `AGENTS.md`.
- Write for a prospective contributor, not an agent handoff. Topic guides begin with the problem and mental model, teach why the system works as it does in easy-to-follow prose, and use a small diagram/example where helpful. Keep behavioral boundaries concise. Architecture maps implemented ownership; decisions preserve selective rationale, without superseded instructions or approval narration.
- Leave implementation inventories, literals, equations' numerical evaluation, serialization mechanics and exhaustive cases to code/comments/tests. Preserve important behavioral guarantees in the owning guide; do not build a second API specification or silently discard reasoning that code does not explain. Use one primary explanation per topic and link rather than repeating it.
- Plans contain only genuinely unfinished scope, open questions, acceptance and next actions. Remove completed steps; delete a finished plan after transferring current behavior and remaining requirements to their proper homes.
- Delete obsolete tools, superseded decisions, duplicate notes and execution records. Do not create archives, completed-plan collections, compiler transcripts, approval histories or session/worktree narratives. Git preserves removed versions; do not retrieve them to resurrect designs or tools unless the user explicitly requests historical investigation.
- Keep a technical explanation only if it helps a contributor understand or maintain the current system. Introduce the idea before terminology and details. Write the insight directly, not the sequence of experiments that produced it; avoid dense qualification-heavy prose and repeated facts across guides, plans and roadmaps.
- Keep maintained tools in `scripts/` or `native/`, with current commands in the contributor guides. Do not keep unsupported snapshots for hypothetical agent use.
- Proposed designs are not policy. Distinguish open hypotheses from established behavior and do not promote feedback or a passing consistency check into a design decision or physical acceptance claim.

## Documentation roles and handoff

- `PLAN.md`: short current status, ordered roadmap and blockers.
- `docs/README.md`: contributor entry points by question/task.
- `docs/development.md`: everyday/feature checks; `docs/diagnostics.md`: supported numerical/reference, RAW, export and presentation recipes; `docs/native-development.md`: production RAW diagnostic and native-build reuse.
- `docs/architecture.md`: implementation map and ownership, distinguished from unwired foundations.
- `docs/{processing,roll-analysis,flat-field,colour-and-output,geometry,editor,projects,scopes}.md`: focused models and their important behavior/invariants/compatibility. Processing explains pixels transformed by supplied settings; roll analysis explains estimating those settings. Update the owning guide when an authorized change changes its guarantees; exact implementation details belong with code/tests.
- Follow the topic/task structure in `docs/README.md`. Extend the owning guide before adding a new page; add one only for a distinct contributor question. Do not recreate catch-all processing documents, a parallel `contracts/` tree or separate introduction/contract/API pages for every feature. Repair navigation and links when moving material.
- `docs/decisions/`: selective rationale for consequential current choices, not repeated specifications.
- `docs/plans/`: substantial unfinished packages. Record scope/non-goals, open design/approval questions, acceptance, current limits and exact next action. Small changes need no plan file.

At a meaningful checkpoint or handoff, update unfinished scope, verification limits and next action; do not append a session log. Update the roadmap only when focus/status/order/blockers change. Run proportional checks and repair affected links. Report checks actually run and unverified acceptance without inflating a snapshot into a general guarantee.

## Wrap-up procedure

An explicit wrap-up request authorizes local delivery of agreed work, never a push:

1. Checkpoint-commit agreed changes if needed, then rebase/integrate onto current main, preserving unrelated changes/worktrees. Inspect new main behavior, owning guides and tests, including semantic interactions that merge without conflicts. Discuss material new design/scope before expanding work.
2. Update current topic guides, architecture and roadmap; remove completed plans and repair links. Keep deferred work/unperformed acceptance explicit, without a closure journal.
3. Verify the integrated result proportionally and inspect the final diff. Report unresolved failures and limits.
4. Commit only agreed changes and fast-forward main to the verified result. If already on main, commit there; do not rewrite unrelated history.
5. Report the main commit, checks and remaining limits. Push only when separately requested.

## Engineering boundaries

- macOS/Apple Silicon first; no minimum chip/RAM target is established. Resource policy follows observed problems. Actual macOS 14, broader devices and physical monitors require separate acceptance.
- Reuse pinned LibRaw development. No custom demosaicer, camera-specific repair, CPU interactive fallback, approximate ACES replacement or weakened numerical gate.
- Preserve native-camera calibration/inversion, signed/HDR AP1 working data and AP0 EXR. AP0 otherwise belongs only inside ACES. Camera model/matrix identity must match the roll's retained basis. Preview/TIFF targets are matching 100-nit sRGB/Display P3 with managed extended-sRGB SDR presentation.
- Dmin/Dmax are RGB-only values. Picker context is transient; thumbnail-cache freshness and development compatibility are separate. Do not introduce RAW content hashing or capture identity without a new requirement. Invalid/cancelled picks and rejected edits change no processing state.
- Processing model is 4; project schema is 5 with no RAW fingerprints. Unsupported schemas/AP0 grades reject without migration. New grades use toe 2.5/shoulder off; the classic toe-1 curve uses the same camera model.
- One RGB16 production source path: 8 bytes/pixel. Higher precision belongs in temporaries/small constants, not a wide retained source cache.
- Samples are read-only, ignored and never committed. Do not automate Lightroom/Resolve or add correction workflows without a request. No hidden pedestal/clamp or development changes to improve inspection.

## UI engineering

For new or changed editor UI, use the shared code rather than copy feature-local layout, painting or interaction logic:

- Inspect [presentation metrics, actions and labelled rows](src/editor_ui.rs), [sections/stage tabs](src/editor_ui_hierarchy.rs), [contact-sheet cards/captions](src/editor_ui_cards.rs), [icon actions](src/editor_icons.rs) and [photographic control adapters](src/editor_controls.rs) first. Use their applicable APIs and inherited colour/interaction roles; preserve natural versus toolbar sizing. Follow the [colour-role guidance](docs/editor.md#colour-roles) rather than defining feature-local palettes. Follow the [editor model](docs/editor.md) and [workspace boundaries](docs/architecture.md#editor-workspace).
- Presentation owns layout/actions, tool framing and UI-only hierarchy. Use `labelled_row` for a left label with right-aligned supplied controls, `section`/`SectionHeader` for disclosure/reset chrome and `ToolGroup` for complete viewer identity/action measurement. `controls` retains parameter/calibration/balance semantics, including defaults, precision and disabled snapshots; feature controllers retain availability, reset scope and dispatch. Preserve deliberately different crop grids and scope chrome rather than forcing them through a labelled row. Use `ContactSheetLayout` for card geometry and `begin` with caller-owned IDs; paint supplied image content before the card's badge/placeholder and caption. Image fitting/mapping, content precedence, virtualization, selection and scheduling remain caller-owned.
- Extend a shared component for a real supported variant instead of bypassing it with local style/interaction overrides. Do not unify deliberately different tab/card/tool treatments or specialized image visualization merely because they share an accent.
- Keep domain values/defaults, availability decisions, transactions, processing, workers and persistence in their existing owners. Components present supplied state and return responses; preserve accessible names, focus/keyboard precedence, disabled safety and geometry/IDs.
- A local implementation is acceptable only for a genuinely one-off requirement that does not fit an existing component family. State the concrete reason when proposing it and keep it within the shared UX language. When a second consumer needs that pattern, promote it into a shared component rather than copy it. Native egui menu/selection controls and specialized visualization need not become generic text actions.
- Verify the affected component contract and composed consumer. Avoid speculative abstractions, a parallel widget/state framework and unrelated migrations. Keep these entry-point links current as shared families evolve.

## Build and generated storage

Dogfood the human workflow: ordinary Cargo from the repository root, project rustup selection and Apple Clang from `.cargo/config.toml`. Follow [README](README.md), `./scripts/setup-native.sh`, `cargo build-app` and `cargo app`. Do not maintain an agent bootstrap or Cargo wrapper. Sandbox-required environment overrides are local execution details, not a second workflow.

`.cache/` is disposable storage for dependencies, build/probe products, generated/private photographic outputs and downloaded third-party inputs. Maintained source and current rationale belong in the checkout; generated logs/screenshots do not. Never promote user photographs, copied upstream experiments or copyrighted documents automatically.

Setup/downloads require permission when the environment requires it. No system/Homebrew installations, sample edits, commits/main integration or pushes are authorized by this file; commits/integration require a user request, including explicit wrap-up as defined above.
