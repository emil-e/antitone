# Shared film and picture metadata

Spatial metadata describes the developed source rather than a tool's preferred crop or sampling inset. Analyze and Auto-crop request the same document-owned preparation through normal source/CPU services. Ready regions can be reused and saved independently of either consumer's accepted choices. [Remaining acceptance](plans/physical-framing-production.md) separates authored workflow checks from photographic, physical-input and resource assessment.

## One description, independent consumers

The document owns reusable source-keyed data: picture regions, available film limits, holder-opening estimates and coarse film/holder/light/unknown classification. Some components can be unavailable. Native observations remain distinct from fitted picture bounds; each boundary is measured, inferred, camera-limited or unresolved. Coordinates are developed source pixels, before user orientation/crop; planes use inward unit normals. Source pose and axis skew retain both transformed raster directions even when companion boundaries are absent; unequal scale and active-area origin are not discarded.

A camera-limited side records unavailable physical extent, not a measured line at the sensor edge. Missing film or holder coverage describes availability separately from where the original picture frame would be. Unknown data cannot certify a full frame. Coarse material labels and pooled holder limits are estimates, not exact contours or calibrated confidence probabilities.

Auto-crop derives nominal picture framing within the available camera raster, independently of film, holder and material exclusions. Its inward margin is a photographic choice. Analyze instead qualifies complete sampling footprints against film/material/holder support; picture statistics additionally obey the nominal picture boundaries. Its source-scaled inward band avoids boundary contamination without moving shared edges. Authored sampling windows remain editable but do not license samples outside those physical supports. Both consumers leave shared facts unchanged; Analyze retains its review, fine measurements and original-grid numerical eligibility.

Either feature triggers preparation when ready metadata does not match current inputs. Analyze-first leaves reusable data even if grading/review is subsequently cancelled. Auto-crop-first does not invoke Analyze or accept sampling. Browsing or changing grades/margins does not redetect compatible sources. Requests capture batch target IDs before awaiting preparation, so changed selection cannot retarget a queued operation. A queued Photo proposal belongs to its original manual Crop visit.

## Preparation and common dimensions

Preparation consumes immutable bounded RGB16 camera-code captures and native line candidates, not UI state or a roll of developed-source leases. Normal admitted services acquire one developed source at a time and verify its identity, then retain only bounded captures while fitting regions. Measurement excludes the optional roll flat-field: embedded Sony/DNG development correction remains part of the source interpretation, not an optional measurement dependency. Grouping follows declared matching source geometry/raster and camera basis; the physical assumption is that transferring source-pixel extents is useful. Camera identity alone does not establish unchanged magnification or picture format, and this is not a new user-managed reference workflow.

W/H are common fitted parameters despite measurement variation. The current estimator uses median measured opposite-side extents independently per axis. It retains source identities supporting each axis; inferred bounds, accepted crops and sampling insets never feed back into size. There is no nominal format snapping or reciprocal/aspect folding. Preparation is stable under filmstrip ordering through source-keyed ordering.

Complete reference photographs are not a prerequisite. Differently clipped captures can supply complementary dimensions. If every top is unavailable, width and source-visible picture may remain useful without pretending that full height was measured. Full physical bounds, available framing and unknown components are kept separate.

With complete dimensions, native joint fitting poses three/four credible sides against fixed W/H; the adjacent-edge search handles two-side completion or failed joint fits. Every supplied side requalifies on the completed geometry. A film edge cannot become a measured picture edge, but unavailable film at an inferred side does not erase an otherwise supported nominal frame. Conflicting picture evidence and distinct competing-frame ambiguity still reject. Partial native observations need multiple credible sides and can contribute only their measured opposite-side dimensions. Without a complete-peer placement template, fitting/search uses common dimensions without inventing a pooled placement prior.

The original pooled selector can miss legitimate partial families at different translations. Preparation also checks a bounded set of existing native families when that selector supplies no qualified observation. This is not a new pixel/pose discovery grid or exhaustive ambiguity proof. Partial qualification uses finite camera-envelope companion spans internally, without publishing them as measured boundaries.

