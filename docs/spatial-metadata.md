# Shared film and picture metadata

Spatial metadata describes the developed source rather than a tool's preferred crop or sampling inset. Analyze and Auto-crop request the same document-owned preparation through normal source/CPU services. Ready regions are reused within the open document independently of either consumer's accepted choices. Authored workflow checks and cached replay are separate from photographic, physical-input and resource assessment.

## One description, independent consumers

The document owns reusable source-keyed data: picture regions, available film limits, holder-opening estimates, coarse film/holder/light/unknown classification and finished automatic sampling windows. Some components can be unavailable. Native observations remain distinct from fitted picture bounds; each boundary is measured, inferred, camera-limited or unresolved. Coordinates are developed source pixels, before user orientation/crop; planes use inward unit normals. Source pose and axis skew retain both transformed raster directions even when companion boundaries are absent; unequal scale and active-area origin are not discarded.

A camera-limited side records unavailable physical extent, not a measured line at the sensor edge. Missing film or holder coverage describes availability separately from where the original picture frame would be. Unknown data cannot certify a full frame. Coarse material labels and pooled holder limits are estimates, not exact contours or calibrated confidence probabilities.

Auto-crop derives nominal picture framing within the available camera raster, independently of film, holder and material exclusions. Its inward margin is a photographic choice. Analyze instead qualifies original sample sites against film/material/holder support; picture statistics additionally obey the nominal picture boundaries. Its source-scaled inward band avoids boundary contamination without moving shared edges. Authored sampling windows remain editable but do not license samples outside those physical supports. Both consumers leave shared facts unchanged; Analyze retains its review and original-grid numerical measurements and eligibility checks.

Either feature triggers preparation when ready metadata does not match current inputs. Analyze-first leaves reusable data even if grading/review is subsequently cancelled. Auto-crop-first does not invoke Analyze or accept sampling. Browsing or changing grades/margins does not redetect compatible sources. Requests capture batch target IDs before awaiting preparation, so changed selection cannot retarget a queued operation. A queued Photo proposal belongs to its original manual Crop visit.

## Preparation and common dimensions

