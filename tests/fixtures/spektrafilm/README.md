# Spektrafilm source conformance

`conformance.json` contains controlled observations from executed upstream spektrafilm at revision `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. The [exporter](../../../scripts/spektrafilm/export.py) generates them alongside the [runtime data](../../../assets/spektrafilm/README.md).

For every selectable camera-negative/print-medium pair, eight supplied AP1 inputs exercise neutral, coloured, signed, HDR and zero-light cases. Observations include film exposure, developed density, paper exposure/density and the pre-inverse authoring-reference RGB. Paper-only cases exercise the declared positive-to-paper adapter separately.

Native tests compare source stages and composed viewed output. Paper-only observations combine an independent scalar Python implementation of the calibrated reference with upstream paper development/readout; they are model conformance, not physical-film measurements. Fixture-only photometry retains the paper dye/base, viewing illuminant and CIE-Y basis before the scanner's numeric log-zero epsilon. Analytic controls separately test exposure/dye matrix order, signed effective coordinates, receiver integration and fixed gray balance. An interval check covers passive transmission throughout the authored input cube. Reference RGB is obtained before inverse encoding, so the GPU forward reference also tests the sampled scanner/inverse component.

`supra-original.json` captures a 5³ Supra paper-only numeric asset from main commit `f0749f9d80d9c28b98fb0430f4c253c630bbeb11`, before the input/readout calibration update. It tests that saved samples remain authoritative even when their catalogue labels still exist; it is not an appearance target for new generation.

No photographs or scanner-device measurements are included. Spektrafilm data derivatives retain the attribution and CC BY-SA terms in [the source-data notice](../../../assets/spektrafilm/LICENSE-data.txt). The [original Academy notice](../../../assets/spektrafilm/NOTICE-Academy-APD.txt) remains retained for the frozen calibration target's ancestry and earlier distributed reference-derived components.
