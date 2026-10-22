# Documentation screenshots

The screenshots referenced by the root README and the [user guide](../../guide/README.md)
are committed here. A capture comes from real negatives, so these images are part of the
product's first impression and part of the change that alters them.

The PNGs are committed at 1× (1440×900, about 1.6 MB each). GitHub displays them at their
CSS size, and the smaller set keeps a regeneration an ordinary commit. Capturing at 2×
would commit 22 MB per refresh, which is why the harness default is used.

## Regenerate

1. **Get the roll.** The committed set came from one complete roll of 36 scans,
   `DSC04379.ARW`–`DSC04414.ARW`, from the maintainer's archive. Any complete roll
   reproduces the workflow, but only the same roll keeps the new images comparable with
   the old ones.
2. **Run the capture** from the repository root, after [build and run](../../docs/build-and-run.md#build-and-run):

   ```sh
   ANTITONE_DOC_SCANS=/absolute/path/to/roll ANTITONE_UI_CAPTURE=assets/docs \
     cargo test --locked --features ui-tests \
     --lib editor::kittest_tests::guide_shots -- --test-threads=1
   ```

   Expect about seven minutes for 36 scans: it analyzes the roll and develops every frame
   before the shots of the settled grade.
3. **Inspect all five PNGs** against the list below before committing. The test asserts the
   states are reachable, not that the pictures are right.
4. **Commit `*.png` and `SOURCES.txt`** with the change that made them stale, and run
   `python3 scripts/check-docs.py` so the image links still resolve.

## What a correct capture looks like

In every shot:

- **Upright.** The scans are read from the emulsion side, so the capture applies the shared
  baseline's vertical flip before analyzing.
- **No pointer.** The capture drops the harness pointer before each render; a cursor arrow
  baked over a control means an older capture.
- **Native menus.** The capture installs the macOS menus, so the window matches the app
  people download rather than showing an in-window menu strip.

In the four shots taken after analysis (`getting-started-filmstrip`, `viewer-bar`,
`export-dialog`, `window-overview`):

- **Neutral.** A cast means the input was not one whole roll — a partial set, or the exposed
  leader and frame-edge frames from the start of a strip, skews Dmin and the shared white
  balance for every photograph.
- **Settled.** The viewer status shows no "Acquiring view" or "Cached preview"; the capture
  waits for the displayed view to match the current processing.
- **Developed.** Filmstrip cards show graded positives rather than empty previews.
- **Consistent.** The export dialog counts match the roll you loaded.

`analyze-review` is the exception: it records the review *before* the roll is graded, so it
correctly shows the uncropped source negative with its sampling areas, unmeasured
calibration and undeveloped filmstrip cards.

## Without real scans

Omitting `ANTITONE_DOC_SCANS` drives the same states on the committed synthetic fixtures in
`tests/fixtures/raw/`, which contain no photograph, camera sample or private metadata. They
prove the states stay reachable in about ten seconds, but render test patterns, so a
fixture capture is never committed here: write it somewhere under `.cache/` instead.

## Provenance

The capture writes `SOURCES.txt` beside the images: the app version and commit, the window
size and density, and how many scans produced them. It usually names the commit before the
change that adds the images, because a capture cannot record a commit that does not exist
yet. If a change alters what a documented screen looks like and `SOURCES.txt`
stays unchanged in the same commit, the screenshots were not regenerated.
