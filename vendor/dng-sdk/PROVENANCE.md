# Adobe DNG SDK source

This directory contains unmodified source selected from Adobe DNG SDK **1.7.1 build 2768**, whose archive readme dates the build October 13, 2026. It includes Adobe's bundled IJG JPEG, JPEG XL (including Brotli, Highway and skcms), and XMP toolkit (including Expat, Boost, zlib and UUID/MD5 support).

- Upstream: <https://helpx.adobe.com/camera-raw/digital-negative.html> and <https://www.adobe.com/go/dng_sdk>.
- Supplied archive: `SDK-1.7.1-2768.zip` (upstream directory `dng_sdk_1_7_1`).
- Archive SHA-256: `c1d3a545ed45b150e59af2e6aa0695c1551f0e2bed34ad87436898a05194d2b1`.
- `SHA256SUMS` identifies every selected upstream file and its original bytes. Paths are relative to this directory. No upstream files have been patched.

The archive's main SDK `source/`, JPEG source and JPEG XL source/build descriptions are retained, together with the XMP core/files/common sources, public headers and supporting third-party sources. Source headers under XMP's `build/` directory contain upstream version metadata, not compiled products. Samples, compiled libraries, executable project files and standalone PDF documentation are excluded. This is a source-build baseline rather than a claim that every retained header or helper is used by Antitone.

Build integration and source membership live outside this directory in `native/dng/`. The SDK/JPEG/XMP source lists follow Adobe's validator and XMP projects, excluding validator `main`; JPEG XL uses its included CMake build. The SDK and bridge share `native/dng/defines.txt`. The SDK's validation instrumentation is retained to avoid mixing a native build change with a processing-basis change. This does not build or repackage the validator executable.

## Licenses and distribution

`LICENSE.txt` is Adobe's SDK license, not Antitone's MIT license. Using the SDK accepts its terms. These include notice preservation, restricted trademark use and an indemnity obligation for distribution in a commercial product. This provenance record is not legal clearance or a waiver of those conditions.

Bundled components retain their original file notices and licenses. In particular, IJG JPEG's `README` includes its license and required acknowledgment; JPEG XL and its dependencies retain their license files. Expat, zlib, Boost and UUID/MD5 sources also contain their own notices. The archive omits the Boost Software License text referenced by its headers; the supplemental [`licenses/BSL-1.0.txt`](../../licenses/BSL-1.0.txt) supplies it, with provenance in `licenses/README.md`, without changing archive-original files. Antitone's package notice inventory must account for native code as well as Cargo dependencies. Do not replace these notices with Antitone's license.

For an update, obtain and review the exact new archive, verify its digest and inspect changes to source/dependency licenses. Replace the selected source from that archive (never from a local build tree), regenerate the per-file manifest, update native source lists/flags if needed and run adapter/shared-source tests. A new SDK version may change saved DNG interpretation compatibility; do not update only the build pin without assessing that boundary.
