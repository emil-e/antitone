# Documentation screenshots

Generated UI screenshots referenced by the [user guide](../../guide/README.md) and the
README. They are produced from the committed synthetic RAW fixtures in
`tests/fixtures/raw/`, which contain no photograph, camera sample or private metadata.

## Regenerate

From the repository root, after [build and run](../../docs/build-and-run.md#build-and-run):

```sh
ANTITONE_UI_CAPTURE=assets/docs cargo test --locked --features ui-tests \
  --lib editor::kittest_tests::guide_shots -- --test-threads=1
```

The test drives the states the documentation shows — the roll workspace, a selected
filmstrip photo on Photo, Analyze's sampling-area review, the viewer help popover and
the export dialog — and asserts they stay reachable. The PNGs are written at 2× density
only when `ANTITONE_UI_CAPTURE` names a directory; the suite is otherwise capture-free.

`ANTITONE_DOC_SCANS` (a directory of ARW files) substitutes real scans for the fixtures.
Do that only for screenshots intended for publication: the output then contains those
photographs, and regenerating the committed set needs the same scans again.
