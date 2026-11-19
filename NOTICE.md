# Antitone licensing and acknowledgments

Copyright © Emil Sahlén.

Antitone's original code is licensed under the MIT license; see `LICENSE`. You may use, modify and redistribute it under that license. It comes without warranty. Independently licensed third-party source and assets retain their own terms and notices; the top-level license does not relicense them.

## Included components

- Adobe DNG SDK 1.7.1 build 2768 and its bundled XMP/codec dependencies: see `vendor/dng-sdk/PROVENANCE.md`, the original Adobe license and component notices.
- This software is based in part on the work of the Independent JPEG Group.
- XMP includes the RSA Data Security, Inc. MD5 Message-Digest Algorithm and UUID support with their retained notices.
- LibRaw 0.22.3: dual LGPL/CDDL licensing. Original copyright and license files accompany packages.
- GPU DHT shader: derived from Anton Petrusevich's DHT, copyright 2013, with LGPL 2.1 selected. Source, modifications and license are retained in `src/gpu/demosaic/dht.wgsl` and `native/dht/`; packages include these materials.
- RawSpeed, revision [`804ae3bcc70c`](https://github.com/darktable-org/rawspeed/tree/804ae3bcc70c9362518dab8a039310d26516af1d): LGPL; original license and credits accompany packages. The embedded camera database is retained unmodified under CC BY-SA 3.0, with attribution to RawSpeed contributors and its original license declaration.
- pugixml 1.15: MIT, copyright Arseny Kapoulkine; the original license accompanies packages.
- LLVM OpenMP 22.1.8: the original LLVM OpenMP license accompanies the bundled runtime.
- IBM Plex fonts: SIL Open Font License 1.1, retained in `assets/fonts/OFL.txt`.
- OpenColorIO-derived rendering algorithms: retained notice in `assets/ocio/LICENSE-OpenColorIO.txt`. The production executable does not dynamically link the OCIO reference library.
- spektrafilm by Andrea Volpato, https://github.com/andreavolpato/spektrafilm, revision `3bb2c2d2801ff68b92019cf1dbcbb133d60832bc`: native algorithm ports are GPLv3; evaluated upstream colour-profile/LUT derivatives are CC BY-SA 4.0. Retained licenses, attribution and modifications are in `assets/spektrafilm/`. The top-level MIT license does not relicense these components.
- Historical Academy APD/ADX reference, S-2008-002 v1.1 (2010): source and original notices remain retained for the frozen calibration target's ancestry and earlier distributed reference-derived components in `assets/spektrafilm/NOTICE-Academy-APD.txt`, copied into packages under `Licenses/spektrafilm/`.
- Independently sourced B&W materials: Antitone reconstructions of ILFORD, FOMA and Kodak publications, including Ilford Multigrade RC Deluxe, FP4 Plus and HP5 Plus. Sources, processes and interpretation assumptions are in `assets/papers/` and `assets/negatives/`; generated provenance is in `assets/monochrome/`. These are not manufacturer-authored or Spektrafilm colour-stock profiles. Negative reconstruction reuses the separately licensed Hanatos spectrum family. Public-distribution clearance for the source-derived materials remains unresolved.
- Rust dependencies: the selected production dependency graph's declared licenses and supplied license/notice files are inventoried in packaged `Contents/Resources/Licenses/`.

## Local packages versus public releases

The maintained package command currently produces local test artifacts, not a declaration of release clearance. The generated license inventory is evidence for review, not a legal compatibility decision. Native bundled components, Cargo dependencies and asset provenance all matter; Spektrafilm-derived code and assets retain the separate terms documented in `assets/spektrafilm/README.md`.

RawSpeed is statically linked and the LGPL DHT shader is embedded. Before public binary distribution, supply the corresponding source/relinking materials their LGPL terms require; bundled license text alone is insufficient.

Before publishing binaries publicly, review the dependency and asset license inventory again: match each bundled component's own terms, keep its notices, and re-check the Adobe SDK commercial-distribution clause. Release also involves Developer ID signing/notarization if chosen, clean-machine execution and broader platform acceptance. Do not infer release clearance from a green build or a valid ad-hoc signature.