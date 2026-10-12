# UI polish and interaction language

Status: **slices 1–3 implemented; acceptance partial**. The user agreed the UX direction/product choices, authorized the shared-control slice, then requested its commit and continuation into workspace hierarchy/application preferences. Slice 1 is committed as `d1cd936`; the user visually accepted slice 2, committed as `a2dddd5`, and authorized continuation. Slice 3 (viewer/tools) is implemented and locally verified, uncommitted. Its initial Details-based presentation was rejected. The user accepted the compact pickers, rejected crop/flat-field overlays and authorized the inspector/viewer-bar placement revision below. That placement revision is implemented. The user's later consistency clarification supersedes picker overlays too: every image tool now shares a distinct viewer-bar identity/Apply/Cancel group, with no floating tool box. Latest grouping is locally verified and awaits visual acceptance. Feedback/native-shell slices are not implemented or selected. Exact visual details remain subject to validation; authorization does not cover the entire package.

Worktree: `.worktrees/ui-polish`, branch `plan/ui-polish`, baseline `0beee3e`. [Audit and sources](../research/ui-polish-audit.md). [Roadmap](../../PLAN.md) owns cross-package priority.

## Goal and boundaries

Make the existing editor feel like one coherent photographic workspace: predictable places for navigation, editing, inspection, transient tools and feedback; fewer competing boxes and explanatory paragraphs; consistent icons and state language.

This is not a processing redesign, new grading controls, a catalog, undo implementation, a thumbnail/cache optimization or a new rendering framework. Preserve numerical controls/defaults, native-camera calibration, signed/HDR data, unchanged ACES, matching preview/TIFF targets, AP0 EXR, full-resolution picks, cancellation/rejection invariants and existing worker/resource bounds. Future rendering controls get a home, not placeholder widgets or new operators.

## Recommended workspace

```text
macOS menu bar: Antitone | File | Edit | View | Window | Help
window title: project name + unsaved indicator
┌───────────────────────────────────────┬───────────────────────────┐
│                                       │ SCOPES             ▾      │
│                                       │ Waveform / Vectorscope    │
│              PHOTO                    │ plot                      │
│                                       │ P3 · Approximate          │
│                                       ├──── draggable divider ────┤
│                                       │ Roll → Photo → Rendering  │
│                                       │ ━━━ selected tab ━━━      │
│    [selection/frame only]            │ ▾ section title     reset │
│                                       │ parameter controls        │
├───────────────────────────────────────┤ ▸ collapsed section       │
│ fit / zoom | tool · Cancel / Apply   │ crop settings when active │
├───────────────────────────────────────┴───────────────────────────┤
│ resizable filmstrip                                               │
├───────────────────────────────────────────────────────────────────┤
│ current photo / dimensions             operation or issue, if any │
└───────────────────────────────────────────────────────────────────┘
```

[Illustrative wireframe](ui-polish-wireframe.svg) uses synthetic artwork, not a user photograph. It illustrates zones and inspector-owned crop settings/viewer-bar actions, not final pixel-perfect assets or verified app behavior.

- The bottom viewer bar belongs to the **viewer width**, immediately above the filmstrip. It is not the global status bar and does not extend under the inspector.
- Its height is fixed across positive/source view, pick/crop entry, loading and errors. Moving existing rows will change the normal Fit canvas once; temporary tool entry must not change it again.
- Scopes stay in the existing right sidebar, in a distinct inspection region with its own header/background, controls and resize boundary. No new floating/dockable windows in this pass. Post-slice-2 feedback explicitly requests collapsible/hideable scopes: the existing checkbox already hides the plot and persists visibility; refine this into an obvious disclosure-style region header in slice 6. Keep reopening accessible and retain the preferred size; collapsing must not alter grades or viewer geometry.
- The inspector has one stable tab row and one scroll region. Remove the always-pinned Output card and avoid oversized pinned orientation/crop cards above scrollable edits.
- Persistent panel resizing may intentionally change available space. Transient text, previews, errors, progress and toolbars must not.
- On macOS, native menus replace the in-window File/Export row once integration is accepted. Until then retain that row, rather than hiding commands prematurely. Remove repeated app branding from the content header; retain document identity/dirty state in a useful title or compact document header.

## Tabs: workflow, not a pretend processing graph

Recommend **Roll → Photo → Rendering**, in that order. Retain the familiar Roll/Photo labels and their important scope distinction; add Rendering rather than inventing Tone/Color/Finish stages containing nonexistent features.

| Tab | Meaning | Existing controls |
| --- | --- | --- |
| Roll | Shared setup and negative inversion | Scan orientation, flat-field correction, calibration, gamma/balance, toe/shoulder, shared exposure |
| Photo | This image's composition and photographic grade | Composition/orientation, white balance, exposure |
| Rendering | Shared rendered appearance and matching TIFF target | ACES SDR target: Display P3 or sRGB |

