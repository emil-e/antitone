# Shared UI components and standards

Status: shared metrics/tint and text actions are implemented; tool-group, input, card and feedback consolidation remain unfinished. [Architecture](../architecture.md#editor-workspace), [controls](../contracts/editor-controls.md#shared-visual-language) and [root agent rules](../../AGENTS.md#ui-engineering) describe the implemented foundation. [Roadmap](../../PLAN.md) owns cross-package priority.

## Goal and scope

Make the accepted Antitone UX language the normal way to build editor UI, not guidelines or copied painting code. Shared components must provide visual **and interaction** contracts and be straightforward to extend for a concrete new use.

This is UI polish and code standardization only. Preserve accepted appearance/behavior while consolidating implementation. Resource assessment, photographic acceptance and other feature work remain separate; coordinate shared-file changes rather than taking over their scope.

Remaining scope:

- Semantic typography, spacing and surface/state roles as the next real component families need them.
- Tool identity/transaction groups, toolbar and labelled-row composition, section/input presentation, thumbnail cards/captions and existing status/error/busy presentation.
- Incremental consumer migration, isolated and composed-view coverage, and discoverable shared-first instructions that stay aligned with implemented APIs.

Non-goals:

- A new framework/dependency, registry, plugin system, declarative schema or parallel event/state system. Continue using egui layouts, IDs and responses.
- A visual redesign, universal identical styling or hypothetical variants without an actual consumer.
- Processing, colour mathematics, numerical defaults/ranges, geometry, project/preferences schemas, caches, worker scheduling or cancellation changes.
- New operation phases/progress measurement, export cancellation, native menus/dialogs, drag/drop, selection/removal or scopes-disclosure designs. Those remain separately scoped.
- Unrelated formatting/comment cleanup, theme-selection preferences, localization infrastructure or physical/device acceptance claims.

## Remaining migration inventory

Inspect entry points against current code before each slice; this table is not a second behavior specification.

| Family | Entry points | Remaining consolidation |
| --- | --- | --- |
| Actions/icons | `src/editor_ui.rs`, `src/editor_icons.rs` | Reuse these working APIs as later consumers migrate; preserve momentary versus selected semantics and meaningful names/help |
| Inspector hierarchy/inputs | `src/editor_controls.rs` | Shared header/row composition beneath existing section/parameter/calibration/balance adapters, not numerical or wheel mathematics |
| Viewer/tools | `src/editor_viewer.rs` | A reusable tool/action group whose total width measurement and painting share one definition |
| Crop/flat-field settings | `src/editor_crop.rs`, `src/editor_flat_field.rs` | Shared action/labelled-row composition without draft/reference ownership |
| Scopes/feedback | `src/editor_scopes.rs`, `src/editor.rs` | Chrome and existing status/error/busy presentation, not plot colours, analysis or error-priority decisions |
| Filmstrip | `src/editor_thumbnails.rs` | Card layout/state treatment, caption elision and supplied placeholders/badges without image/cache/selection ownership |

The existing helpers already cover semantic icons, independent disclosure/reset, safe numeric editing and UI-only expansion. Extend them rather than introduce parallel replacements. Native egui menu/selection controls, stage tabs, colour-wheel/crop artwork and scope plotting are specialized consumers, not reasons to force every widget through text actions.

Card components must preserve embedded Camera preview/Camera badges and their loading/error precedence. Keep decoding/admission/cache ownership outside card components and never adopt camera previews as authoritative RAW working data.

## Component and ownership constraints

- Use semantic roles, not a catalogue of renamed literals or unrestricted per-call styling overrides. Resolve theme-responsive roles through egui visuals and retain deliberate neutral photographic surrounds. Selected cards, active tools and selected tabs can legitimately differ.
- Introduce spacing/typography/surface tokens when their family needs them. Normal/compact filmstrip geometry and narrow viewer priorities are real contextual variants; do not merge unrelated values because their numbers happen to match.
- Keep generic presentation in the existing private egui-only `presentation` module. Specialized `icons`/`controls` remain adapters. Do not supply `Editor`, `Ready`, `Command`, documents, RAW data or caches to generic components; use explicit imports and small display-ready inputs.
- Controllers retain availability, reset scope, accepted/draft state, validation, Apply/Cancel dispatch, view restoration and worker lifecycle. Domain wrappers remain the source of parameter ranges/defaults/precision; application preferences remain the persistence owner.
- Preserve existing caller ID scopes, focus/popup identities and response semantics. Generic components must not add hidden keyboard handlers or a second transaction/state system.
- Common fixed action names/help belong with the relevant component; contextual titles, paths, scientific explanations and backend errors remain caller inputs. No global string registry.
- Add variants for real semantic needs, not hypothetical extensibility. Follow the shared-first/one-off exception policy in root `AGENTS.md` and update its entry-point links as each family lands.

## Ordered remaining slices

### 1. Tool and toolbar composition

- Sketch the smallest tool-group input/response API using the implemented `TextAction` and shared metrics/tint. Share total width measurement and allocation, not just action widths.
- Preserve the one trailing icon/spinner/name/Cancel/primary Apply group, right alignment, 40-point viewer bar, 12-point name→Cancel gap and narrow-width auxiliary-control priorities.
- Migrate viewer tool groups and inspector active-tool framing; migrate crop/flat-field action rows and scope chrome only where the same primitives fit.
- Keep busy, proposal availability, drafts, keyboard dispatch and canvas ownership in the controllers. No new placement, floating panels or implicit commit/cancel behavior.

### 2. Sections and input composition

- Consolidate header/row presentation beneath existing section, parameter and calibration APIs.
- Preserve numerical/balance wrappers, including Contrast/Pivot/Colour preservation/Saturation/Density. Add a generic input descriptor only when current consumers justify it; do not relocate numerical operators.
- Preserve full-precision commits, disabled snapshots, independent reset/disclosure, pending-edit cancellation, stable IDs, summaries and preference retention.

### 3. Cards, captions and existing feedback

- Extract accepted filmstrip presentation/caption elision and return responses/content bounds so callers can paint current GPU and camera-preview content.
- Retain virtualization, source lookup, pixel-rounded mappings, selection/sampling and scheduling in their current owners. Preserve square image wells, full aspect, compact geometry and stable footprints.
- Consolidate repeated status/error/busy presentation without adding states, inferring progress or changing error priority. Receive real available actions and display-ready text/state; do not manufacture cancellation or idle repaint loops.
- Coordinate with [operation-feedback work](completion.md#operation-feedback); backend phase/cancellation design is not a dependency for extracting current presentation.

### 4. Coverage and handoff

- Check applicable inventory consumers use their shared family; remove redundant implementations without forcing specialized visualization through generic controls.
- Add concise usage/examples alongside implemented components and keep `AGENTS.md` rules/links discoverable. Do not duplicate a full style guide.
- Update affected contracts/architecture as code lands. Remove completed scope from this plan; delete it when no unfinished work remains. Keep unperformed physical/device acceptance in its existing plans.

Each slice should be independently reviewable and verified before proceeding. A material appearance/behavior change or newly required domain work needs separate discussion, not silent inclusion.

## Acceptance and verification

Acceptance requires reusable code and adoption, not guidelines/constants alone:

- Components own the declared visual/interaction contracts; applicable consumers do not duplicate them or bypass them with raw style/interaction overrides.
- Core components are testable without editor/worker/document fixtures. Domain adapters remain explicit.
- Preserve normal/narrow/scaled layout, action ordering, accessible names/roles, keyboard activation, disabled safety and distinct focus/selection treatment.
- Numeric/popover ownership still precedes tool/canvas keys and gestures; tabs/outside clicks do not accept/cancel drafts. Preserve numerical precision/defaults, project/preferences boundaries and exact view restoration.
- Preserve repaint/resource bounds through the abstraction. This is not a new performance/resource acceptance exercise.
- Root `AGENTS.md` points to implemented reusable code and retains a narrow, reasoned one-off exception; a second consumer should prompt shared extraction, not copying.

Use focused `editor::presentation::tests`, `editor::controls::tests`, composed viewer/workspace/scopes tests and thumbnail style/pixel-exact clipping regressions according to the migrated family. Keep composed checks as well as isolated tests; do not weaken geometry/numerical tolerances or replace behavioral assertions with captures.

Follow [development checks](../development.md) and [layout/fixture probes](../build-and-probes.md#editor-and-layout-smoke). Run native normal/narrow captures for changed chrome and GPU card fixtures for changed filmstrip presentation. Keep existing deadlines and visibility gates; ask before broad/expensive/stateful checks.

### Current verification checkpoint

Local snapshot on Apple M5 Pro/arm64, macOS 26.6.2, Rust 1.99.0, Metal: 55 focused integration tests passed (presentation 5, controls 18, viewer 15, workspace 8, explicit response opt-in 1, embedded preview 8). Tests include direct-button paint/geometry/ID equivalence at 1×/2×, style-aware width measurement without widget allocation, semantic names, focus order, inherited/local disable transitions, exact action/bar heights, transaction guards and numerical precedence. Reproduce with:

```sh
cargo test --locked --offline --features editor --lib editor::presentation::tests
cargo test --locked --offline --features editor --lib editor::controls::tests
cargo test --locked --offline --features editor --lib editor::tests::viewer
cargo test --locked --offline --features editor --lib editor::tests::workspace
cargo test --locked --offline --features editor --lib original_grade_requires_explicit_response_opt_in_and_drawing_never_regrades_it
cargo test --locked --offline --features editor --lib editor::thumbnails::embedded::tests
cargo fmt --all -- --check
cargo clippy --locked --offline --features editor --lib --tests -- -D warnings
cargo build --locked --offline --features editor --bin antitone
```

Formatting, strict editor Clippy and debug build passed. Both native tool capture routes passed (six tool and two flat-reference normal/narrow captures); all eight were visually inspected. Use the runbook's window helper and `scripts/capture-viewer-tools.py` with the debug binary/read-only samples to reproduce. Captures/tests do not establish physical input, VoiceOver, preference relaunch, photographic or broader-device acceptance; those remain outside this package. The integrated camera-preview feature retains its separate decoding/admission/cache ownership and filmstrip-only presentation.

## Open interfaces and next action

- What is the smallest tool-group API that shares total width measurement while retaining caller IDs and dispatch?
- Which selected/focus/active treatments need their own roles? Preserve appearance before considering unification.
- What response/content-bounds API supports card GPU/camera-preview content without image ownership?
- Which feedback consumers can share current presentation, and which need separately designed operation states?

Next: settle the tool-group input/response/measurement interface, then migrate its viewer and inspector consumers in a separate worktree. Preserve current appearance, action priority and controller ownership; do not expand into backend feedback or resource/photographic acceptance.
