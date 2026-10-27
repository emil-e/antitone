# Editor fonts

The editor embeds unmodified static IBM Plex Sans Text (450) and Medium (500) faces under the [SIL Open Font License 1.1](OFL.txt). Their reading and instrument roles are owned by [shared presentation](../../src/editor/presentation.rs); see [typography guidance](../../docs/editor.md#typography-roles).

Both faces and the licence come from [IBM/plex at c5f949677f6f163e8dfe98ca2c326bd48b42fa1b](https://github.com/IBM/plex/tree/c5f949677f6f163e8dfe98ca2c326bd48b42fa1b/packages/plex-sans). The files are `fonts/complete/ttf/IBMPlexSans-{Text,Medium}.ttf` and `LICENSE.txt` in that package.

SHA-256:

- `IBMPlexSans-Text.ttf`: `4abc4919d313b56a5e3ffdb249ed2450392a5df21478aa4e25ff23901398b1fb`
- `IBMPlexSans-Medium.ttf`: `331c8639d7598b2cde62a911a71db195e30cb655cd6bdf2e324a7e984955f907`
- `OFL.txt`: `7e6b2818edbd8f6a01ae80641cc8f16a51080d08fb4e532be3a0b6f74adb07da`

Retain the licence when distributing the embedded fonts; local macOS packaging copies it into the application resources. These fonts do not replace the outlined Arimo brand artwork or install any system fonts.
