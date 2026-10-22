# Documentation screenshots

Generated UI screenshots referenced by the [user guide](../../guide/README.md) and the
root README. `guide_shots` drives the documented states — the roll workspace, a selected
filmstrip photo on Photo, Analyze's sampling-area review, the viewer help popover and the
export dialog — and asserts they stay reachable. PNGs are written at 2× density only when
`ANTITONE_UI_CAPTURE` names a directory; the suite is otherwise capture-free.

The PNGs are ignored by git, because a capture either contains traces of the fixtures or
the photographer's own negatives.

## Regenerate

From the repository root, after [build and run](../../docs/build-and-run.md#build-and-run):

```sh
ANTITONE_UI_CAPTURE=assets/docs cargo test --locked --features ui-tests \
  --lib editor::kittest_tests::guide_shots -- --test-threads=1
```

That run uses the committed synthetic RAW fixtures in `tests/fixtures/raw/`, which contain
no photograph, camera sample or private metadata. They exercise every state but look like
test patterns, so they are a reachability check rather than imagery for publication.

`ANTITONE_DOC_SCANS` substitutes a directory of ARW files for the fixtures:

```sh
ANTITONE_DOC_SCANS=/absolute/path/to/roll ANTITONE_UI_CAPTURE=assets/docs \
  cargo test --locked --features ui-tests \
  --lib editor::kittest_tests::guide_shots -- --test-threads=1
```

Point it at **one complete roll**, in chronological order. The capture analyzes the scans
and shows the resulting grade, so a partial set — or a set that includes the exposed leader
and frame-edge frames from the start of the strip — skews the roll's Dmin/Dmax and white
balance and the screenshots come out badly cast. The scans are read from the emulsion
side, so the test applies the shared baseline's vertical flip before analyzing.

A real-scan run develops every frame in the background: five scans take about a minute,
thirty-six closer to seven.
