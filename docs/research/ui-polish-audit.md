# UI polish: audit and comparative evidence

Baseline: `0beee3e`, inspected in `.worktrees/ui-polish` (`plan/ui-polish`). This is a code/documentation and historical-image audit, not a fresh runtime usability study. The [design proposal](../plans/ui-polish.md) owns recommended UX language, priorities and approval questions. No processing changes follow from these observations.

## Current UI inventory and findings

Line references describe this baseline and will drift after implementation.

| Area / evidence | Observed behavior | Design consequence |
| --- | --- | --- |
| Shell: `src/editor.rs:1551–1625`; `src/editor_files.rs:83–155` | In-window File/Export menus, repeated app heading, document name/Modified, plain bottom status label; native file dialogs but egui confirmation Windows | Native menus can recover content space; document identity matters more than repeated branding. Confirmations need deliberate placement/focus, not merely new colors |
| Inspector: `src/editor.rs:1237–1402` | Pinned Output card; scopes; selectable Roll/Photo labels; pinned geometry; remaining scrollable edits | Several visually similar layers compete for scarce vertical space. Tabs resemble selected buttons. Output is always expensive even when not being edited |
| Section helper: `src/editor_controls.rs:12–39,243–260` | Every section framed and always expanded; color component adds another section inside Gamma & balance | Disclosure and flatter hierarchy improve density without changing operators. Preserve independent nested reset semantics even if nested frames disappear |
| Viewer: `src/editor.rs:1640–1712` | Source checkbox, optional reference/picker instructions, separate preview-state row, Fit/100%/−/+ and conditional Inspection popup | Three top rows compete with the image; descriptions wrap and alter available layout. Bar should have stable dimensions and grouped roles |
| Picker: `src/editor.rs:1713–1716,1880–1904` | Reserves 36 points beneath canvas; rectangle numeric fields and Apply are emitted after the image; Cancel/reference chooser are above it | Mode actions are spatially split; use one recurring contextual toolbar. Don't infer that all current modes literally put buttons at the bottom |
| Crop: `src/editor_crop.rs:150–350`; `src/editor.rs:1269–1272` | Composition section switches to aspect/angle/guides/dimensions/Reset/Cancel/Apply; only visible on Photo | Crop differs from picker layout; switching to Roll hides its controls without cancelling. The common overlay should stay available independent of tab |
| Flat-field: `src/editor_flat_field.rs:60–122` | Reference selection, file name, Enabled/Edit area/Clear and permanent multi-line setup/calibration explanation | Retain consequential setup/recalibration guidance contextually; repeated paragraph need not occupy the idle inspector |
| Scopes: `src/editor_scopes.rs:48–148,165–193,272–286` | Checkbox, plot selector, vector scale or RGB/Luma, plot/caption and divider; above controls with reserved geometry/Output heights | A separator exists already, but region identity/control hierarchy is weak. Separate chrome, don't rebuild scopes or move them into a viewer-size-changing region |
| Thumbnails: `src/editor_thumbnails.rs:368–379,406–429` | Static ellipsis for absent image, `!` for error, filenames retained; requests serialize and wait for foreground/settled work | Queued and actually developing are different states. Spinners alone don't fix scheduling/latency. Preserve bounded caches and existing priority |
| Feedback: `src/editor.rs:733,807,843,922–943,1022–1069,1178,1217,1624`; `src/editor_crop.rs:117–133` | One status string mixes work, results, rejection, reference advice and crop instructions; local preview text exists too | Define owner, priority and lifetime before cosmetic styling. Refinement success already avoids overwriting prior errors; extend that principle, don't claim every completion currently erases errors |
| Icons/tooltips: `src/editor_controls.rs:42–44,109–145`; crop/zoom/scope call sites | A few Unicode reset/swap/zoom glyphs; many long action labels; parameter tooltips already describe numeric/fine control | Use a consistent icon vocabulary for familiar actions, retain text for scope/parameters/transaction decisions. Audit keyboard/accessibility, not only hover copy |

### Visual evidence actually inspected

Two private historical captures from `.worktrees/scopes/.cache/probes/`:

