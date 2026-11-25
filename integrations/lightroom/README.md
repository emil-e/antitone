# Lightroom round trip

The [Antitone plug-in](Antitone.lrplugin/) opens camera RAW originals in a separate Antitone process, saves an editable project and imports returned TIFFs. Lightroom development settings are not translated. The plug-in is available on macOS and Windows. Windows launching has not yet been verified in Lightroom.

## Install and choose Antitone

1. Extract the Antitone application ZIP for your platform and the Lightroom plug-in ZIP from the same release. Keep the extracted `Antitone.lrplugin` folder in a stable location.
2. In Lightroom, open **File → Plug-in Manager → Add** and select `Antitone.lrplugin`. Disable an older Antitone entry before enabling this one.
3. In the plug-in's **Antitone** section, choose **Choose Antitone Application…** and select `Antitone.app` on macOS or `antitone.exe` on Windows. Keep the Windows executable beside the DLLs from its application package. A standalone `antitone` executable can also be selected for development.

The selection is retained in Lightroom preferences. Without a selection, macOS checks `/Applications/Antitone.app` before opening the chooser; Windows opens the chooser directly. A missing configured executable also returns to the chooser. The plug-in checks the application's transport version before opening an editing window; install matching app/plug-in packages if it reports an incompatibility.

For a source checkout, follow [native setup and build](../../docs/build-and-run.md#build-and-run), add `integrations/lightroom/Antitone.lrplugin`, and choose that checkout's `target/release/antitone` (`antitone.exe` on Windows). Existing ignored `Config.local.lua` executable overrides remain usable for development; they are excluded from distribution.

## Edit and return

Select originals and choose **Library → Plug-in Extras → Edit in Antitone**. If Lightroom knows about a saved project, it offers to reopen it. Otherwise Antitone opens a new project without asking where to save. To open a project Lightroom doesn't know about, choose **Open Saved Antitone Project…** instead. An existing project must already contain every selected original.

The project can live anywhere. Each returned TIFF is written beside its original RAW, allowing Lightroom to stack it with that original. Selections spanning several folders activate all returned folders and select all returned images.

Use **File → Save** to keep working later. For a new project, Antitone suggests the first RAW's folder and asks where to put the `.antitone` file. Lightroom remembers the association when Antitone exits, without importing TIFFs. **Save As** makes a copy without changing an established association or the return destination.

Choose **Save Back to Lightroom** in Antitone's header when you want TIFFs. It asks for a project path if you haven't saved yet, then saves the project and renders only the requested photographs. Apply drafts and finish pending calculations first if you want their results included. Newer accepted edits keep the document open. Lightroom imports after Antitone exits.

Choosing one linked original or returned TIFF later reopens the project and selects that photograph. Subsequent browsing does not change the photographs returned. Returns create new `*-Antitone.tif` versions without modifying originals or earlier TIFFs.

## Missing projects and failed returns

If a recorded project is missing or unavailable, choose **Locate Saved Project**, **Create Replacement Project**, or Cancel. Locating preserves the saved project's edits. A replacement starts from the selected originals and cannot recover those edits. Lightroom changes the association after the project is saved. Older returned TIFFs follow their original's current association when reopened.

Save Back saves the project before rendering. If rendering or import fails, reopen the project with **Edit in Antitone** and retry. If the catalogue couldn't record the association, use **Open Saved Antitone Project…** and select the saved project. Earlier TIFFs remain on disk.

Failures offer **Show Log** to reveal the current diagnostic file in Finder or Explorer. Each invocation uses a separate temporary folder for `request.json`, `saved.json` after a project write, `result.json` after rendering, process output in `session.log` and the compatibility check's `protocol.txt`. Copy a needed log before OS temporary-file cleanup. Projects and TIFFs live outside this folder.

Avoid simultaneous editing of one project: saves have ordinary Antitone file semantics, without multi-writer coordination. Requested RAWs must be online. Relinking RAWs, virtual-copy variants, multiple project associations and replacing previous TIFFs are not implemented. Project compatibility follows the editor's [saved format](../../docs/projects.md#source-interpretation-and-identity); development formats have no migrations.

## Transport and catalogue identity

Transport version 4 uses JSON: see the shared [request](../../tests/lightroom/request.json) and [return receipt](../../tests/lightroom/result.json) fixtures. Requests carry new/open mode and original UUID/path pairs; only open mode carries a project path. A request to open a moved project also carries its old path to rebase relative RAW locators. The save receipt identifies the project and maps all roll sources to paths. The return receipt adds the rendered paths and requested UUID/source pairs. UUIDs stay outside the project; source IDs are strings to preserve their integer range in Lua.

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