Services verify each source during its single acquisition for detection. Preparation overlaps at most two acquisition/capture jobs, retaining shared [camera rasters](roll-analysis.md#review-uses-finished-detection) rather than full-image leases or a separate geometry preview. Current acquisitions supply one runtime encoding scale. Geometry bilinearly resamples the whole uncorrected raster to at most 768 pixels on its longest side, retaining the same active extent, then encodes RGB16 for detection and fitting. Statistics and GI retain their original camera sites. Geometry includes dark holder/film sites rather than using GI's eligibility mask. The encoding scale is not project state or reusable region identity. Measurement excludes the optional roll flat-field: embedded Sony/DNG development correction remains part of the source interpretation, not an optional measurement dependency. Grouping follows declared matching source geometry/raster and camera basis; the physical assumption is that transferring source-pixel extents is useful. Camera identity alone does not establish unchanged magnification or picture format, and this is not a new user-managed reference workflow.

Automatic sampling windows use the same captures, with pooled tightening of the conservative holder interior before publication. They remain distinct from nominal picture framing: weak statistical picture support can be useful when a photographic crop is unresolved. Analyze copies these completed suggestions into its review and overlays authored windows without acquiring images or running another detector. Missing components are shown for manual review rather than requesting repeat detection.

W/H are common fitted parameters despite measurement variation. The current estimator uses median measured opposite-side extents independently per axis. It retains source identities supporting each axis; inferred bounds, accepted crops and sampling insets never feed back into size. There is no nominal format snapping or reciprocal/aspect folding. Preparation is stable under filmstrip ordering through source-keyed ordering.

Complete reference photographs are not a prerequisite. Differently clipped captures can supply complementary dimensions. If every top is unavailable, width and source-visible picture may remain useful without pretending that full height was measured. Full physical bounds, available framing and unknown components are kept separate.

With complete dimensions, native joint fitting poses three/four credible sides against fixed W/H; the adjacent-edge search handles two-side completion or failed joint fits. Every supplied side requalifies on the completed geometry. A film edge cannot become a measured picture edge, but unavailable film at an inferred side does not erase an otherwise supported nominal frame. Conflicting picture evidence and distinct competing-frame ambiguity still reject. Partial native observations need multiple credible sides and can contribute only their measured opposite-side dimensions. Without a complete-peer placement template, fitting/search uses common dimensions without inventing a pooled placement prior.

The original pooled selector can miss legitimate partial families at different translations. Preparation also checks a bounded set of existing native families when that selector supplies no qualified observation. This is not a new pixel/pose discovery grid or exhaustive ambiguity proof. Partial qualification uses finite camera-envelope companion spans internally, without publishing them as measured boundaries.

Camera-limit inference uses distributed photo-compatible edge support with local inward continuation. It is deliberately separate from transition measurement and does not require uniform darkness. Its current station/band settings are exploratory choices, not calibrated clipping uncertainty; ambiguous/missing support stays unresolved. Authored controls exercise all-clipped cases, a disconnected three-pixel edge strip, heterogeneous support, blanks and film cuts. These supplied-candidate controls do not prove native discovery or photographic accuracy.

Runtime estimates may use observations from the same sources they describe. Such results are training-conditioned, not held-out predictions. The retained strict evaluation path excludes targets upstream of pooled selection/appearance and dimension fitting; that validation guarantee is not a production requirement that every output be held out.

## Reuse and publication

Runtime metadata records source IDs, cheap file revisions, source/development interpretation, measurement correction and a preparation version. File revisions are freshness hints captured at Import/Open, not photographic identity or a RAW content hash. Changes to those inputs or replacement of boundary observations can invalidate dependent group results. Selecting, hiding, reordering, grading and changing consumer insets do not themselves change the observations.

A document-owned store retains immutable ready metadata. Camera rasters have document-scoped ownership separate from the region description and full-image LRU. Analyze can collect missing rasters without replacing finished regions; Auto-crop needs only the finished regions. Its request token belongs to that particular publication epoch: cancelled/superseded/foreign/closed requests cannot publish. Captured data snapshots retain no publication authority. Consumers capture only ready results matching current inputs. Completing a project write preserves live metadata and independent preparation.

DocumentScope owns the store and preparation task; immutable Document captures hold ready data without publication authority. Feature actions await [compatible ready regions](../src/editor/spatial.rs) through borrowed UiContext, then resume their own tools outside painting. Auto-crop's batch action belongs to the document; a Photo proposal belongs to its manual Crop draft. Retiring that draft drops its waiter while shared preparation continues; document replacement or application retirement cancels preparation. Running work retains its source leases through cleanup. Preparation uses the shared source loader, never borrows the selected Scan and retains no roll of mandatory full developments. Publication validates captured inputs against the live document; Analyze verification and Auto-crop Apply also check supporting peers, including those not visible or targeted.

## Runtime evidence and saved choices

Automatic observations, fitted geometry and sampling suggestions live with the open document. Save writes accepted crops, analysis support and grading; Open starts without spatial evidence and prepares it when a consumer requests it. Preparation alone neither creates an undoable edit nor prompts for Save.

Shared preparation owns modal feedback and blocks ordinary UI file actions. An internal captured write can overlap preparation without cancelling it or replacing its live results. A consumer that becomes ready during the write resumes after completion; a failed write retains it until failure acknowledgement. [Project I/O](projects.md#prepare-before-replacing) owns accepted-state validation and failure-atomic writes.

## Checking the library

```sh
cargo test --locked --offline --features project --lib spatial_metadata
cargo test --locked --offline --release --features project --lib preparation_tests
cargo test --locked --offline --release --features project --lib
cargo test --locked --offline --features project --test project
cargo clippy --locked --offline --features project --lib --tests -- -D warnings
```

The opt-in `observe_spatial_metadata_cached_roll` check uses `raw,project`, an explicit version-1 protocol and a fresh ignored output directory. It reads frozen camera-code proxies and runs native line detection, not RAW development or the editor. This evidence predates corrected Sony/HDR/direct-DNG paths; agreement and marked-window accounting are not current-development or independent whole-picture acceptance. Native setup follows the normal [contributor workflow](native-development.md#reusing-native-builds).

Start in [spatial metadata](../src/spatial_metadata.rs), [preparation](../src/scan_regions/material/framing/mod.rs), [consumer geometry](../src/auto_crop.rs) and [project I/O](../src/project.rs).

## Evidence and remaining acceptance

Supplied candidates and cached camera-code replays exercise geometry and consumer consistency; photographic content preservation still needs independent marks on current corrected Sony, HDR and direct-DNG captures, including partial frames and neighbours. Bounded native searches and camera-continuation rules have no calibrated photographic error rate. Native keyboard/pointer use and aggregate preparation memory/latency remain separate checks in [diagnostics](diagnostics.md). Shared detection and Analyze's original-pixel numerical sampling remain separate workloads; bounded captures do not establish their aggregate resource acceptance.
