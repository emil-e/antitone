# Broader LibRaw source compatibility

Status: future work. Investigate support for more sources already decoded by pinned LibRaw, rather than treating the current Bayer bridge as a permanent product boundary. [The roadmap](../../PLAN.md) owns priority; no decoder expansion is selected.

## Goal and current limits

Keep established LibRaw development while accepting additional sensor layouts and black-level representations that can supply a consistent three-channel linear camera-RGB source. File-picker extensions are hints, not a compatibility guarantee; [processing](../processing.md#start-with-the-cameras-measurements) owns the current supported boundary.

The bridge currently assumes integer Bayer sensor storage, Bayer filter-mask diagnostics and scalar/per-channel black levels. It explicitly excludes X-Trans and non-CFA images and rejects any declared repeated black pattern, including a 1×1 declaration. These are implementation constraints, not demonstrated requirements of downstream RGB inversion or AP1 grading. Some sources may also require different LibRaw development settings; supporting them is not simply removing guards.

A separate entry-point gap remains: `Editor::new` recognizes only ARW as an initial RAW path; other suffixes route as projects. Picker extension support does not establish CLI startup support. Aligning that routing is distinct from expanding the decoder's accepted representations.

## Scope and non-goals

Investigate X-Trans, suitable non-CFA/linear camera sources, decoded floating-point inputs and spatial black-level metadata. Assess Foveon and four-colour cameras separately: decoding them does not establish that their channel basis fits native-camera inversion.

Preserve original-capture diagnostics, compatible calibration and signed/HDR working values. The proposed expansion starts from pinned LibRaw, fixed roll WB, the current camera model/matrix compatibility checks, AP1 grading, AP0 EXR and RGB16 retained sources. These implementation choices explain the present checks, rather than requiring every future source representation to use the same mechanisms. A changed development basis needs corresponding calibration, diagnostic and precision validation; silently adding a pedestal or clamp would not demonstrate source compatibility. Custom RAW development remains outside this proposal's scope. Universal LibRaw compatibility and photographic acceptance are not established.

## Open questions

- Which decoded representations already produce valid three-channel camera RGB under our settings, and which need an established LibRaw algorithm/configuration change?
- How should original sensor diagnostics represent X-Trans or non-CFA samples without pretending they use the Bayer four-slot layout? Required clipping/reference checks must not silently disappear.
- Which black-pattern declarations are equivalent to existing scalar/per-channel levels, and which need per-site sampling? Distinguish harmless 1×1 metadata from actual spatial variation.
- When LibRaw converts floating-point input to integer data, what precision/range and diagnostic meaning are retained? Do not confuse file encoding with unpacked storage.
- Can a nonstandard/four-colour source retain a trustworthy three-channel camera basis? Reject it unless that boundary is established.
- Does any accepted path change development compatibility? Decide that before reusing retained project/cache identities.

## Acceptance

For each supported class, use authored fixtures where feasible and explicitly supplied read-only camera samples where necessary. Prove decode, sensor diagnostics, fixed-WB development, calibration, correction, proxy mapping, Save/Open and TIFF/AP0 EXR output. Preserve complete RGB16 agreement across thread policies and rejection of invalid metadata/unsupported variants. Test both photo and reference import, including native selection separately. Software checks do not establish calibrated scene colour, broader-device behavior or physical photographic acceptance.

## Next action

Inventory the pinned LibRaw output/settings for these classes and map the assumptions in `native/raw_bridge.cpp` and `src/raw.rs`. A deterministic scalar-equivalent 1×1 black-level case is a small initial candidate. Each additional class raises its own diagnostic, fixture and acceptance questions.
