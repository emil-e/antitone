# Export performance follow-up

Production export uses authoritative original-sample GPU processing, exact f64 reconstruction and CPU encoders. Straightened compositions now reuse completed tiles within one immutable job; the [output guide](../colour-and-output.md#tiff-and-exr-serve-different-purposes) owns that contract. Matched scalar/OCIO-reference measurements and bounded cache comparisons are available through the [full-size diagnostic](../diagnostics.md#export).

The first bounded reuse improves the measured transformed workload, but does not establish isolated hardware throughput, a minimum chip/RAM target or universal latency. The reference adapter remains an independent measurement oracle, not a second production renderer or a reconstruction of an old binary.

## Remaining scope and acceptance

- Broaden performance characterization across source sizes, correction, crop alignment, exact turns and straightening, both targets and output precisions. Include ordered locals and signed/HDR/floor/bypass cases in correctness checks. Use a coordinated timing window before attributing small differences to hardware or implementation.
- Attribute the remaining coverage planning/reduction and encoding separately from tile access. Reuse reduces photographic work, but CPU reconstruction and file encoding still account for substantial latency. Choose further optimization only when this evidence identifies a useful cost; do not assume more cache capacity or a scheduling framework is warranted.
- Keep additional ownership explicit: completed CPU tile payload, current upload/readback, encoded frame, retained working detail/underlay and encoded/in-flight/native-retired resources. A bounded job cache is not a full-source float store or an aggregate admission guarantee. Measure benefit and lifetime before expanding it.
- Preserve existing original-processing, exact geometry, supplied-sample encoding, profile/metadata, immutable snapshot and destination-safety gates. Keep compensated AP1 through EXR reconstruction; no preview authority, early rounding, new clamp, unsupported-domain shrinkage or silent fallback.
- Repeat bounded resource observations with unchanged pressure, swap-counter, growth, duration and unavailable/stale-telemetry stops. Footprint, resident memory and Metal allocation overlap and must not be added. Do not improve a result by increasing normal cache limits or relaxing diagnostic allowances.

The [development checks](../development.md#export-checks) provide the normal verification route. Software agreement is not photographic or physical-display acceptance. Forced-pressure exercises, native application automation and system setup still require separate authorization. This plan records follow-up scope, not authorization to start it.
