# Shared UI components and standards

Status: shared metrics/tint, text actions, viewer tool groups, inspector framing, sections/stage tabs, labelled parameter/calibration rows and contact-sheet cards/captions are implemented; existing feedback consolidation remains unfinished. [Architecture](../architecture.md#editor-workspace), [controls](../contracts/editor-controls.md#shared-visual-language) and [root agent rules](../../AGENTS.md#ui-engineering) describe the implemented foundation. [Roadmap](../../PLAN.md) owns cross-package priority.

## Goal and scope

Make the accepted Antitone UX language the normal way to build editor UI, not guidelines or copied painting code. Shared components must provide visual **and interaction** contracts and be straightforward to extend for a concrete new use.

This is UI polish and code standardization only. Preserve accepted appearance/behavior while consolidating implementation. Resource assessment, photographic acceptance and other feature work remain separate; coordinate shared-file changes rather than taking over their scope.

Remaining scope:

- Semantic typography, spacing and surface/state roles as the next real component families need them.
- Existing status/error/busy presentation; scope chrome only where a concrete shared presentation pattern fits.
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
| Scopes/feedback | `src/editor_scopes.rs`, `src/editor_analysis.rs`, `src/editor.rs` | Chrome and existing status/error/busy presentation, not plot colours, analysis or error-priority decisions |

Use the implemented presentation hierarchy/labelled rows beneath photographic adapters; do not introduce a generic input descriptor without a concrete supported need. Crop's aligned grid, balance artwork, native egui selection controls and scope plotting are deliberate specialized consumers, not remaining row migrations or reasons to force every widget through text actions.

Use the implemented contact-sheet card/caption/badge component without moving image fitting/mapping, content precedence, selection or cache/worker ownership into presentation. Camera previews remain filmstrip-only and never become authoritative RAW working data.

## Component and ownership constraints

- Use semantic roles, not a catalogue of renamed literals or unrestricted per-call styling overrides. Resolve theme-responsive roles through egui visuals and retain deliberate neutral photographic surrounds. Selected cards, active tools and selected tabs can legitimately differ.
- Introduce spacing/typography/surface tokens when their family needs them. Normal/compact filmstrip geometry and narrow viewer priorities are real contextual variants; do not merge unrelated values because their numbers happen to match.
- Keep generic presentation in the existing private `presentation` module using egui and the shared icon artwork. Specialized `icons`/`controls` remain adapters. Do not supply `Editor`, `Ready`, `Command`, documents, RAW data or caches to generic components; use explicit imports and small display-ready inputs.
- Controllers retain availability, reset scope, accepted/draft state, validation, Apply/Cancel dispatch, view restoration and worker lifecycle. Domain wrappers remain the source of parameter ranges/defaults/precision; application preferences remain the persistence owner.
- Preserve existing caller ID scopes, focus/popup identities and response semantics. Generic components must not add hidden keyboard handlers or a second transaction/state system.
- Common fixed action names/help belong with the relevant component; contextual titles, paths, scientific explanations and backend errors remain caller inputs. No global string registry.
- Add variants for real semantic needs, not hypothetical extensibility. Follow the shared-first/one-off exception policy in root `AGENTS.md` and update its entry-point links as each family lands.

## Ordered remaining slices

### 1. Existing feedback

- Consolidate repeated status/error/busy presentation without adding states, inferring progress or changing error priority. Receive real available actions and display-ready text/state; do not manufacture cancellation or idle repaint loops.
- Coordinate with [operation-feedback work](completion.md#operation-feedback); backend phase/cancellation design is not a dependency for extracting current presentation.

### 2. Coverage and handoff

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

Use focused `editor::presentation`, `editor::controls::tests`, composed viewer/workspace/scopes tests and thumbnail style/pixel-exact clipping regressions according to the migrated family. Keep composed checks as well as isolated tests; do not weaken geometry/numerical tolerances or replace behavioral assertions with captures.

Follow [development checks](../development.md) and [layout/fixture probes](../build-and-probes.md#editor-and-layout-smoke). Run native normal/narrow captures for changed chrome and GPU card fixtures for changed filmstrip presentation. Keep existing deadlines and visibility gates; ask before broad/expensive/stateful checks.

### Current verification checkpoint

Local snapshot on Apple M5 Pro/arm64, macOS 26.6.2, Rust 1.99.0, Metal: 103 focused tests passed (presentation 18, thumbnails 47, viewer 15, workspace 8, Analyze 15). The thumbnail suite includes embedded-preview, cache/admission, selected-preview and pixel-exact GPU scroll-edge regressions. Isolated presentation checks cover card paint/geometry/IDs/clips and AccessKit output against direct artwork at 1×/2×, normal/compact heights, selection/focus and inherited disable; mouse/keyboard responses, allocation-free sizing and Unicode-safe filename elision. Existing tool/row/section coverage also remains in that suite; Analyze checks cover confirmation, pending numeric snapshots, cancellation and atomic installation. Reproduce with:

```sh
cargo test --locked --offline --features editor --lib editor::presentation
cargo test --locked --offline --features editor --lib editor::thumbnails
cargo test --locked --offline --features editor --lib editor::tests::viewer
cargo test --locked --offline --features editor --lib editor::tests::workspace
cargo test --locked --offline --features editor --lib editor::analysis::tests
cargo fmt --all -- --check
cargo clippy --locked --offline --features editor --lib --tests -- -D warnings
cargo build --locked --offline --features editor --bin antitone
```

Formatting, strict editor Clippy and debug build passed. The GPU-painted camera-roll fixture passed at 140/80/52-point heights, and the native viewer-tool route captured all six normal/narrow crop/Dmin/WB layouts; all nine images were visually inspected. Reproduce with the runbook's [camera-roll fixture](../build-and-probes.md#camera-roll-review-fixtures) and window helper/`scripts/capture-viewer-tools.py` with read-only samples. The ordinary thumbnail suite ignores ten explicit probes; the fixture was run separately, while nine RAW/resource probes remain unperformed. Two explicit real-RAW Analyze tests also remain unperformed. Captures/tests do not establish physical input, VoiceOver, preference relaunch, photographic or broader-device acceptance; those remain outside this package. Camera-preview decoding/admission/cache ownership and filmstrip-only presentation remain separate.

## Open interfaces and next action

- Which selected/focus/active treatments need their own roles? Preserve appearance before considering unification.
- Which feedback consumers can share current presentation, and which need separately designed operation states?

Next: inspect existing status/error/busy consumers and identify a genuinely repeated presentation pattern using display-ready state and supplied actions. Coordinate with operation-feedback work; do not introduce backend phases/progress/cancellation or expand into resource/photographic acceptance.
