# Lightroom round trip

The [Antitone plug-in](Antitone.lrplugin/) opens camera RAW originals in a separate Antitone process, saves an ordinary editable roll project and imports returned TIFFs. Lightroom development settings are not translated. The plug-in is available on macOS and Windows. Windows launching has not yet been verified in Lightroom.

## Install and choose Antitone

1. Extract the Antitone application package for your platform and the matching `Antitone-Lightroom.zip`. Keep the extracted `Antitone.lrplugin` folder in a stable location.
2. In Lightroom, open **File → Plug-in Manager → Add** and select `Antitone.lrplugin`. Disable an older Antitone entry before enabling this one.
3. In the plug-in's **Antitone** section, choose **Choose Antitone Application…** and select `Antitone.app` on macOS or `antitone.exe` on Windows. Keep the Windows executable beside the DLLs from its application package. A standalone `antitone` executable can also be selected for development.

The selection is retained in Lightroom preferences. Without a selection, macOS checks `/Applications/Antitone.app` before opening the chooser; Windows opens the chooser directly. A missing configured executable also returns to the chooser. The plug-in checks the application's transport version before opening an editing window; install matching app/plug-in packages if it reports an incompatibility.

For a source checkout, follow [native setup and build](../../docs/build-and-run.md#build-and-run), add `integrations/lightroom/Antitone.lrplugin`, and choose that checkout's `target/release/antitone` (`antitone.exe` on Windows). Existing ignored `Config.local.lua` executable overrides remain usable for development; they are excluded from distribution.

## Edit and return

Select originals and choose **Library → Plug-in Extras → Edit in Antitone**. Create a new `.antitone` roll or choose **Use Saved Roll**. Existing filenames are preserved; choose another name or open the saved roll. An existing roll must already contain every selected original.

The project can live anywhere. Each returned TIFF is written beside its original RAW, allowing Lightroom to stack it with that original. Selections spanning several folders activate all returned folders and select all returned images.

Edit normally, then choose **Save Back to Lightroom** in Antitone's header. It captures accepted edits, saves the whole roll and renders only the requested photographs. Apply drafts and finish pending calculations first if you want their results included. Return never accepts unfinished proposals automatically; newer accepted edits keep the document open. Lightroom imports after the Antitone process exits.

Choosing one linked original or returned TIFF later opens the whole saved roll at that photograph. Subsequent browsing does not change the photographs returned. Returns create new `*-Antitone.tif` versions without modifying originals or earlier TIFFs.

Ordinary Save writes only the project. Save As creates a copy without changing the canonical return project or TIFF folders. Opening a project outside Lightroom creates an ordinary editor window; saving it does not update existing Lightroom TIFFs.

## Missing projects and failed returns

If a recorded project is missing or unavailable, choose **Locate Saved Roll**, **Create Replacement Roll**, or Cancel. Locating preserves the saved roll's edits. A replacement is a fresh roll from the selected originals: it cannot recover the missing project's edits. Choosing a path does not clear catalogue metadata; completed returns apply the explicit new association. Older returned TIFFs follow their original's current association when reopened.

Save Back saves the project before rendering, so a later render or import failure does not lose the saved edits. Choose **Edit in Antitone** again to reopen the roll and render again. If the originals are not linked yet, choose **Use Saved Roll** and select the saved project. Earlier TIFFs remain on disk; another attempt creates new versions.

Failures offer **Show Log** to reveal the current diagnostic file in Finder or Explorer. Each invocation uses a separate temporary folder for `request.json`, completed `result.json`, process output in `session.log` and the compatibility check's `protocol.txt`. Copy a needed log before OS temporary-file cleanup. Projects and TIFFs live outside this folder.

Avoid simultaneous editing of one project: saves have ordinary Antitone file semantics, without multi-writer coordination. Requested RAWs must be online. Relinking RAWs, virtual-copy variants, multiple roll associations and replacing previous TIFFs are not implemented. Project compatibility follows the editor's [saved format](../../docs/projects.md#compatibility-is-about-the-processing-basis); development formats have no migrations.

## Transport and catalogue identity

Transport version 3 uses JSON: see the shared [request](../../tests/lightroom/request.json) and [receipt](../../tests/lightroom/result.json) fixtures. Requests carry new/open mode, the canonical project path and requested original UUID/path pairs. Locating a moved roll also carries its old project path so relative RAW locators can be rebased when needed. Receipts carry whole-roll source/path mappings and requested UUID/source/rendered-path mappings. UUIDs remain opaque catalogue context, outside the project; source IDs are strings to preserve their integer range in Lua.

Rust uses `serde_json`; the bundle vendors [rxi/json.lua v0.1.2](https://github.com/rxi/json.lua/tree/v0.1.2), compatible with Lua 5.1, with its MIT licence retained in [json.lua](Antitone.lrplugin/json.lua). Keep the toolkit identifier and metadata field IDs stable: existing catalogue associations use them independently of display names or bundle location.

## Check and package

From the checkout root:

```sh
python3 scripts/test-lightroom-plugin.py
cargo test --locked --features editor --bin antitone
cargo test --locked --features ui-tests --lib lightroom
cargo test --locked --features ui-tests --lib external_edit
cargo xtask package
cargo xtask verify-package
```

The [Lua runner](../../scripts/test-lightroom-plugin.py) uses installed Lightroom's macOS Lua runtime or a system Lua 5.1 shared library without launching Lightroom. Supply `--lua-library /path/to/library` explicitly, or `--plugin /path/to/Antitone.lrplugin` to test an extracted bundle. [SDK mocks](../../tests/lightroom/plugin.lua) cover configuration, JSON interchange, association, rerendering after failed or partial import, stacking, multi-folder selection and yielding metadata. GPU cases run locally; hosted CI runs the hardware-independent protocol, CLI, packaging and Lua cases.

The macOS [package workflow](../../docs/build-and-delivery.md#package-boundary) produces matching app/plug-in archives and checksums. Repeat a manual disposable-catalogue trial with the extracted pair for native chooser, Plugin Manager, moved-project handling and import behavior, including Windows launching. The [editor guide](../../docs/editor.md#external-edit-sessions) explains document ownership and shared output.
