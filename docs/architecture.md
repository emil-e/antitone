# Architecture

> Express relationships in the program's structure instead of repeatedly maintaining them through procedure.

Correctness should follow naturally from the shape of the program, rather than from everyone remembering to coordinate things correctly. These principles guide design decisions; existing implementation is not an exemption from them. [Development](development.md) covers engineering and verification; the [topic guides](README.md) explain photographic and feature behavior.

## Fundamentals

### Give responsibilities meaningful owners

Ownership should express responsibility, not merely where data happens to be stored. It should be clear who maintains a thing's guarantees, who may change it, and who is responsible for ending it.

Put related state, behavior and obligations together when they share a reason to exist. Give genuinely independent responsibilities independent ownership.

### Make lifetime dependencies structural

When something exists only on behalf of an owner, its usable lifetime should follow that owner. Beginning, transferring and ending ownership should naturally establish, transfer and release the corresponding obligations.

This is the RAII mentality beyond resource cleanup: ordinary program structure carries the lifecycle. Cleanup should follow ownership, rather than depend on every exit path remembering what to undo. Things with different lifetimes need an explicit relationship, rather than being kept alive together for convenience.

### Establish invariants by construction

Prefer representations and interfaces that naturally preserve valid relationships. Make invalid situations difficult to create instead of relying on repeated checks to detect them or repair them afterwards.

Checks remain necessary at boundaries where facts are uncertain or outside the program's control. Distinguish validating genuine uncertainty from continually checking relationships that the program itself could guarantee.

### Keep authority unambiguous

A fact should have a clear authoritative home. Derive what follows from it rather than maintaining several independent accounts that must stay synchronized.

Multiple representations can be useful, provided their meanings and relationship are explicit. The problem is competing authority or redundant bookkeeping, not copying data in itself.

### Enable local reasoning

A component should be understandable through its own responsibilities and contracts. Using or changing it should not require knowing all of its consumers, their internal state, or their cleanup obligations.

Good encapsulation lets someone safely ignore implementation details. Dependencies and effects should be apparent at the boundary rather than hidden in ambient state or distant conventions.

### Compose coherent behavior

Build larger behavior from parts with clear meanings and useful contracts. Boundaries should follow responsibilities and purpose, not merely technical stages or superficial similarities.

When an action naturally consists of a sequence, express that sequence coherently. Other parts of the program should not have to reconstruct where it was, what it meant, and how to resume it. Introduce independent coordination when there is genuine independence to represent.

### Prefer the least machinery that expresses the design

An abstraction earns its place by reducing what a reader must understand or coordinate. More types, layers, indirection or genericity do not automatically produce better structure.

Prefer names that reveal intent and focused operations that explain the whole. Measure simplicity by the number of independent concepts and relationships, not just by line count or the amount of code moved behind an interface.

### Applying the principles

Use judgment, not maximal enforcement through types or a single architectural template. Ask:

- Does the structure tell us who is responsible, and for how long?
- Which guarantees follow from the representation, and which depend on discipline elsewhere?
- How much unrelated machinery must someone understand to change this part safely?
- Does an abstraction remove complexity, or merely relocate it?

## Current execution model

Antitone is a Rust application with native RAW bridges and wgpu. The boundaries below describe its current mechanisms, not a required feature layout or universal task/service abstraction. [README](README.md#module-map) maps the implementation.

The [host](../src/platform/host.rs) owns the UI thread. [Application tasks](../src/application.rs) advance outside painting; updates request repaint without requiring a paint to make progress. Features capture operation inputs and edit targets at the relevant action and use short `UiContext` transactions around awaits. Validate candidates before promotion; accepted edits and their history form one transaction. Never hold an Editor or UI borrow across an await, or perform RAW development, GPU waits or file IO in a paint callback. Shutdown destroys pending futures before retiring host/GPU state.

### Scopes: nested, disposable UI state

The [document](../src/editor/document_scope.rs), [photo visit](../src/editor/photo_scope.rs) and [grade](../src/editor/grade_scope.rs) own accepted state/retention, visit-bound interaction and effective-image demand respectively. The [tool owner](../src/editor/tool.rs) contains the unfinished interaction; replacement retires it without discarding accepted edits. Document-owned interactions can span photo visits. [Editor](editor.md#accepted-state-and-drafts) describes their actual admission and navigation behavior.

### Services and workers

Synchronous work runs through [the blocking pool](../src/background.rs); each job retains its captured inputs and returns panics as errors. [Native owners](native-development.md#editor-scheduling) keep mutable RAW handles worker-local. [Photo preparation](../src/photo_preparation.rs) resolves recorded starting exposure separately from shared pixels before publishing a processed positive. Resource ownership can justify a service; stateless work can remain an ordinary function.

### Source acquisition and retention

[Source acquisition](../src/source/mod.rs) supplies compatible immutable pixel leases independently of optional retention. The document owns RAM retention and recorded interpretation; jobs own their inputs. Eviction or zero retention preserves active leases. Joined demands share execution at the highest active priority while retaining independent cancellation. File identity is captured at Import/Open; RAWs are treated as immutable while open. [Editor retention](editor.md#responsiveness-and-optional-caches) and [native scheduling](native-development.md#editor-scheduling) explain current policy.

### Cooperative cancellation

A [background wait](../src/background.rs) owns a cancellation signal for its job. Retirement withdraws continuation rights; physical work retains inputs through safe cleanup and checks stop at appropriate boundaries. Explicit Cancel can retain the continuation for acknowledgement. A departing acquisition consumer does not cancel independent joined consumers. Represent cancellation distinctly from failure, and preserve already-committed side effects as successes. [Editor](editor.md#working-feedback-follows-what-remains-usable) owns the interaction choices; [output](colour-and-output.md#tiff-and-exr-serve-different-purposes) owns file publication.

### Native and GPU ownership

Frozen native source leases remain valid through their last consumer; interpretation is captured per consumer rather than mutated globally. One device/queue serves GPU consumers. The shared [submission gate](../src/gpu/submission.rs) excludes submission during surface reconfiguration; encoding, uploads and completion waits stay outside it. Workers submit and await completion off the UI thread; resident rendering records into its caller's encoder. Jobs and recorded commands retain resources through their actual use, independently of optional cache ownership. [Output](colour-and-output.md) and [native development](native-development.md) own the detailed contracts.

Production photographic measurements, Analyze and repair inputs share the [ordered photographic definition](processing.md#input-and-grading-have-separate-contracts) with rendering. Adapters supply captured inputs and request endpoints rather than rebuilding stage order; consumers own publication of query results.

### Pixel stages and spatial support

[Recipes](processing.md#one-accepted-image-recipe) compose captured photographic intent; [evaluation](../src/gpu/evaluation.rs) plans the supplied operations and resources. Each stage declares its upstream neighbourhood on an explicit raster grid. Propagate support backwards and use the declared grid pitch rather than recomputing it from rounded geometry. Only actual source bounds extend constantly for filtering: viewport, composition and tile edges retain their declared neighbours. Extract the requested output after processing. For physical views, the final post-grade component composes spatial effects, area reconstruction and footprint-integrated effects in that order. Earlier photographic prefix queries retain their declared grids. Preserve the distinction between original-sample execution and view approximations described in [processing](processing.md#physical-view-processing-is-an-approximation) and [geometry](geometry.md#process-before-averaging).
