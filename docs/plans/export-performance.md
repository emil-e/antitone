# Faster authoritative export

Production export now uses original-sample GPU photographic processing and pinned ACES, with exact f64 coverage reconstruction and CPU file encoding. The [output guide](../colour-and-output.md) owns that contract. This plan tracks performance work, not permission to reintroduce a competing production CPU renderer or reduce photographic quality.

## Problem and acceptance

The verified baseline is useful but the transformed path is slow. A 49.77-MP native composition took approximately 8 seconds for TIFF and 12 seconds for full EXR; a corrected, straightened, quarter-turned 42.66-MP crop took approximately 37/41 seconds. These are indicative observations on one camera/device, not isolated benchmarks or cross-device targets. The retained working detail/underlay and encoded frame can coexist; passing the bounded resource exercise does not establish general pressure safety.

Reduce end-to-end export latency, especially transformed compositions, while preserving:

- Authoritative originals, continuous correction, native-camera inversion, ordered source-centred locals and whole-stack validation.
- Per-original ACES before linear-target reconstruction for TIFF; compensated signed/HDR AP1 into f64 reconstruction/AP0 for EXR.
- Existing numerical, geometry, supplied-sample encoding, profile/metadata and destination-safety gates. No preview input, quantization shortcut, extra clamp, new unsupported domain or silent fallback.
- Explicit bounds on job-owned work, retained intermediates and encoded/in-flight versions alongside the working view. Do not improve timings by increasing normal cache limits, relaxing diagnostic safety allowances or hiding live ownership.

## Next steps

1. Establish a matched release scalar/OCIO-reference versus GPU baseline using the same leased source, correction, accepted snapshot, composition, target/precision and shared encoders. Keep RAW development, infrastructure initialization and file writing identifiable. The retained reference adapter is a measurement oracle, not a production backend or a promise about an old binary's historical performance.
2. Attribute original sampling/upload, photographic GPU work/readback, coverage planning/reduction and encoding. Count actual original work and repeated tiles, not just submissions or declared bytes. Native versus corrected and exact turns versus arbitrary straightening must be separable. Use an agreed GPU window; with unconfirmed competing work, report indicative timings rather than isolated cost.
3. Address the largest measured cost with a small focused change. Current transformed batching processes roughly four original samples per output sample, including repeated tile work, and waits synchronously for each readback. Bounded job-local reuse or scheduling may help; additional retained intermediates require measured benefit and explicit byte/lifetime accounting. Do not assume either that caching is forbidden or that more buffering is automatically better.
4. Verify the chosen change against independent scalar/OCIO, exact coverage and exact supplied-sample encoder checks, then reopen full-size corrected/uncorrected TIFF/AP0 EXR outputs. Include ordered locals, signed/HDR/floor/bypass cases, orientation/crop/straightening, immutable snapshots and explicit failures. Keep photographic numerical equivalence separate from incidental CPU bit identity.
5. Repeat bounded process/resource observations with retained production detail/underlay, using the existing pressure, swap, growth and duration stops. Process footprint/resident memory and Metal allocation overlap and must not be added. Preserve the single normal-view lifecycle and production/reference feature split; document remaining limits rather than claiming universal hardware or photographic acceptance.

The [development checks](../development.md#export-checks) and [full-size diagnostic](../diagnostics.md#export) provide the normal verification route. GPU-service completion is not physical interaction or scanout latency. Actual pressure exercises, native application automation and system setup still require separate authorization.
