# Antitone licensing and acknowledgments

Copyright © Emil Sahlén.

Antitone's original code is licensed under the MIT license; see `LICENSE`. You may use, modify and redistribute it under that license. It comes without warranty. Independently licensed third-party source and assets retain their own terms and notices; the top-level license does not relicense them.

## Included components

- Adobe DNG SDK 1.7.1 build 2768 and its bundled XMP/codec dependencies: see `vendor/dng-sdk/PROVENANCE.md`, the original Adobe license and component notices.
- This software is based in part on the work of the Independent JPEG Group.
- XMP includes the RSA Data Security, Inc. MD5 Message-Digest Algorithm and UUID support with their retained notices.
- LibRaw 0.22.0: dual LGPL/CDDL licensing. Original copyright and license files accompany packages.
- LLVM OpenMP 22.1.8: the original LLVM OpenMP license accompanies the bundled runtime.
- IBM Plex fonts: SIL Open Font License 1.1, retained in `assets/fonts/OFL.txt`.
- OpenColorIO-derived rendering algorithms: retained notice in `assets/ocio/LICENSE-OpenColorIO.txt`. The production executable does not dynamically link the OCIO reference library.
- Rust dependencies: the selected production dependency graph's declared licenses and supplied license/notice files are inventoried in packaged `Contents/Resources/Licenses/`.

## Local packages versus public releases

The maintained package command currently produces local test artifacts, not a declaration of release clearance. The generated license inventory is evidence for review, not a legal compatibility decision. Native bundled components, Cargo dependencies and asset provenance all matter; creative-look source/asset clearance remains separately unresolved in `assets/looks/README.md`.

Before publishing binaries publicly, review the dependency and asset license inventory again: match each bundled component's own terms, keep its notices, and re-check the Adobe SDK commercial-distribution clause. Release also involves Developer ID signing/notarization if chosen, clean-machine execution and broader platform acceptance. Do not infer release clearance from a green build or a valid ad-hoc signature.