# Settled-preview buffer limits — completed

Status: implemented. The host requests adapter-supported buffer capacities; strip processing remains deferred until observed resource/performance problems warrant it. No minimum chip/RAM target is established.

## Diagnosis and result

Valid large-display Fit requests could exceed requested wgpu defaults even when the adapter supported them. The previous defaults were 128 MiB maximum storage binding and 256 MiB maximum buffer size. An 8640×5760 source fitted to a 2560×1707 logical canvas at 2 pixels/point requests 5120×3414 output and **279,674,880 bytes** of RGBA32F sums—more than either default.

`platform::device_limits` now requests the adapter's reported storage-binding and buffer-size limits while leaving other limits at defaults. Requesting those capacities does not allocate them upfront. Reconstruction, physical-pixel output, numerical processing, cancellation and invalid-dimension rejection are unchanged.

Scratch and linear output each still use 16 bytes per output pixel, excluding other retained data. This is not an aggregate memory budget or a universal device guarantee.

## Verification and limits

Unit checks cover the two-field limits policy. A Metal regression reproduces the old rejection and accumulates/finishes the full-size output under negotiated limits, checking corners/interior normalization and unchanged tolerance; invalid zero/upscale requests still reject. Existing fractional averaging, target/presentation and real-source Fit/native smoke checks passed at the recorded checkpoint.

[Archived verification](../../research/history/implementation/preview-stability.txt) preserves the environment, adapter capacities, timings and original probe details. The research `preview-budget-check` bin is historical; the maintained regression is `large_retina_fit_area_average_uses_supported_buffer_limits` in `tests/viewer.rs`, reproduced through [feature-specific checks](../../development.md#feature-specific-checks).

Large output was verified offscreen, not on a physical large-display setup. Aggregate peak memory, broader devices, actual macOS-14 execution and physical-monitor acceptance remain separate [release work](../completion.md#export-and-release-work).
