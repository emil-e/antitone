# Spektrafilm source conformance

`conformance.json` contains controlled observations from executed upstream spektrafilm at revision `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`. The [exporter](../../../scripts/spektrafilm/export.py) generates them alongside the [runtime data](../../../assets/spektrafilm/README.md).

For every selectable camera-negative/print-medium pair, eight supplied AP1 inputs exercise neutral, coloured, signed, HDR and zero-light cases. Observations include film exposure, developed density, paper exposure/density and the pre-inverse authoring-reference RGB. Paper-only cases exercise the declared positive-to-paper adapter separately.

Native tests compare source stage observations and composed viewed output. Shared preprocessing and calibration functions make these tests upstream-model conformance, not independent physical-film measurements. The reference RGB is obtained before inverse encoding, allowing the GPU forward reference to test the native sampled scanner/inverse component.

No photographs or scanner-device measurements are included. Data derivatives retain the attribution and CC BY-SA terms in [the source-data notice](../../../assets/spektrafilm/LICENSE-data.txt).
