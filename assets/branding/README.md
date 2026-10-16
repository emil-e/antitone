# Antitone branding

The identity borrows the compact inscriptions and indexing marks printed along film edges. Amber lettering identifies the editor; dark markings on an orange film-base tile identify the application outside it. The photographic workspace stays neutral.

The wordmark uses normal-width Arimo Bold with a leading triangle pointing into the name. Its proportions evoke film-edge lettering, without claiming to reproduce a historical Kodak typeface.

## Use the assets

- `wordmark-amber.svg`: transparent wordmark for dark backgrounds.
- `wordmark-density.svg`: the same outlines for light backgrounds.
- `wordmark-header.png`: the amber inscription rasterized at 4× for the app’s 16-point header. It is independent of the UI’s text fonts.
- `icon.svg` and `icon-small.svg`: large and optically adjusted small vector artwork. Use the small version at 32 logical points or below.
- `png/`: transparent square icons from 16 to 1024 pixels, with the small artwork used through 32 pixels.
- `icon.ico`: multi-size PNG-backed Windows icon; `icon.icns`: the macOS icon set, including Retina representations.

macOS can apply its own appearance treatment to bundle icons, so judge the Finder/Dock result as well as the flat SVG. The macOS icon adds a transparent canvas inset for Dock alignment. Its small representations are selected by logical size, so 32-point Retina uses the small artwork at 64 physical pixels. PNG/ICO exports do not have the macOS inset. See the [application prerequisites](../../README.md#prerequisites) for supported platforms.

SVG wordmarks contain paths, not text or external font references. Preserve their proportions and the distinction between the horizontal wordmark index and the icon’s vertical film edge.

## Regenerate

From the repository root on macOS with Apple’s command-line developer tools:

```sh
./scripts/generate-branding.sh
```

[`native/branding_assets.swift`](../../native/branding_assets.swift) owns the geometry and typography, emits both vector and raster assets, and uses `iconutil` for ICNS. Intermediate icon sets and Swift caches stay in `.cache/branding-assets/`. Normal app builds use the checked-in assets and do not need asset generation or font downloads.

The checked-in, unmodified [Arimo Bold source](https://github.com/googlefonts/Arimo/tree/4a6255f269916ae7ad3fc2706b0935e7621396b8) is used only for regeneration. Its [SIL Open Font License](fonts/OFL.txt) accompanies `fonts/Arimo-Bold.ttf`; no font is installed globally or embedded in the application. The window/index artwork is project-authored.

For app/bundle use, see [build and run](../../README.md#build-and-run). The local macOS bundle is not a signed, self-contained release.