Use full-width, equal-allocation tab labels attached to the inspector content, a clear selected underline/surface, and subdued arrows between stages. Not individually filled button pills. Keep labels visible at a 270-point sidebar; no icon-only tabs, stage numbering, completion ticks or disabled later tabs. Tabs navigate; they never apply/reset processing or cancel a draft.

The arrows communicate **shared foundation → per-image work → rendered result**, not a literal operator execution order. Geometry composes separately, shared/per-photo exposure combine, and ACES is downstream. Tooltips explain scope, including Rendering's shared/project-level effect. Do not rename Roll to “RAW development”: development is fixed, and inversion is not ordinary camera RAW grading.

“Rendering” is not the monitor profile or an export-format chooser. Its target affects preview, thumbnails, scopes and TIFF together; EXR bypasses it. Use a labelled **Target** selector (Display P3 / sRGB), a concise visible **Preview + TIFF · 100 nit** note and a tooltip explaining managed presentation/AP0 EXR. Keep a compact target indicator in the viewer bar and scope caption so its state remains discoverable when another tab is active. Do not split preview/TIFF targets or expose monitor-profile overrides.

## Recurring interaction language

### Control hierarchy and sections

Use **region → tab → collapsible section → parameter**, with no decorative nested cards. Gamma & balance currently contains another framed Gamma balance section; flatten its color component while retaining its independent reset.

Section headers have a disclosure chevron, a visible title and a trailing scoped reset when meaningful. Clicking reset must not also collapse the section. An adjusted collapsed section stays collapsed and shows one short state summary and a non-default marker; collapse never disables processing, dirties the project or commits pending numeric text. Missing calibration stays visibly “Not measured” even if the section is collapsed. Do not offer calibration reset through generic section reset.

Allow multiple sections open; do not automatically close another section. Remember section expansion, scope visibility/size and filmstrip height across application launches as application preferences, not project fields. They must not dirty a project or alter processing. Every successfully created new project starts on **Roll**, regardless of the previous tab or restored UI preferences; do not reset the tab merely on photo switching/import. Open-tab behavior otherwise retains existing semantics. Inspection EV, viewport and tool drafts are not made durable by this preference decision.

Implemented first-run defaults: Calibration and Gamma & balance open on Roll; Scan orientation, flat-field, Toe & shoulder and shared Exposure collapsed. Composition, White balance and Exposure open on Photo. Rendering's Target open. Missing calibration opens Calibration for a new project, not on every repaint. Collapse preferences must not affect orientation availability before calibration.

Keep stable parameter names, values, units, numeric entry and slider semantics. Keep existing normal/fine steps; do not change wheel sensitivity during this layout pass. Section reset, parameter reset, crop-only reset and orientation reset keep their existing distinct scopes/defaults. Generic “Reset all” is not introduced.

### Icons versus text

Use a single licensed, reproducible outline icon vocabulary, consistent stroke/size/alignment and states. Do not use emoji or arbitrary Unicode as the production asset system. Slice 1 uses project-authored vector strokes in `src/editor_icons.rs`, with no copied icon set, font or added dependency. Native SF Symbols availability does not automatically make them portable egui assets.

| Prefer icons, with tooltip and accessible name | Keep visible text/value |
| --- | --- |
| Fit, zoom −/+, source-view toggle, inspection settings, help | Zoom percentage; target name |
| Rotate left/right, flip horizontal/vertical, crop | Roll/Photo/Rendering tabs and section names |
| Pick/eyedropper, reset, disclosure, aspect swap, guides | Dmin, Dmax, Gamma, WB, Exposure and their values/units |
| Scope visibility, help/info | Flat-field reference chooser, Apply, Cancel, Save, Discard, Replace |

An eyedropper next to Dmin/Dmax or White balance inherits that labelled context; its tooltip says precisely what it measures. A standalone mystery icon does not. Use different icons for rotate, aspect swap and reset; mirror/axis symbols must remain distinguishable. Flips remain momentary actions, not checked canonical flags. Crop is selected while its draft is active; source inspection and guide icons are actual toggles.

Provisional visual tokens (logical points): 4/8/12 spacing scale, 12-point region padding, 16-point outline artwork within a consistent approximately 28-point pointer hit area. These are Antitone desktop starting values, not universal Apple requirements; verify usable focus and hit targets on real hardware. Reserve the accent color for selection, active toggles and the one primary Apply action. Use neutral surfaces, fewer full card borders, consistent header/value typography and separators; don't style every button as equally prominent. Distinguish state through shape/text/focus as well as color.

Tooltips name the action, scope and shortcut; longer technical explanation belongs in an info/details popover or Help. Tooltips supplement keyboard focus/accessible labels, never replace them. Accessibility integration in the custom host needs verification rather than assuming hover text produces VoiceOver names.

### Viewer bar

Leading **navigation** (Fit icon, zoom percentage/100% action, −/+) and adjacent **inspection/help**; flexible space; trailing **state/actions**. Normally the trailing group shows rendering target and compact preview quality. During any image tool, target/quality yield to one tinted icon/name/Cancel/Apply group, with primary Apply at the far right. The unavailable source toggle is omitted during all tools. Target identity remains in Rendering/scopes. This is one fixed-height row, not equal thirds or independently centered groups. Keep 100% numeric rather than replacing it with an obscure icon. A precise menu can offer Fit/100%; do not add new arbitrary zoom behavior.