- `scopes-polish-roll-normal.png`: right-side plot and controls have similar chrome; the flat-field paragraph and pinned Output card leave little room for Calibration; three viewer rows and repeated app branding are visible; thumbnails show static ellipses.
- `scopes-polish-photo-narrow.png`: large scope plus pinned Output leaves only a sliver of White balance; Roll/Photo are small selectable pills rather than inspector tabs; normal completion text remains in the global footer.

These captures **predate integrated orientation/crop panels**. Current source confirms those additional pinned controls, but their exact onscreen appearance was not freshly captured here. Diagnostic photograph color is not evaluated by this UX audit. Images remain ignored/private and are not copied into authored documentation. No app launch, external photo-editor automation, new sample processing, permission bypass or physical trackpad exercise was performed.

### Important constraints from current contracts

[Editor controls](../contracts/editor-controls.md), [geometry](../contracts/geometry.md), [scopes](../contracts/scopes.md), [output](../contracts/output.md) and [project state](../contracts/project-state.md) are implemented contracts, not suggestions.

- Current pinned Output and tab-scoped pinned geometry were previously accepted. Relocation is a new proposed layout, not a code defect.
- Roll/Photo is shared/per-image scope, not an exhaustive operator graph. Composition follows canonical source geometry, while full-resolution measurements/export must retain their coverage and coordinate contracts.
- “Developed source” is accurate: RAW development has already occurred. Inspection EV is transient, bypasses grading and does not affect calibration/export.
- The target is a shared 100-nit ACES preview/TIFF choice, with scopes in that target's native encoded basis; presentation then converts to managed extended sRGB. It is not a display-device profile selector. AP0 EXR remains scene-linear and bypasses the rendering target.
- Crop's intentional full-source Fit/handle margin and Apply/Cancel view restoration are distinct from unwanted canvas changes caused by inserted UI.
- Missing calibration, approximate preview/scope data, physically unsuitable references and actionable rejection cannot become tooltip-only information.

## Comparative primary sources

Sources were read as public documentation, not through application automation. They support specific patterns, not a claim that all professional editors follow one standard.