Camera-limit inference uses distributed photo-compatible edge support with local inward continuation. It is deliberately separate from transition measurement and does not require uniform darkness. Its current station/band settings are exploratory choices, not calibrated clipping uncertainty; ambiguous/missing support stays unresolved. Authored controls exercise all-clipped cases, a disconnected three-pixel edge strip, heterogeneous support, blanks and film cuts. These supplied-candidate controls do not prove native discovery or photographic accuracy.

Runtime estimates may use observations from the same sources they describe. Such results are training-conditioned, not held-out predictions. The retained strict evaluation path excludes targets upstream of pooled selection/appearance and dimension fitting; that validation guarantee is not a production requirement that every output be held out.

## Reuse and publication

Metadata records source IDs, cheap file revisions, source/development interpretation, measurement correction/code basis and a preparation version. File revisions are disposable freshness, not photographic identity or a RAW content hash. Changes to those inputs or replacement of boundary observations can invalidate dependent group results. Selecting, hiding, reordering, grading and changing consumer insets do not themselves change the observations.

A document-owned store retains immutable ready metadata. Its request token belongs to that particular publication epoch: cancelled/superseded/foreign/closed requests cannot publish. Captured data snapshots retain no publication authority. Consumers/save capture only ready results matching current inputs; pending work is not saveable output. Later publication cannot change a captured write, and completing a write must not replace later live metadata.

DocumentScope owns the store and preparation task; immutable Document captures hold ready data without publication authority. Feature actions register work through borrowed UiContext and complete outside painting. A photo-visit replacement does not retire shared preparation; document replacement or application retirement cancels its continuation. Running native work retains admission and source leases through cleanup. Preparation uses the existing scheduler, never borrows the selected Scan and retains no roll of mandatory full developments. Publication checks file identities again after worker completion; Analyze verification and Auto-crop Apply also check supporting peers, including those not visible or targeted.

## Saved derived data, not another accepted edit

The project reader/writer supports an optional `spatial_metadata` payload with its own format/interpretation version. Version3 separates nominal picture framing from material-qualified sampling and retains both transformed raster directions for anisotropic observations. Mismatched derived versions are unavailable and can be prepared again. Existing schema-6 accepted files without it remain supported; accepted processing contracts are unchanged. Older strict readers do not understand the new optional field and reject files containing it rather than silently reinterpret them. No old accepted-state migration is introduced.

Metadata-aware load detaches that payload from accepted project snapshots; metadata-aware Save captures it separately. Project equality remains real equality—there is no special equality implementation hiding derived changes. The document adapter uses those APIs and the separate store rather than installing automatic results into accepted project/history state.

Save/Save As remain available during shared preparation without enabling destructive document actions. They capture ready metadata without awaiting preparation or cancelling its continuation. A consumer that becomes ready during the write resumes after completion; a failed write retains it until failure acknowledgement. Save shares physical foreground admission, so already-running native work can still delay the write's execution. Automatic preparation alone is not an undoable edit or an unsaved-edit prompt; unsaved derived results may be discarded. Invalid/unsupported derived records are unavailable rather than accepted geometry. Accepted fields retain strict deserialization, bounded decoded strings and failure-atomic/no-clobber writes. Ignoring stale cache data does not bypass existing project/source compatibility checks or rewrite accepted crops/grades.

## Checking the library

```sh
cargo test --locked --offline --features project --lib spatial_metadata
cargo test --locked --offline --release --features project --lib preparation_tests
cargo test --locked --offline --release --features project --lib
cargo test --locked --offline --features project --test project
cargo clippy --locked --offline --features project --lib --tests -- -D warnings
```

The opt-in `observe_spatial_metadata_cached_roll` check uses `raw,project`, an explicit version-1 protocol and a fresh ignored output directory. It reads frozen camera-code proxies and runs native line detection, not RAW development or the editor. This evidence predates corrected Sony/HDR/direct-DNG paths; agreement and marked-window accounting are not current-development or independent whole-picture acceptance. Native setup follows the normal [contributor workflow](native-development.md#reusing-native-builds).

Start in [spatial metadata](../src/spatial_metadata.rs), [preparation](../src/scan_regions/framing/mod.rs), [consumer geometry](../src/auto_crop.rs) and [project I/O](../src/project.rs). The [acceptance plan](plans/physical-framing-production.md) owns remaining photographic, physical-input and resource assessment, not another detector-tuning campaign.
