# Supplemental notices

Some published crate archives omit license files located at their workspace root. `rust-notices.json` records version-specific upstream license sources at the commits identified by those crates' `.cargo_vcs_info.json` files. Two supplement modes exist: `mit` entries combine an observed copyright line with the complete, common MIT permission/disclaimer in `MIT-permission.txt` (line wrapping normalized); `text` entries pin an upstream notice file verbatim under `licenses/` — used where upstream ships only a pointer-style notice (`LICENSE.md` SPDX statement) rather than full license text, and where a crate's own VCS commit was rewritten away by upstream force-pushes so only the repository's current statement is pinnable. Such provenance notes are recorded in each entry's `note` and shipped in the package's `notice-provenance.json`.

This is a reviewed supplement, not a rule that every future version shares the same notice. Dependency updates must revisit these entries. The packager also collects license files recursively from the selected crate archives, including bundled Little CMS and font notices. Crates still lacking notice files are explicitly identified as unresolved in the generated inventory and console output; a declared SPDX expression alone is not presented as a completed notice review.

`BSL-1.0.txt` is the Boost Software License 1.0 from <https://www.boost.org/LICENSE_1_0.txt>. The selected Adobe archive includes Boost headers referring to this license but omits its full text. This supplemental file supplies it without modifying the archive-original source/manifest. It is included in local package notices as well.

These files do not resolve public-release licensing/source-distribution acceptance. See [NOTICE.md](../NOTICE.md).