The source toggle tooltip says “Show developed source”, not “undeveloped”: LibRaw has already developed it. When active, show a short **Source** state label, **Inspection EV** value and specifically named inspection control inside the fixed-height bar so changed view brightness is not mistaken for a grade. Hide the inspection control/value entirely in positive view, retaining the value for later source viewing. At constrained widths zoom ± and inline EV copy may yield to the active group; Fit/100%, inspection and help remain available, and EV stays in its tooltip/popup. No hidden adjustment to exports/calibration. The bar and viewer surround share a darker surface without an intervening divider; the lighter filmstrip has its own top boundary.

Remove the always-visible “Preview ready” row. Incomplete quality remains discoverable as **Approximate**, with a spinner only while refinement is actually running. Ready is a quiet state/tooltip, not a persistent success message. Keep navigation help keyboard-accessible, not just a hover-only `?` label.

### Contextual tools, with separate homes

Every image tool shares one **viewer-bar transaction group**: recognizable icon and explicit name, then Cancel / primary Apply on a common light tint. This distinguishes a pending tool from loose status text. Busy work replaces the icon with a spinner and disables unsupported actions. Nothing floats over the image except its selection/frame.

| Mode | Settings / selection home | Identity and Cancel / Apply |
| --- | --- | --- |
| Dmin / Dmax | Canvas selection and filmstrip roll choice; no external chooser | Shared viewer-bar group |
| Gamma / WB neutral | Canvas selection | Shared viewer-bar group |
| Flat-field support | Existing Roll section and separate bare-light workflow | Shared viewer-bar group |
| Crop & straighten | Existing Photo Composition: aligned rows and scoped draft reset | Shared viewer-bar group |

No tool overlay, generic Details, filename/coordinate panel or placement chevron remains. Active geometry sections temporarily expose their bodies without overwriting expansion preferences; ordinary orientation actions stay visible/disabled. No orientation-in-draft behavior is added.

Tabs may hide geometry settings, but retain draft/frame, view and the shared bar group. No implicit Apply/Cancel on tabs or outside clicks. Apply remains present but disabled without a proposal; non-cancellable measurement/acceptance never offers false cancellation. Numeric/popover precedence, view restoration and existing transaction boundaries remain unchanged. Viewer-bar, inspector and specific-popover interactions cannot also manipulate the canvas; gestures remain in help/tooltips.

**Viewport stability is not “crop never changes scale”.** Crop deliberately Fits the full source with handles on entry; Apply Fits the accepted composition; Cancel restores the prior viewport. Dmin/Dmax reveal uncropped oriented source; flat-field shows an untransformed reference. Preserve those semantics. The proposal removes incidental scale/size changes caused by UI rows only. Gamma/WB retain current grading and deliberate in-picker navigation. Invalid/cancelled picks change no processing state.

### Menus and dialogs

