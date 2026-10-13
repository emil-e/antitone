# Match rendered outputs and use managed SDR presentation

A P3 file should contain a P3 rendering, not an sRGB rendering hidden inside a wider container. Preview and TIFF therefore use the same selected ACES target and matching encoding/profile.

CPU export/reference and GPU preview share pinned native definitions and generated algorithms. We keep ACES mathematics unchanged; agreeing implementations still need independent gamut, profile and presentation checks, since both could agree on the same mistake.

The preview converts to an explicitly tagged extended-sRGB floating-point SDR layer. This preserves P3 coordinates outside ordinary sRGB until the OS performs monitor conversion. Encoding happens once; neither a second display transform nor an application monitor transform is added.

HDR display output or another target would need its own validated boundary. Software agreement and layer tagging do not establish physical-monitor calibration or broad device support.

[Colour and output](../colour-and-output.md) explains the paths. [OCIO artifacts](../../assets/ocio/README.md), [profiles](../../assets/profiles/README.md) and [diagnostics](../diagnostics.md#rendering-gamut-and-icc) own provenance and checks.