| Source actually read | Verified pattern | What to borrow / what not to infer |
| --- | --- | --- |
| [darktable — Darkroom layout](https://docs.darktable.org/usermanual/development/en/darkroom/darkroom-view-layout/) | Right panel has scopes, module groups/search, processing modules and module-order control; bottom panel has inspection/overlay toggles, with filmstrip below; scopes can be moved left | Supports bottom viewer-adjacent inspection tools and region separation. Doesn't establish that Antitone needs dockability, a left sidebar or every darktable toggle |
| [darktable — Module header](https://docs.darktable.org/usermanual/development/en/darkroom/processing-modules/module-header/) | Expand/collapse, reset and other management actions share module headers; opening one normally collapses another, Shift allows multiple | Borrow consistent disclosure/reset headers; reject automatic accordion closure for Antitone's smaller control set unless users request it |
| [darktable — Module groups](https://docs.darktable.org/usermanual/development/en/darkroom/organization/module-groups/) | Workflow/category grouping and reduced beginner sets coexist with separate processing-order concepts | Tabs can communicate useful workflow without pretending to be a literal graph. Don't copy large-editor module search/preset infrastructure |
| [Affinity Photo 2 — Developing RAW](https://affinity.help/photo2/en-US.lproj/pages/Raw/raw.html) | Dedicated Basic/Tones/Details/Lens panels, Scope panel, preview comparison/clipping toolbar controls; “Before” comparison is separate from output | Supports inspection versus adjustment separation and labelled domain groups. Antitone's source bypass is not a new Before/After feature; no alternate RAW engine/default processing is implied |
| [Affinity Photo 2 — Crop tool](https://affinity.help/photo2/en-US.lproj/pages/Tools/tools_crop.html) | Context toolbar groups Apply/Cancel, aspect modes, dimensions, straighten, overlay, reset and outside-crop darkening | Strong precedent for one coherent crop action cluster. Its toolbar is not evidence for an overlay or guaranteed fixed canvas; those are Antitone-specific proposals |
| [Apple HIG — Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars) | Deliberate selection, logical groups, recognizable symbols except ambiguous actions, one prominent trailing primary action; on macOS, toolbar commands also belong in menu bar | Supports icons with clear meanings and restrained hierarchy. Apple's native macOS toolbar sits at the top; the proposed bottom bar is a photographic view-control adaptation, not a literal NSToolbar prescription |
| [Apple HIG — Menu bar](https://developer.apple.com/design/human-interface-guidelines/the-menu-bar) | Familiar ordering/short titles, standard shortcuts; unavailable actions remain visible but disabled | Supports native-menu conventions and state parity. Does not provide automatic egui text/responder integration |
| [Apple HIG — Progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators) | Consistent location, concise accurate context, determinate only when possible, cancellation when feasible; avoid shape/layout disruption during progress transitions | Supports owner-local feedback and truthful spinners. Does not make LibRaw/GPU/export phases interruptible or justify invented percentages |

Version/evidence limits: darktable pages rendered the **Development / 5.6** manual; requested older version paths did not yield verified 5.4/5.2 documentation. Affinity sources are the Photo 2 desktop help. Apple pages are current guidance, including newer system styling; borrow durable behavior/hierarchy rather than adopting Liquid Glass or raising Antitone's macOS baseline. Apple material was read through rendered pages because direct fetches sometimes returned empty content.

Adobe Lightroom/Camera Raw pages returned HTTP 403; Capture One tool-tab documentation hit access verification; RawPedia did not yield usable article content. No specific pattern is attributed to those unread pages, and they are not needed to justify the proposal. Apple tab-bar guidance is primarily app-area navigation; it is not used as a requirement for this desktop inspector's three tabs. Tooltip/accessibility recommendations in the proposal are design requirements to validate, not claims extracted from an unread HIG tooltip page.

## Native-menu feasibility

Current stack: pinned egui/egui-winit/egui-wgpu **0.36.2**, winit **0.30.13**, rfd **0.17.2**; a custom host in `src/platform/host.rs` owns the event loop, repaint proxy and app (`run`, `Host`, `App`). Existing Objective-C dependencies configure Metal/ColorSync presentation, not AppKit menus. No native application-menu adapter is currently wired.

[**muda 0.21.2 documentation**](https://docs.rs/muda/0.21.2/muda/) describes a macOS AppKit backend, `Menu::init_for_nsapp`, main-thread-only use and forwarding menu events to winit through `EventLoopProxy` so idle apps wake. Its documented development example dependency is winit 0.30. This is evidence of feasibility, **not a compiled compatibility check or approval to add the dependency**. No download/install/lockfile change was performed.

Options:

- **Small menu library adapter:** less direct AppKit lifecycle/selector work; new pinned dependency and transitive/license review. Preferred spike candidate if it preserves standard macOS behavior with this host.
- **Direct `objc2-app-kit` / NSMenu bridge:** keeps the integration macOS-specific and gives exact native control; requires selectors, retained lifetime, main-thread/responder correctness and more custom unsafe/native maintenance. Not automatically simpler because Objective-C crates already exist.

Either path must route into current typed file/export actions and existing guarded state; don't build a generic command bus for cosmetic polish. Verify menu enable/check updates, numeric text commits, key-equivalent duplication, Close/Cmd-Q dirty guards, modal routing and event-loop wake-up. Native menus and file dialogs don't imply a native inspector/toolbar rewrite.

## Verification and unresolved evidence

Inspected `src/editor{,_controls,_crop,_scopes,_files,_flat_field,_thumbnails,_navigation}.rs`, desktop host/dependencies, relevant contracts and test entry points. Existing regression homes: `editor_controls_tests`, `editor_crop_tests`, `editor_navigation_tests`, `editor_scopes_tests`, `editor_thumbnails_tests`, `editor_files_tests` and broad `editor_tests`; onscreen holds live in `editor_smoke`/`editor_flat_field_smoke`.

No code tests or fresh UI smoke were run because authored changes are documentation/wireframe only. Link/file checks and diff inspection belong to the proposal handoff, not runtime acceptance. Still needed after agreement: fresh integrated-geometry screenshots, rendered icon/readability checks, focus/VoiceOver feasibility, narrow overlay hit testing, physical pointer/trackpad behavior, and compiled native-menu spike. Scope/monitor/photographic accuracy remains separate from layout quality.
