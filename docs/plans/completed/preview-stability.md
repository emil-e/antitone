# Settled-preview buffer limits

Status: complete. Adapter-supported buffer limits implemented and focused checks passed. User approved raising limits and deferring strip processing until observed problems warrant it. No minimum chip/RAM target is established; M1 Pro/16 GB was a provisional example, not a required gate.

## Goal and scope

Fix the reported `Settled preview failed: GPU processing: invalid area-average dimensions or GPU budget` without changing reconstruction, response/ACES/ICC/export mathematics, physical-pixel output/native 100%, cancellation or other device limits. No strip implementation, reduced-resolution fallback, persistence or gesture/UI redesign.

## Confirmed diagnosis and decision

A valid large-display request can exceed the **requested default** wgpu buffer limits despite the adapter supporting it. `Geometry::output_size` generates positive dimensions no larger than the visible source region; `AreaAverage::new` requires one output-sized RGBA32F buffer. The host previously requested default limits: 128 MiB maximum storage binding and 256 MiB maximum buffer size.

Reproduced on M5 Pro/Metal without RAW IO: an 8640×5760 source fitted into a 2560×1707 logical canvas at 2 pixels/point requests 5120×3414 output. Full-frame sums require **279,674,880 bytes**, exceeding both defaults. Construction rejects this with the exact reported error. This establishes a reachable cause, not the user's exact unknown window/source state or every possible preview failure.

The actual adapter reports **4,294,967,292-byte** maximum storage binding and **4,294,967,295-byte** maximum buffer size. Raising both requested fields to their reported capacities fixes this mismatch without a new reconstruction algorithm or fixed speculative memory ceiling. Other limits remain at defaults; resource/dimension rejection remains intact. Requesting capacities does not allocate them upfront.

The earlier strip proposal is not selected. Implement it only if actual application/user memory or performance problems justify it. Larger previews still allocate more memory: scratch and linear output each use 16 bytes per output pixel, excluding retained images/RAW/runtime. This is not a claim of an aggregate memory budget or universal device acceptance.

## Implementation and verification

- [x] Reproduce the default-limit rejection and query the actual Metal adapter.
- [x] `platform::device_limits` selects only the two adapter buffer capacities; the shared host uses it when requesting the device.
- [x] Unit regression verifies only those fields change, including default-size and larger supported capacities.
- [x] Metal regression uses real viewport Fit geometry, confirms the old rejection, then accumulates/finishes into the full-size 5120×3414 output under negotiated limits. Samples at both corners and a zero interior check normalization, storage beyond the old limit and full target coverage at the unchanged 2e-6 tolerance. Invalid zero/upscale requests still reject.
- [x] Existing fractional tiled averaging, viewer/CPU targets, yellow continuity and extended-sRGB tests pass (4 viewer tests total, 0.46 s M5 snapshot).
- [x] Four viewport tests, 21 editor/control tests and the buffer-limit unit test pass. Strict Clippy for the library/viewer regression/editor binary and root/new-probe formatting pass. Release editor/presentation binaries and named research probe build offline.
- [x] Presentation probe passes transfer/UI checks and two requested resizes, with 11 submitted frames. Real-source editor smoke passes Fit/native refinement and photo/reference switching in 10.64 s / 429 submitted frames; Fit 1538×1026 takes 2.381 s, native 1538×1320 takes 0.145 s (M5 snapshots).
- [x] Record final checks/limits and hand off to basic Save/Open, without automatic commit/push or next-feature implementation.

Diagnostic: `docs/research/tools/film/src/bin/preview-budget-check.rs` compares the default rejection with the same production limits policy. Build/run named bins using the [tools guide](../../research/tools/README.md#current-gate-reproduction). Its baseline initially exited 1; the updated probe passes and prints both default rejection and successful negotiated construction. The Metal regression additionally proves actual accumulation and finish/readback, not merely constructor acceptance.

Production scope is `src/platform/{host,mod}.rs`; the averaging kernel, refinement scheduler and viewer geometry are unchanged. Active guidance now treats hardware examples as provisional rather than mandatory minimum specifications; historical results remain historical.

## Handoff

Next action: basic editor Save/Open in [the durable-projects plan](../durable-projects.md). No implementation of that feature or commit/push in this slice. Record actual resource/performance problems before considering strip machinery.

Large output was verified offscreen; real-source editor smoke used its normal window, not a physical large-display scenario. No fresh full suite, aggregate peak-memory benchmark or universal device claim. Broader device/macOS-14/physical-monitor checks remain separate work, not a fixed M1 release gate. An initial viewport command inherited an unavailable GCC path; rerunning with the documented Apple-Clang environment passed, without global environment/toolchain changes.