Native integration is agreed for the **application menu bar only**; context menus and inspector dropdowns remain egui. Native macOS application menus are feasible, but not currently implemented. Native file dialogs already exist. Recommend a focused main-thread menu adapter in `src/platform`, not rewriting the host or adding a general command framework. Keep editor actions platform-neutral: future Windows menus would normally attach to a window, while Linux integration depends on its desktop/toolkit. No Windows/Linux backend or support claim is added by this pass. [Feasibility and options](../research/ui-polish-audit.md#native-menu-feasibility).

- **File:** New, Open, Add RAWs, Save, Save As, Export submenu (TIFF / EXR half / EXR full), Close. Preserve standard shortcuts and destination/dirty guards.
- **Edit:** supported text-edit actions with correct field focus. Do not offer working-looking undo/redo for document edits until implemented. Determine native responder versus egui text routing in the menu spike.
- **View:** Fit, 100%, zoom, developed source, scope visibility. Check state mirrors the same editor state, not a second menu copy.
- **App / Window / Help:** standard supported macOS commands, navigation help. Don't create empty Settings, Help or Window commands solely for appearance.
- Keep frequent, visual manipulation controls near the image/parameter. Menus provide discoverability/shortcuts and secondary commands; don't move crop, calibration or numeric parameters exclusively into menus.
- Context menus are optional secondary access, never the only route. Do not add unsupported remove, presets, copy/paste grades, compare or batch commands in this pass.

Native menu and egui buttons/shortcuts must invoke the **same guarded action path once**, after accepted numeric edits. Verify busy/dirty/draft validation, app focus, Cmd-Q/Close, key-equivalent double firing and repaint wake-up. Native Quit must not bypass unsaved protection. Remove duplicated in-window menus only after this works. Native menu styling does not make egui popups automatically native; inspector popups and context menus stay egui in this pass.

Use one consistent centered, owner-relative modal pattern for Save/Discard/Cancel and replacement decisions, with explicit focus/default/cancel handling and an inert background. Existing egui Windows disable the editor but are not yet a complete modal/focus policy. Prefer a real attached sheet when native dialog integration supports it; otherwise an egui modal is a scoped alternative. Crop/pick modes are not modal confirmation dialogs. File-dialog cancellation remains a no-op.

### Feedback and status policy

Feedback belongs to its owner. The global footer is not a log and not an instruction manual.

| State | Where / lifetime |
| --- | --- |
| Main RAW developing | Viewer-owned spinner and concise filename/phase; keep canvas geometry |
| Thumbnail queued | Stable neutral placeholder with filename; no claim that it is actively developing |
| Thumbnail developing | Spinner in that cell; retain filename/selection; errors replace spinner with marker/details |
| Settled preview refining | Quiet viewer-bar spinner/quality state; no global “ready” announcements |
| Scopes updating/suspended | Scope header/caption; retain approximation and target identity; never show stale incompatible bins |
| Pick/crop instructions | Viewer help/tooltips, not another tool panel or global footer paragraphs |
| Rejected value/pick | Visible near the owning control/tool with concise cause + details; keep draft/accepted state rules |
| Save/Open/export | Global operation/issue slot with concise phase; determinate only if measured |
| Saved/exported | Brief completion acknowledgement; don't erase an unresolved failure |
| Unsaved project | Document title/header only, independent of job feedback |

The footer can retain current photo name/index and dimensions plus **one global operation/issue slot**. Local development/preview state should not be repeated there. Background thumbnail/refinement completions never overwrite foreground issues. Priority: unresolved actionable failure → foreground operation → short completion → neutral/empty. Retain failures until dismissed or meaningfully resolved, not until any later worker message; keep longer diagnostics in an expandable detail view. Avoid a new notification center/history subsystem.

Use spinners for opaque work (LibRaw); no invented percentages. Queued is not running. Measurable phases can later supply progress in a fixed-size operation slot. Existing export and LibRaw phases are not safely mid-phase cancellable; UI polish must not imply otherwise. Animate only visible active indicators and stop repaint requests when idle; keep the worker polling needed for results. Share the small presentation vocabulary with [operation feedback](completion.md#operation-feedback), without expanding to batch export/cancellation in this pass.

### Visible copy versus details

Keep titles, values/units, active modes, missing calibration, source/reference identity, unsaved state, target/approximation, safe-decision text and actionable warnings visible. Move repeated gestures, camera-model internals, color conversion prose and coordinate nomenclature to tooltips/details.

Shorten rather than simply hide consequential text. Flat-field setup requirements appear when choosing a reference; after a correction change, retain a concise “Correction changed — check calibration” notice. Dmin/Dmax are not automatically resampled, and no new persistent calibration-validity flag is implied. Do not claim a reference is physically suitable merely because metadata passed. RGB reference numbers can live behind a details disclosure while “Measured / Not measured” remains visible.

Use user-facing picker names instead of Rust debug enum labels (`Wb`, `Flat`), consistent ellipsis only when a command opens another step, and specific reset tooltips. Disabled controls should explain prerequisites where useful; don't bury the only recovery action in a tooltip.

## Implementation slices and acceptance

Slices 1–3 are implemented; checkpoints are below. Remaining priorities are proposed, not approval to execute the whole package.

| Slice | Focus / likely files | Acceptance |
| --- | --- | --- |
| 1 — Shared visual primitives | Icon asset/license, sections, headers, flatten nested color card; `editor_controls`, crop/orientation call sites | Consistent focus/hit areas; disclosure/reset independence; hidden edits unchanged; all numeric/reset/default tests retained |
| 2 — Workspace hierarchy | True Roll/Photo/Rendering tabs, Output relocation, separate scopes chrome, application UI preferences; `editor`, `editor_scopes`, preference storage boundary | All controls reachable at 315×800 / 270×600 inspector sizes; tab changes never mutate grades/drafts; target still matches preview/TIFF/scopes; preferences round-trip without project changes; New from Photo/Rendering always selects Roll |
| 3 — Stable viewer and tools | Bottom viewer bar + unified tool identity/actions and inspector-owned geometry settings; `editor`, `editor_crop`, `editor_flat_field`, navigation | Same canvas rectangle before/after mode UI; intended crop/source transforms retained; no overlay input fall-through; narrow mode controls stay reachable |
| 4 — Feedback | Visible loading spinners, local preview/scope state, deliberate footer/issues; worker/UI event boundaries | Queued/running/error distinct; no orphan/perpetual spinner; foreground errors survive background completion; no fictitious progress/cancel |
| 5 — Native shell | Menu adapter + title + centered/modal guard dialogs; `platform/host`, `editor_files`, `editor` | Main-thread lifecycle/wake-up; command once; field focus; menu enabled/check parity; dirty Close/Quit and overwrite guards verified |
| 6 — Copy and coherence closure | All tabs/modes/dialogs/strip; relevant contracts + architecture | Compare normal/narrow screenshots, tab/focus order, tooltips, source/target states and physical pointer/trackpad/VoiceOver limitations |

Native menus are worthwhile but have more integration risk than icons/tabs; perform a small menu feasibility spike before committing to an adapter/dependency, not as a prerequisite to every layout improvement. Spinner work may land earlier as an independently useful slice after its owner/state definitions are agreed. Existing thumbnail clipping/latency and hand-cursor defects remain in their current plans, not silently absorbed here. The previously triaged New-tab issue is now explicitly covered by the agreed New-always-selects-Roll requirement in slice 2.

### Verification matrix

For each implemented slice, run the narrow existing editor/control/crop/navigation/scope/thumbnail/file tests plus focused new event/layout tests. Use [the runbook](../build-and-probes.md); no full numerical suite simply for cosmetic edits.

Before closing the pass, inspect real rendered views at normal and narrow windows, Retina scale and reduced inspector/filmstrip space:

- empty project, uncalibrated RAW, calibrated Roll/Photo/Rendering, long project/source/reference names;
- scopes hidden, waveform/vector, resizing, updating/suspended/error states;
- developed source with nonzero inspection EV, Fit/native navigation and precision picking;
- Dmin/Dmax temporary reference, Gamma/WB pick, flat-field proposed/manual support;
- crop entry, every handle/angle/aspect, tab switch while draft active, Apply/Cancel and numeric focus;
- queued/developing/failed thumbnails, load failure, rejected edit, preview failure, Save/Open/export completion/failure;
- dirty New/Open/Close/Quit and output/project replacement, keyboard-only traversal, modal focus and icon names.

Capture only the app window; photographs stay ignored/private. Headless tests establish state/layout, not trackpad feel, physical monitor accuracy or VoiceOver acceptance.

## Agreed direction and remaining choices

The user endorsed the overall workspace/interaction direction and explicitly confirmed:

1. Remember section expansion, scope visibility/size and filmstrip height across launches as application preferences. **New projects always start on Roll.**
2. Adjusted sections remain collapsed with a non-default marker and concise summary; do not automatically open them.
3. Native integration concerns the **menu bar only**, not context menus or inspector dropdowns. Future Windows/Linux support remains separately scoped.

Icon artwork, spacing, first-run section defaults and compact-overlay details are implementation choices to validate visually. The first two slices were explicitly selected/authorized and are recorded below; the user has now authorized continuation into slice 3. Later slices still require selection before expanding application-code changes. Update `docs/contracts/editor-controls.md` (or a linked UX contract if it becomes too large) as each slice changes implemented behavior; keep accepted design intent here until then so current layout contracts aren't mistaken for delivered polish.

## Progress / handoff

- Created the requested isolated worktree; left `main`'s modified feedback inbox and untracked `.pi/` untouched.
- Inspected current code/contracts/tests and normal/narrow historical scope/control captures. Captures predate integrated geometry and are not fresh screenshots of this baseline.
- Compared accessible primary documentation from darktable, Affinity Photo and Apple; examined native-menu library documentation. Blocked Adobe/Capture One/RawPedia pages are not treated as verified comparisons.
- Authored the initial proposal/evidence note/synthetic wireframe and recorded the user's design alignment and explicit product answers. Quick Look wireframe generation was sandbox-blocked; the SVG is structurally checked, not a rendered app capture. Initial audit performed no app changes; subsequent implementation is distinguished below.

### Slice 1 checkpoint

Implemented in `src/{editor_icons,editor_controls,editor,editor_crop,editor_flat_field,editor_scopes}.rs`:

- One project-authored outline vocabulary with 28-point targets, tooltips, named widget metadata and interaction/disabled states; reset, pick, turn/flip, crop, aspect swap and guides use it. Transactional Apply/Cancel stay labelled.
- Shared separator/disclosure/reset headers, per-scope session expansion, live collapsed summaries/non-default markers, visible missing-calibration state and multiple independently open sections. No project/schema/processing changes. Cross-launch preferences and New selecting Roll remain slice 2, not falsely reported complete.
- Removed the nested Gamma balance card while retaining independent balance/parent resets and existing wheel/numeric semantics. Crop drafts cannot hide their Apply/Cancel controls through ordinary Composition collapse; normal expansion is restored afterward.
- Regression tests cover pending numeric cancellation/reopen, section/reset independence, stable IDs, separate Roll/Photo expansion, project/dirty neutrality, keyboard activation and accessibility metadata, missing calibration, draft visibility and disabled icon rendering. Larger targets initially shifted the scope plot between modes; equal-height rows fix that invariant without scope-processing changes.
- Updated the implemented [editor control contract](../contracts/editor-controls.md#shared-visual-language). Source tests locate icon actions by generated semantic metadata rather than Unicode glyph text.

Verification snapshot: **102 focused editor tests passed, one real-RAW integration test intentionally ignored**; formatting, strict editor-feature library/test Clippy and debug editor build passed. [Environment, commands and retained summary](../research/evidence/ui-polish-controls.txt). This is not a full numerical/export or broader-device acceptance run.

The controls-only real-source/window smoke passed its unchanged nonzero-visible-frame gate (**378 submitted frames**, four normal/narrow holds). One Roll-normal app-window screenshot was captured and inspected; Photo/remaining narrow captures could not retrieve window metadata after visibility/occlusion changed. That screenshot also exposed overly bright disabled icon artwork; painting inside the disabled UI scope plus a shape regression fixed it afterward. Final disabled rendering is headlessly verified, not freshly recaptured. Private inputs/captures/logs stay under ignored storage.

The earlier optional scopes-readback smoke was stopped after 120 seconds under occlusion, before layout holds. Its unmodified wait can return before requesting another repaint while no scope GPU callback is submitted; the controls-only retry omits that optional wait, not the visible-frame gate. No host/smoke workaround was committed. Combined scope/window acceptance, full Photo/narrow/crop screenshots and physical keyboard/pointer/VoiceOver review remain open.

Slice 1 was subsequently committed as `d1cd936` at the user's request; the user then authorized slice 2. The snapshot above remains historical evidence, with later verification distinguished below. No push or main-worktree edits were made.

### Slice 2 checkpoint

Implemented workspace hierarchy and application UI preferences:

- Equal-allocation **Roll → Photo → Rendering** tabs with visible selection, scope tooltips and tested keyboard/AccessKit tab metadata. One tab-specific scroll region contains normal orientation/Composition and grading controls. Rendering owns the matching shared Target selector/reset and 100-nit preview/TIFF note; the existing viewer row and scope caption retain target identity. No processing/output/project schema changes.
- Distinct shaded scopes region, independent chrome/divider and stable inspector reservation across tabs/drafts. Scope visibility and width-relative size, symbolic per-scope section expansion and filmstrip preferred height persist separately. Window limits do not overwrite preferred size. Adjusted sections remain collapsed; crop drafts do not overwrite ordinary Composition expansion.
- `editor_preferences.rs` owns version-1 application JSON, validation, defaults and same-directory atomic writes. Normal launch loads it; the host's small exit hook saves changed preferences after the event loop ends. Read/version/write failures are diagnostic and cannot dirty or block a project; unreadable/malformed/future files are left unchanged. Tests/smokes use defaults or temporary stores, not the user's settings. Inspection EV, view, drafts, tab and scope plot/scale are not persisted.
- Successfully prepared New selects Roll and opens missing Calibration only, irrespective of prior Photo/Rendering selection. Cancelled/failed/stale New leaves the previous tab/document intact. Initial RAW startup creates a new Roll project; ordinary imports, Open and photo switching keep existing tab semantics.
- Targeted regressions cover three-tab state/view/draft neutrality, independent reset/selector and crop guards, actual normal/narrow scrolling reachability (including crop Apply/Cancel), fresh-context preference restore/project neutrality, atomic storage/defaults/rejection and constrained filmstrip sizing. Fractional tab-button allocations initially caused slight sidebar growth and changed a frozen crop Fit scale; a single bar allocation fixes the source without changing the existing tolerance or viewport code.

Verification snapshot: **115 focused editor tests passed, one real-RAW integration test intentionally ignored**; strict editor-feature library/test Clippy, formatting and debug build passed. [Environment, commands and retained evidence](../research/evidence/ui-polish-workspace.txt).

The real-source controls/window smoke passed: **1,722 submitted frames**, reported elapsed **38.10 s**, six normal/narrow Roll/Photo/Rendering holds. All six final app-window captures were inspected: selected stage, readable tabs/target, separated scopes and no repeated pinned Output. Inspector overflow remains scrollable; it is not a claim that all expanded controls fit onscreen simultaneously. Captures also include the final disabled-icon rendering from slice 1. Private photographs/logs stay ignored.

The first six-hold attempt kept four seconds per hold and exceeded the existing 45-second smoke deadline; its Rendering captures were stale under occlusion and are not acceptance. The corrected holds use 2.5 seconds each, sharing the original wait budget, and the added Rendering holds use the same focus request as existing holds. Timeout and nonzero-visible-frame gates remain unchanged; no numerical/host occlusion workaround was added. The retry passed and captured the correct stages.

Limits: no physical keyboard/trackpad/VoiceOver, macOS-14/other-hardware/monitor acceptance, fresh crop/picker window captures, or normal-app manual relaunch acceptance. Preference round-trip/shutdown wiring is verified with temporary stores/headless editor contexts, not by changing the user's settings. The optional combined scope-readback smoke was not repeated; visible vectorscopes are not full photographic/numerical scope acceptance.

The user visually accepted slice 2, committed as `a2dddd5`, and authorized **slice 3 — stable viewer bar and common contextual tool overlay**, implemented below with the outstanding acceptance retained. No push/main integration or native-menu work was performed.

### Slice 3 checkpoint

The user requested continuation after accepting the workspace and asked that scopes be collapsible/hideable (timing flexible). The scope presentation follow-up remains recorded for slice 6; checkbox hiding/persistence already works.

**Initial presentation (superseded by the revision below):** the following implementation/checks predate the user's rejection of the generic Details and bulky picker layout. They are not current visual acceptance.

Implemented in `src/editor_viewer.rs`, with focused editor/crop/icon integration:

- Fixed 40-point viewer-width bottom bar above the filmstrip: named Fit/zoom/source/inspection/help icons, displayed zoom percentage with physical 100% action, visible Source ±EV, target and quiet Approximate/Refining quality. Empty/loading/error, positive/source and tool states retain the same canvas rectangle. Removed ad hoc picker/header/coordinate height subtraction and the permanent preview-ready row.
- One nonmodal in-canvas overlay for crop, Dmin/Dmax, Gamma/WB neutral and flat-field support, independent of inspector tabs. Named title/reference identity, Details and consistent Cancel/primary Apply; unselected proposals cannot apply, and non-cancellable measurement/acceptance disables unsupported actions. Wide crop controls are inline; narrow controls, angle slider/dimensions, coordinates, reference selection and instructions remain in Details. Long identity truncates without enlarging the canvas.
- Explicit chevron moves the toolbar between bottom/top edges to expose covered source pixels and handles without reframing. This is session state, not a durable preference or project field. Normal inspector Composition retains an active-draft summary, not a second transaction toolbar.
- Overlay/popovers own pointer/drag/wheel/touch regions, including gestures begun there. Enter/Escape retain numeric priority; popovers remain open during control clicks. Crop angle and picker coordinate text now commit only on Enter/focus loss and cancel on Escape, with value comparisons preventing egui's pending-text changed flag from reconstructing/re-rounding geometry. Existing crop/source transforms, exact view restoration and worker/processing/resource guards remain intact.

Verification snapshot: **121 focused editor tests passed, one real-RAW integration test intentionally ignored**; strict library/test Clippy, formatting and debug build passed. Tests retain existing crop Fit tolerance and navigation/measurement/return invariants, and add canvas/bar stability, every mode/tab normal/narrow reachability, popup input exclusion, numeric precedence, disabled Apply/Cancel, explicit docking and bottom-handle resize. [Environment/reproduction/evidence](../research/evidence/ui-polish-viewer.txt).

The guarded real-source tool/window smoke passed: **1,755 submitted frames**, **38.97 s** reported elapsed, six normal/narrow crop/Dmin/WB app-window captures inspected. Crop remained active on Roll, picks on Rendering, source +2 EV was visible, and controls stayed above the viewer bar. An earlier unguarded capture showed an unexpected applied crop during the narrow hold (consistent with external input, not conclusively attributed); it is not acceptance. The smoke now fails if the expected tool mode exits during a hold, rather than masking that interference or relaxing its existing timeout/visible-frame gates. Private photographs/captures/logs remain ignored.

Limits: Details/docked position are event-tested, not freshly captured in a native window; Gamma/Dmax/flat-field overlay states are headless-covered, not new real-reference photographic acceptance. Physical keyboard/pointer/trackpad/VoiceOver, broader hardware/macOS-14/monitor, manual preference relaunch and existing composed-export/photographic acceptance remain open. Native menus, broader feedback/footer policy and disclosure-style scopes refinement are not implemented here.

### Slice 3 presentation revision

The user rejected the generic Details approach, duplicated crop popover and bulky simple-pick overlay. This is historical geometry presentation: the user subsequently accepted its pickers but rejected crop/flat-field overlays, superseded below. The user approved removing precise selection fields and tool filenames, removing external calibration-reference selection in favour of roll selection, hiding source inspection outside source view, retaining Composition controls, and clarifying viewer/bar ownership; background treatment was left to UX judgment.

Implemented the agreed revision:

- Compact content-sized pick rows; no Details, tool filename or x/y/w/h entry. Crop aspect/angle/guides/reset stay visible; a second controls row handles narrower canvases. Specific popovers/help remain, with no duplicated crop controls.
- Dmin/Dmax switch only by registered ordered roll ID through the filmstrip. Removed the external chooser and unregistered temporary-load branch; exact working-photo/selection/view return remains. Flat-field's independent bare-light chooser and reference-only registration are unchanged.
- Source inspection control/value disappear in positive view while retaining EV. Viewer surround/bar share a dark surface; lighter filmstrip/top divider distinguish roll ownership without changing canvas dimensions.
- Composition retains its ordinary actions and expansion during crop editing, with disabled incompatible actions and active Crop indication. No orientation-in-draft behavior was added.
- Inline numeric precedence now handles egui 0.36 clearing text focus before Escape dispatch and its retained second lost-focus commit; cancelled angle text neither exits the mode nor alters geometry. Tool/popup drag, scroll, touch and keyboard ownership remain guarded.

Revision verification snapshot: **125 focused editor tests passed, 0 failed, one real-RAW integration test intentionally ignored**, strict Clippy/formatting/debug build passed. Guarded real-source smoke: **1,905 submitted frames**, **39.59 s**, all six normal/narrow crop/Dmin/WB app-window captures inspected. Added headless reachability at 700×600, including explicit crop wrapping. [Revision evidence/reproduction](../research/evidence/ui-polish-viewer-revision.txt). The first revision smoke stopped at a relative-versus-resolved locator assertion; its corrected check verifies path equivalence and registered source identity, without relaxing measurement/view gates.

Limits remain: user visual acceptance pending; physical pointer/trackpad/keyboard/VoiceOver, manual normal-app preference relaunch, broader hardware/monitor and composed export/reopen acceptance are not inferred. Composition-during-crop, Gamma/Dmax/flat-field, docking and specific popover states are headless/event-covered, not new native-window captures.

### Slice 3 geometry placement revision

Historical picker presentation, superseded by the shared tool bar below; geometry settings retain these homes.

The user said “Pickers are good now”, rejected crop/flat-field boundary obstruction and stair-stepped crop controls, agreed on their existing inspector homes plus viewer-bar Apply/Cancel, then authorized implementation (“Do it”).

Implemented:
- Crop controls in Photo → Composition, with light active-mode tint/title, aligned Aspect/Angle/Guides rows and a scoped draft reset in the mode heading. Ordinary orientation actions remain visible/disabled.
- Flat-field area identity/progress in its existing Roll section; separate bare-light selection/support/acceptance unchanged.
- Geometry Cancel then primary Apply at the far right of the fixed 40-point viewer bar, available on every tab. No geometry overlay or docking action. Navigation/inspection/help lead; target/quality or geometry actions trail with flexible space.
- Geometry sections temporarily expose their bodies without changing saved expansion. Tabs may hide settings but preserve draft/frame, view and actions. Accepted pick overlays are unchanged.
- Regressions cover all modes/tabs at 1100×800, 800×600 and 700×600, actual bar hit bounds, aligned rows, draft/preference neutrality, busy/unselected guards, Apply/Cancel dispatch and numeric cancellation across hidden/reopened Composition. Painted-label test bounds now include egui's aligned-galley offset; no geometry/numerical tolerance was relaxed.

Verification snapshot: **129 focused editor tests passed, 0 failed, one real-RAW integration test intentionally ignored**; strict Clippy, formatting and debug build passed. Fresh six-hold crop/Dmin/WB smoke and two real flat-reference layout holds passed; window-only captures inspected. [Exact commands/results and limits](../research/evidence/ui-polish-geometry-tools.txt). Geometry captures explicitly hide scopes to expose inspector settings; this is diagnostic capture setup, not automatic app behavior. Flat-reference layout capture cancels without accepting a map/exporting.

Limits: user visual acceptance of geometry placement pending. Physical keyboard/pointer/trackpad/VoiceOver, normal-app preference relaunch, broader hardware/monitor and composed export/reopen remain open. Gamma/Dmax and specific popup states remain event-covered rather than freshly captured. Native shell, broader feedback and disclosure-style scopes remain unselected.

### Slice 3 shared tool bar

The user clarified that every image tool should have the same “accept what you're seeing” Apply/Cancel UX. After actions moved into the bar, they rejected the remaining floating title box and explicitly authorized removing it, with visually coherent active-tool identity.

Implemented one trailing tinted group with recognizable icon, tool name, Cancel and primary Apply for crop, Dmin/Dmax, Gamma/WB and flat-field. A real busy spinner replaces the icon; existing numeric/proposal/busy/return guards remain. Removed floating area, chevron/placement state and artificial overlay hit exclusion. At constrained widths auxiliary zoom ±/inline EV yield to identity/actions; Fit/100%, inspection and help stay reachable. Crop/flat-field settings keep their inspector homes. Normal target/quality return after exit. The latest requested padding trial increases the name→Cancel gap from 4 to 12 points, retaining tight button spacing, right alignment and bar height. Its 15 focused viewer tests, debug build, formatting/diff checks and fresh normal/narrow flat-field captures passed; both captures were inspected (`.cache/probes/ui-polish/tool-bar-padding-flat/`). The six-hold capture limitation below remains open.

Fresh focused snapshot: **130 editor tests passed, 0 failed, one intentionally ignored integration test**; strict Clippy, formatting and debug build passed. Tests cover tinted identity/action ownership, accessibility, all modes/tabs and narrow hit bounds, input ownership, numeric cancellation and actual worker/return transactions. [Reproduction/results](../research/evidence/ui-polish-tool-bar.txt).

Native capture limits: crop normal/narrow and Dmin/WB normal captured/inspected; six-hold run timed out after crop-narrow, so remaining narrow picker holds are not acceptance. Earlier attempts logged occlusion; this latest log did not, so its cause remains unestablished. The separate real flat-reference layout route passed and both captures were inspected. No timeout/visibility/numerical gate was relaxed. Physical input/VoiceOver, normal-app preference relaunch and broader photographic/hardware checks remain open.

**Next action:** user visual review of the shared tool group; diagnose/complete the remaining narrow picker native captures before packaging. Slice 3 remains uncommitted; no push/main integration or automatic continuation into later slices.
