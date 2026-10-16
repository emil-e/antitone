use crate::{
    Error,
    color::Balance,
    coverage::Coverage,
    crop::Crop,
    export::{ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::{ProcessingInput, ProcessingParameters, ProcessingSignature},
    local_adjustments::PhotoGradeSnapshot,
    platform::host,
    processing::{Calibration, PhotoSettings, RollSettings},
    raw::Rect,
    refinement::ViewRequest,
    rendering::OutputTarget,
    scan::Scan,
    viewer::{GpuImage, ImagePaint},
    viewport::{PickDrag, Viewport},
};
use glam::DVec3;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

#[path = "editor_cache_limits.rs"]
mod cache_limits;
#[path = "editor_controls.rs"]
mod controls;
#[path = "editor_development.rs"]
mod development;
#[path = "editor_document.rs"]
mod document;
#[path = "editor_icons.rs"]
mod icons;
#[path = "editor_preferences.rs"]
mod preferences;
#[path = "editor_ui.rs"]
mod presentation;
use crate::project::{Development, Source, SourceId};
use document::Document;
#[path = "editor_files.rs"]
mod files;
use files::{FileAction, Transition};
#[path = "editor_feedback.rs"]
mod feedback;
use feedback::{Failure, FailureKind};
#[path = "editor_activity.rs"]
mod activity;
use activity::{Task, TaskActivity, TaskProgress, TaskRequest};
#[path = "editor_fullscreen.rs"]
mod fullscreen;
use fullscreen::FullscreenView;
#[path = "editor_thumbnails.rs"]
mod thumbnails;
use thumbnails::Thumbnails;
#[path = "editor_processing.rs"]
mod processing;
use processing::Grades;
#[path = "editor_adjustments.rs"]
mod adjustments;
use adjustments::Adjustments;
#[path = "editor_working_view.rs"]
mod working_view;
use working_view::WorkingView;
#[path = "editor_original_view.rs"]
mod original_view;
use original_view::{OriginalPurpose, OriginalView};
#[path = "editor_selected_preview.rs"]
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
#[path = "editor_scopes.rs"]
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
#[path = "editor_analysis.rs"]
mod analysis;
#[path = "editor_analysis_review.rs"]
mod analysis_review;
#[path = "editor_dust.rs"]
mod dust;
#[path = "editor_flat_field.rs"]
mod flat;
use flat::{Correction, FlatEdit, FlatPreview};
#[path = "editor_auto_crop.rs"]
mod auto_crop;
#[path = "editor_crop.rs"]
mod crop_editor;
use crop_editor::CropEdit;
#[path = "editor_masks.rs"]
mod mask_editor;
use mask_editor::MaskEdit;
#[cfg(test)]
#[path = "editor_cache_preferences_tests.rs"]
mod cache_preferences_tests;
#[path = "editor_canvas.rs"]
mod canvas;
#[cfg(test)]
#[path = "editor_export_tests.rs"]
mod export_tests;
#[path = "editor_frame.rs"]
mod frame;
#[cfg(all(test, feature = "ui-tests"))]
#[path = "editor_kittest.rs"]
mod kittest_tests;
#[path = "editor_memory.rs"]
mod memory;
#[path = "editor_navigation.rs"]
mod navigation;
#[path = "editor_roll_selection.rs"]
mod roll_selection;
#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
#[path = "editor_viewer.rs"]
mod viewer_ui;
use roll_selection::RollSelection;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    Dmin,
    Dmax,
    Gamma,
    RollWb,
    Wb,
    Flat,
    Analysis,
}

struct PickReturn {
    path: PathBuf,
    selected: usize,
    adjustment: Option<(SourceId, adjustments::Target)>,
}

#[derive(Clone, Copy)]
struct PickView {
    source: bool,
    viewport: Viewport,
    anchored: Viewport,
}

struct Ready {
    file: thumbnails::FileKey,
    thumbnail: Option<(thumbnails::FileKey, Arc<thumbnails::Proxy>)>,
    path: PathBuf,
    area: Rect,
    // None: not requested; Some(None): no local hint (or explicit roll-search reset).
    analysis_support: Option<Option<Rect>>,
    floor: f64,
    proxy: crate::viewer::GpuProxy,
    record: Source,
    development: Development,
}

enum DocumentWork {
    Import(Document, Vec<PathBuf>, Option<(u64, SourceId)>),
    Open(PathBuf),
    New,
    Save(Document, PathBuf, crate::project::Save),
    Correction(Document, Option<SourceId>),
}

struct Prepared {
    document: Document,
    ready: Option<Ready>,
    selected: Option<SourceId>,
    reuse_ready: bool,
    correction: Option<Arc<Correction>>,
    preserve_view: bool,
}

struct LoadCache {
    store: Arc<crate::source_store::SourceStore>,
    parent: Option<Arc<crate::source_store::SourceStore>>,
    file: thumbnails::FileKey,
    key: Option<crate::source_store::SourceKey>,
}

impl LoadCache {
    fn new(
        path: &std::path::Path,
        expected: Option<&Source>,
        wb: Option<[f64; 4]>,
        document: &Document,
    ) -> Self {
        let file = thumbnails::FileKey::new(path);
        let key = expected.zip(wb).and_then(|(source, wb)| {
            crate::source_store::SourceKey::from_revision(
                path,
                file.clone(),
                &source.development,
                &Development::new(crate::raw::version(), wb),
            )
        });
        Self {
            store: document.cache().clone(),
            parent: document.parent_cache().cloned(),
            file,
            key,
        }
    }
}

struct ExportIdentity {
    source: u64,
    request: u64,
}

enum Command {
    Unload,
    Analyze(u64, Box<analysis::Request>),
    AnalysisAreas(u64, Box<analysis::Request>),
    AnalysisSupport(u64, u64),
    Dust(dust::Request),
    Refine(u64, u64, Box<ProcessingInput>, OutputTarget, ViewRequest),
    ViewSource(Arc<working_view::Mailbox>, egui::Context),
    Load(
        u64,
        PathBuf,
        Option<[f64; 4]>,
        Option<Box<Source>>,
        Option<Arc<Correction>>,
        Option<crate::project::CameraBasis>,
        LoadCache,
    ),
    FlatPreview(
        u64,
        PathBuf,
        Option<[f64; 4]>,
        Option<Box<Source>>,
        Option<crate::project::CameraBasis>,
        Option<LoadCache>,
    ),
    FlatAccept(u64, Rect, Box<Document>, Option<SourceId>),
    CancelFlat(u64),
    Document(u64, Box<DocumentWork>),
    Pick(
        u64,
        Pick,
        Coverage,
        Option<Calibration>,
        RollSettings,
        PhotoSettings,
    ),
    Export(
        ExportIdentity,
        PathBuf,
        OutputFormat,
        Calibration,
        RollSettings,
        PhotoGradeSnapshot,
        Composition,
        Overwrite,
    ),
}

#[derive(Clone, Copy)]
enum OutputFormat {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

enum Measurement {
    Reference(DVec3),
    Balance(Balance),
}

enum Message {
    DocumentProgress(u64, usize, usize),
    AnalysisSupport(u64, u64, Result<Option<Rect>, String>),
    AnalysisProgress(u64, usize, usize),
    AnalysisAreas(u64, Result<Option<analysis_review::Approved>, String>),
    Analyzed(u64, Result<Option<Box<analysis::Outcome>>, String>),
    Dust(u64, Result<Option<Box<dust::Response>>, String>),
    Refined(u64, Result<Option<Arc<GpuImage>>, String>),
    Ready(u64, Result<Box<Ready>, String>),
    Picked(u64, Pick, Result<Measurement, String>),
    Exported(u64, Result<PathBuf, String>),
    Prepared(u64, Result<Box<Prepared>, String>),
    Saved(u64, Result<Box<Document>, String>),
    FlatPreview(u64, Result<Box<FlatPreview>, String>),
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
) -> Result<(Ready, Scan), Error> {
    prepare_source_cached(
        renderers, device, queue, path, wb, expected, correction, basis, None,
    )
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn prepare_source_cached(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
    cache: Option<&LoadCache>,
) -> Result<(Ready, Scan), Error> {
    prepare_source_scheduled(
        renderers,
        device,
        queue,
        path,
        wb,
        expected,
        correction,
        basis,
        cache,
        &development::Scheduler::new(),
        &|| false,
    )
}

#[allow(clippy::too_many_arguments)]
fn prepare_source_scheduled(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: PathBuf,
    wb: Option<[f64; 4]>,
    expected: Option<&Source>,
    correction: Option<&Arc<Correction>>,
    basis: Option<&crate::project::CameraBasis>,
    cache: Option<&LoadCache>,
    developments: &development::Scheduler,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Ready, Scan), Error> {
    if let Some(expected) = expected {
        expected.grade_snapshot()?;
    }
    if cancelled() || cache.is_some_and(|cache| cache.store.is_closed()) {
        return Err(Error::UnusableSample("document source work retired"));
    }
    let started = std::time::Instant::now();
    let thumbnail_file = cache.map_or_else(
        || thumbnails::FileKey::new(&path),
        |cache| cache.file.clone(),
    );
    let retained = cache.and_then(|cache| {
        cache.key.as_ref().and_then(|key| {
            cache
                .store
                .get(key)
                .or_else(|| cache.parent.as_ref().and_then(|parent| parent.get(key)))
        })
    });
    let native_hit = retained.is_some();
    let tick = std::time::Instant::now();
    let source = match retained {
        Some(source) => source,
        None => developments.load(
            development::Request {
                path: path.clone(),
                wb,
                expected: expected.cloned(),
                basis: basis.cloned(),
                file: thumbnail_file.clone(),
                key: cache.and_then(|cache| cache.key.clone()),
                store: cache.map(|cache| cache.store.clone()),
                parent: cache.and_then(|cache| cache.parent.clone()),
                retain: true,
            },
            development::Priority::Foreground,
            cancelled,
        )?,
    };
    let development_seconds = tick.elapsed().as_secs_f64();
    let record = document::source_metadata(&path, &source.metadata())?;
    let registration_seconds = started.elapsed().as_secs_f64() - development_seconds;
    if let Some(basis) = basis {
        basis.require_source(&record.development)?;
    }
    if let Some(expected) = expected {
        document::verify_source(expected, &record)?;
    }
    let tick = std::time::Instant::now();
    let mut scan = Scan::from_source(source);
    document::verify_scan(&record, &scan)?;
    let verification_seconds = tick.elapsed().as_secs_f64();
    if cancelled() {
        return Err(Error::UnusableSample("source demand retired"));
    }
    if cache.is_some_and(|cache| cache.store.is_closed()) {
        return Err(Error::UnusableSample("document source work retired"));
    }
    // Revisions gate cache reuse/publication, not photographic acceptance. A stale
    // pre-work key cannot be refreshed here to relabel an authoritative development.
    let native_key = crate::source_store::SourceKey::from_revision(
        &path,
        thumbnail_file.clone(),
        &record.development,
        &Development::new(crate::raw::version(), scan.fixed_wb),
    );
    if let Some(cache) = cache
        && let Some(key) = &native_key
        && let Some(source) = cache
            .store
            .publish_source(key.clone(), scan.source().clone())?
    {
        scan = Scan::from_source(source);
    }
    if let Some(correction) = correction {
        correction.settings.require_source(&record.development)?;
        scan.set_flat_field(Some(correction.settings.map.clone()))?;
    }
    let tick = std::time::Instant::now();
    let (width, height, pixels) = scan.preview_source(1024)?;
    let proxy = cache
        .and_then(|cache| {
            native_key
                .as_ref()
                .and_then(|key| cache.store.get_proxy(key))
        })
        .map_or_else(|| thumbnails::Proxy::from_scan(&scan).map(Arc::new), Ok)?;
    if let Some(cache) = cache
        && let Some(key) = &native_key
    {
        cache.store.publish_proxy(key.clone(), proxy.clone())?;
    }
    let thumbnail = Some((thumbnail_file.clone(), proxy));
    let proxy_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let renderer = renderers.get(device, queue)?;
    let renderer_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let a = scan.area;
    let proxy = crate::viewer::GpuProxy::new(
        renderer,
        device,
        [width, height],
        &pixels,
        scan.camera_to_ap1,
        correction.map(|c| c.gpu.clone()),
        crate::gpu::SourceGeometry::new([a.x, a.y, a.width, a.height], [width, height])?,
    )?;
    let images_seconds = tick.elapsed().as_secs_f64();
    println!(
        "selected_source native_cache_hit={native_hit} seconds={:.6} registration_seconds={registration_seconds:.6} development_seconds={development_seconds:.6} verification_seconds={verification_seconds:.6} proxy_seconds={proxy_seconds:.6} renderer_seconds={renderer_seconds:.6} images_seconds={images_seconds:.6}",
        started.elapsed().as_secs_f64()
    );
    let ready = Ready {
        file: thumbnail_file,
        thumbnail,
        path,
        area: scan.area,
        analysis_support: None,
        floor: scan.input_floor,
        proxy,
        record,
        development: Development::new(crate::raw::version(), scan.fixed_wb),
    };
    Ok((ready, scan))
}

#[allow(clippy::too_many_arguments)]
fn prepare_document_scheduled(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &mut Option<(u64, Scan)>,
    id: u64,
    work: DocumentWork,
    previous_correction: Option<&Arc<Correction>>,
    developments: &development::Scheduler,
    cancelled: &dyn Fn() -> bool,
    progress: impl FnMut(usize, usize),
) -> Result<Prepared, Error> {
    let preserve_view = matches!(work, DocumentWork::Correction(..));
    let selected_override = match &work {
        DocumentWork::Correction(_, selected) => Some(*selected),
        _ => None,
    };
    let (document, reuse) = match work {
        DocumentWork::Import(mut document, paths, reuse) => {
            document.fork_cache();
            document.import_with_progress(&paths, Some(device), progress)?;
            (document, reuse)
        }
        DocumentWork::Open(path) => (
            Document::open_with_progress(&path, Some(device), progress)?,
            None,
        ),
        DocumentWork::New => (Document::default(), None),
        DocumentWork::Correction(mut document, _) => {
            document.fork_cache();
            (document, None)
        }
        DocumentWork::Save(..) => unreachable!("save does not prepare a RAW"),
    };
    let correction = Correction::prepare(device, &document, previous_correction)?;
    for record in &document.project.sources {
        crate::gpu::ProcessingSource::new(
            device,
            &[[0; 4]],
            glam::DMat3::from_cols_array_2d(&record.development.camera_to_ap1),
        )?;
    }
    if let Some((loaded, selected)) = reuse
        && let Some((active, _)) = source
        && *active == loaded
    {
        *active = id;
        return Ok(Prepared {
            document,
            ready: None,
            selected: Some(selected),
            reuse_ready: true,
            correction,
            preserve_view,
        });
    }
    let selected = selected_override.unwrap_or_else(|| document.project.order.first().copied());
    let prepared = if let Some(selected) = selected {
        let path = document.source_path(selected)?;
        let wb = document
            .project
            .development
            .as_ref()
            .map(|value| value.fixed_wb);
        let cache = LoadCache::new(
            &path,
            Some(document.project.source(selected)?),
            wb,
            &document,
        );
        let (ready, scan) = prepare_source_scheduled(
            renderers,
            device,
            queue,
            path,
            wb,
            Some(document.project.source(selected)?),
            correction.as_ref(),
            document.project.camera_basis.as_ref(),
            Some(&cache),
            developments,
            cancelled,
        )?;
        Some((ready, scan))
    } else {
        None
    };
    let ready = match prepared {
        Some((ready, scan)) => {
            *source = Some((id, scan));
            Some(ready)
        }
        None => {
            *source = None;
            None
        }
    };
    Ok(Prepared {
        document,
        ready,
        selected,
        reuse_ready: false,
        correction,
        preserve_view,
    })
}

#[allow(clippy::too_many_arguments)]
fn backend(
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    (queue, submissions): (wgpu::Queue, crate::platform::SubmissionGate),
    commands: mpsc::Receiver<Command>,
    messages: mpsc::Sender<Message>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    analysis_generation: Arc<std::sync::atomic::AtomicU64>,
    developments: development::Scheduler,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut source: Option<(u64, Scan)> = None;
    let mut correction: Option<Arc<Correction>> = None;
    let mut flat_preview: Option<(u64, Scan, Source)> = None;
    let mut view_kernels = None;
    let mut dust_screening =
        crate::gpu::dust_screening::Screening::new(&device, &queue, &submissions);
    while let Ok(command) = commands.recv() {
        match command {
            Command::AnalysisSupport(request, loaded) => {
                let result = source
                    .as_ref()
                    .filter(|(id, _)| {
                        *id == loaded
                            && source_epoch.load(std::sync::atomic::Ordering::Relaxed) == loaded
                    })
                    .map(|(_, scan)| scan.propose_flat_field_area().map_err(|e| e.to_string()))
                    .unwrap_or_else(|| Err("source demand retired".into()));
                if messages
                    .send(Message::AnalysisSupport(request, loaded, result))
                    .is_err()
                {
                    break;
                }
            }
            Command::AnalysisAreas(id, request) => {
                let result = analysis_review::prepare(
                    &request,
                    &analysis_generation,
                    &developments,
                    &device,
                    |done, total| {
                        let _ = messages.send(Message::AnalysisProgress(id, done, total));
                    },
                )
                .map_err(|e| e.to_string());
                if messages.send(Message::AnalysisAreas(id, result)).is_err() {
                    break;
                }
            }
            Command::Analyze(id, mut request) => {
                let result = if let Some((_, scan)) = source
                    .as_ref()
                    .filter(|(loaded, _)| *loaded == request.loaded)
                {
                    (|| {
                        if analysis_generation.load(std::sync::atomic::Ordering::Relaxed)
                            != request.revision
                        {
                            return Ok(None);
                        }
                        if request.areas.is_none()
                            && request.local_support
                            && request.film.is_none()
                        {
                            request.film =
                                scan.propose_flat_field_area().map_err(|e| e.to_string())?;
                        }
                        analysis::run(
                            &device,
                            &request,
                            &analysis_generation,
                            correction.as_ref(),
                            &developments,
                            |done, total| {
                                let _ = messages.send(Message::AnalysisProgress(id, done, total));
                            },
                        )
                        .map(|result| result.map(Box::new))
                        .map_err(|e| e.to_string())
                    })()
                } else {
                    Err("source changed before analysis".into())
                };
                if messages.send(Message::Analyzed(id, result)).is_err() {
                    break;
                }
            }
            Command::ViewSource(mailbox, context) => working_view::acquire(
                &mailbox,
                &context,
                &source,
                correction.as_ref(),
                &renderers,
                &mut view_kernels,
                &device,
                &queue,
                &submissions,
                &source_epoch,
            ),
            Command::Dust(request) => {
                let cancelled = || {
                    use std::sync::atomic::Ordering;
                    if source_epoch.load(Ordering::Relaxed) != request.loaded {
                        request.cancelled.store(true, Ordering::Relaxed);
                    }
                    request.cancelled.load(Ordering::Relaxed)
                };
                let result = match source
                    .as_ref()
                    .filter(|(loaded, _)| *loaded == request.loaded && !cancelled())
                {
                    Some((_, scan)) => {
                        dust::evaluate_with(scan, &request, Some(&mut dust_screening), &cancelled)
                            .map(|r| r.map(Box::new))
                            .map_err(|e| e.to_string())
                    }
                    None => Ok(None),
                };
                let result = if cancelled() { Ok(None) } else { result };
                if messages
                    .send(Message::Dust(request.request, result))
                    .is_err()
                {
                    break;
                }
                request.wake.request_repaint();
            }
            Command::Refine(id, revision, parameters, target, view) => {
                let active = match &flat_preview {
                    Some((loaded, scan, _)) if *loaded == id => Some((scan, None)),
                    _ => source
                        .as_ref()
                        .filter(|(loaded, _)| *loaded == id)
                        .map(|(_, scan)| (scan, correction.as_ref().map(|c| c.gpu.clone()))),
                };
                let result = match active {
                    Some(context) => renderers
                        .get(&device, &queue)
                        .and_then(|renderer| {
                            crate::refinement::refine(
                                &renderer,
                                &device,
                                (&queue, &submissions),
                                context,
                                *parameters,
                                target,
                                (&generation, revision),
                                view,
                            )
                        })
                        .map_err(|e| e.to_string()),
                    _ => Ok(None),
                };
                if messages.send(Message::Refined(revision, result)).is_err() {
                    break;
                }
            }
            Command::Unload => {
                source = None;
            }
            Command::Load(id, path, wb, expected, field, basis, cache) => {
                let result = prepare_source_scheduled(
                    &renderers,
                    &device,
                    &queue,
                    path,
                    wb,
                    expected.as_deref(),
                    field.as_ref(),
                    basis.as_ref(),
                    Some(&cache),
                    &developments,
                    &|| source_epoch.load(std::sync::atomic::Ordering::Relaxed) != id,
                )
                .map(|(ready, scan)| {
                    source = Some((id, scan));
                    correction = field;
                    Box::new(ready)
                })
                .map_err(|error| error.to_string());
                if messages.send(Message::Ready(id, result)).is_err() {
                    break;
                }
            }
            Command::FlatPreview(id, path, wb, expected, basis, cache) => {
                let result = prepare_source_scheduled(
                    &renderers,
                    &device,
                    &queue,
                    path,
                    wb,
                    expected.as_deref(),
                    None,
                    basis.as_ref(),
                    cache.as_ref(),
                    &developments,
                    &|| source_epoch.load(std::sync::atomic::Ordering::Relaxed) != id,
                )
                .and_then(|(ready, scan)| {
                    let region = scan.propose_flat_field_area()?;
                    flat_preview = Some((id, scan, ready.record.clone()));
                    Ok(Box::new(FlatPreview { ready, region }))
                })
                .map_err(|e| e.to_string());
                if messages.send(Message::FlatPreview(id, result)).is_err() {
                    break;
                }
            }
            Command::CancelFlat(id) => {
                if flat_preview
                    .as_ref()
                    .is_some_and(|(loaded, _, _)| *loaded == id)
                {
                    flat_preview = None;
                }
            }
            Command::FlatAccept(id, region, mut document, selected) => {
                let result = (|| -> Result<Box<Prepared>, Error> {
                    let (_, scan, record) = flat_preview
                        .as_mut()
                        .filter(|(loaded, _, _)| *loaded == id)
                        .ok_or(Error::UnusableSample("flat-field reference changed"))?;
                    let map = Arc::new(scan.estimate_flat_field(region)?);
                    let reference = document.register_reference(
                        record.clone(),
                        Development::new(crate::raw::version(), scan.fixed_wb),
                    )?;
                    let enabled = document
                        .project
                        .flat_field
                        .as_ref()
                        .is_none_or(|field| field.enabled);
                    let mut settings = crate::project::FlatFieldSettings::new(
                        document.project.source(reference)?,
                        map,
                        document
                            .project
                            .development
                            .as_ref()
                            .ok_or(Error::UnusableSample("flat-field roll development missing"))?,
                    );
                    settings.enabled = enabled;
                    document.project.flat_field = Some(settings);
                    let prepared = prepare_document_scheduled(
                        &renderers,
                        &device,
                        &queue,
                        &mut source,
                        id,
                        DocumentWork::Correction(*document, selected),
                        correction.as_ref(),
                        &developments,
                        &|| source_epoch.load(std::sync::atomic::Ordering::Relaxed) != id,
                        |_, _| {},
                    )?;
                    correction = prepared.correction.clone();
                    flat_preview = None;
                    Ok(Box::new(prepared))
                })()
                .map_err(|e| e.to_string());
                if messages.send(Message::Prepared(id, result)).is_err() {
                    break;
                }
            }
            Command::Document(id, work) => {
                let message = match *work {
                    DocumentWork::Save(document, path, policy) => Message::Saved(
                        id,
                        document
                            .save_with_device(&path, policy, Some(&device))
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    ),
                    work => Message::Prepared(
                        id,
                        prepare_document_scheduled(
                            &renderers,
                            &device,
                            &queue,
                            &mut source,
                            id,
                            work,
                            correction.as_ref(),
                            &developments,
                            &|| source_epoch.load(std::sync::atomic::Ordering::Relaxed) != id,
                            |done, total| {
                                let _ = messages.send(Message::DocumentProgress(id, done, total));
                            },
                        )
                        .map(|prepared| {
                            correction = prepared.correction.clone();
                            Box::new(prepared)
                        })
                        .map_err(|error| error.to_string()),
                    ),
                };
                if messages.send(message).is_err() {
                    break;
                }
            }
            Command::Pick(id, kind, region, calibration, roll, photo) => {
                let result = (|| -> Result<Measurement, Error> {
                    let (loaded, scan) = source
                        .as_mut()
                        .ok_or(Error::UnusableSample("no developed source"))?;
                    if id != *loaded {
                        return Err(Error::UnusableSample("source changed before sampling"));
                    }
                    match kind {
                        Pick::Dmin | Pick::Dmax => {
                            scan.reference_coverage(region).map(Measurement::Reference)
                        }
                        Pick::Gamma => scan
                            .roll_neutral_coverage(
                                region,
                                calibration.ok_or(Error::UnusableSample(
                                    "both film references are required",
                                ))?,
                                roll,
                            )
                            .map(Measurement::Balance),
                        Pick::Flat | Pick::Analysis => Err(Error::InvalidParameter(
                            "area acceptance uses a separate transaction",
                        )),
                        Pick::RollWb => scan
                            .photo_neutral_coverage(
                                region,
                                calibration.ok_or(Error::UnusableSample(
                                    "both film references are required",
                                ))?,
                                RollSettings {
                                    wb: Balance::default(),
                                    ..roll
                                },
                                PhotoSettings::default(),
                            )
                            .map(Measurement::Balance),
                        Pick::Wb => scan
                            .photo_neutral_coverage(
                                region,
                                calibration.ok_or(Error::UnusableSample(
                                    "both film references are required",
                                ))?,
                                roll,
                                photo,
                            )
                            .map(Measurement::Balance),
                    }
                })()
                .map_err(|e| e.to_string());
                if messages.send(Message::Picked(id, kind, result)).is_err() {
                    break;
                }
            }
            Command::Export(id, path, format, calibration, roll, grade, composition, overwrite) => {
                let result = (|| -> Result<PathBuf, Error> {
                    let (loaded, scan) = source
                        .as_ref()
                        .ok_or(Error::UnusableSample("no developed source"))?;
                    if id.source != *loaded {
                        return Err(Error::UnusableSample("source changed before export"));
                    }
                    let renderer = renderers.get(&device, &queue)?;
                    crate::gpu_export::write(
                        &renderer,
                        &device,
                        (&queue, &submissions),
                        crate::gpu_export::Request {
                            scan,
                            field: correction.as_ref().map(|c| c.gpu.clone()),
                            calibration,
                            roll,
                            grade,
                            composition,
                            format: match format {
                                OutputFormat::Tiff(target) => {
                                    crate::gpu_export::Format::Tiff(target)
                                }
                                OutputFormat::Exr(precision) => {
                                    crate::gpu_export::Format::Exr(precision)
                                }
                            },
                        },
                        &path,
                        overwrite,
                    )?;
                    Ok(path)
                })()
                .map_err(|e| e.to_string());
                if messages
                    .send(Message::Exported(id.request, result))
                    .is_err()
                {
                    break;
                }
            }
        }
    }
}

pub struct Editor {
    started: bool,
    submissions: crate::platform::SubmissionGate,
    analysis_generation: Arc<std::sync::atomic::AtomicU64>,
    analysis_pending: Option<analysis::Pending>,
    analysis_area_pending: Option<u64>,
    analysis_preparing: Option<analysis::Pending>,
    analysis_review: Option<analysis_review::Review>,
    thumbnails: Thumbnails,
    scopes: Scopes,
    grades: Grades,
    working: WorkingView,
    original: OriginalView,
    dust: dust::State,
    reset_repairs_pending: bool,
    viewport: Viewport,
    fullscreen: Option<FullscreenView>,
    commands: mpsc::Sender<Command>,
    messages: mpsc::Receiver<Message>,
    id: u64,
    ready: Option<Ready>,
    pending_preview: Option<thumbnails::SelectedPreview>,
    selection_timing: Option<Arc<SelectionTiming>>,
    busy: bool,
    status: String,
    tasks: TaskActivity,
    failure: Option<Failure>,
    document_failure: Option<FailureKind>,
    return_notice: Option<String>,
    dmin: Option<DVec3>,
    dmax: Option<DVec3>,
    floor: f64,
    roll: RollSettings,
    photo: PhotoSettings,
    adjustments: Adjustments,
    roll_orientation: RollOrientation,
    photo_orientation: PhotoOrientation,
    crop: Option<Crop>,
    crop_edit: Option<CropEdit>,
    auto_crop_edit: Option<auto_crop::Draft>,
    auto_crop_requested: Option<auto_crop::Target>,
    auto_crop_margin: f64,
    mask_edit: Option<MaskEdit>,
    last_layout: Option<(crate::viewport::CanvasLayout, f32)>,
    target: OutputTarget,
    source: bool,
    source_exposure: f64,
    pick: Option<Pick>,
    pick_view: Option<PickView>,
    pick_return: Option<PickReturn>,
    region: Option<Coverage>,
    pick_drag: Option<PickDrag>,
    tab: usize,
    preferences: preferences::Preferences,
    preference_store: Option<preferences::Store>,
    new_project_pending: bool,
    document: Document,
    current_source: Option<SourceId>,
    next_request: u64,
    document_pending: Option<u64>,
    export_pending: Option<u64>,
    transition: Option<Transition>,
    after_save: Option<Transition>,
    resume_transition: Option<Transition>,
    project_overwrite: Option<(PathBuf, Option<Transition>)>,
    allow_close: bool,
    selected: usize,
    roll_selection: RollSelection,
    removal_pending: Option<Vec<SourceId>>,
    overwrite: Option<(PathBuf, OutputFormat)>,
    correction: Option<Arc<Correction>>,
    flat_edit: Option<FlatEdit>,
    developments: Option<development::Scheduler>,
    source_epoch: Arc<std::sync::atomic::AtomicU64>,
}

impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, initial: Option<PathBuf>) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let submissions = crate::platform::SubmissionGate::default();
        let renderers = Arc::new(crate::renderer::RendererCache::new(submissions.clone()));
        renderers.prewarm(device.clone(), queue.clone());
        let developments = development::Scheduler::new();
        let thumbnails = Thumbnails::new(
            device.clone(),
            queue.clone(),
            renderers.clone(),
            developments.clone(),
        );
        let worker_developments = developments.clone();
        let source_epoch = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_source_epoch = source_epoch.clone();
        let scopes = Scopes::new(device);
        let grades = Grades::new(device.clone());
        let device = device.clone();
        let queue = queue.clone();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_generation = generation.clone();
        let analysis_generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_analysis_generation = analysis_generation.clone();
        let mut editor = Self::state(commands, messages, generation);
        editor.submissions = submissions.clone();
        editor.working = WorkingView::new(device.clone());
        std::thread::spawn(move || {
            backend(
                renderers,
                device,
                (queue, submissions),
                receiver,
                send,
                worker_generation,
                worker_analysis_generation,
                worker_developments,
                worker_source_epoch,
            )
        });
        editor.analysis_generation = analysis_generation;
        editor.developments = Some(developments);
        editor.source_epoch = source_epoch;
        editor.thumbnails = thumbnails;
        editor
            .thumbnails
            .bind_sources(editor.document.cache().clone());
        editor.preferences.cache_limits = Some(editor.thumbnails.limits);
        editor.scopes = scopes;
        editor.grades = grades;
        if let Some(path) = initial {
            if path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("arw"))
            {
                editor.add_raws(vec![path]);
                editor.new_project_pending = editor.document_pending.is_some();
            } else {
                editor.start_document(DocumentWork::Open(path));
            }
        }
        editor
    }

    fn state(
        commands: mpsc::Sender<Command>,
        messages: mpsc::Receiver<Message>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        let document = Document::default();
        let mut thumbnails = Thumbnails::default();
        thumbnails.bind_sources(document.cache().clone());
        Self {
            submissions: crate::platform::SubmissionGate::default(),
            developments: None,
            source_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            analysis_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            analysis_pending: None,
            analysis_area_pending: None,
            analysis_preparing: None,
            analysis_review: None,
            thumbnails,
            scopes: Scopes::default(),
            grades: Grades::default(),
            working: WorkingView::default(),
            original: OriginalView::new(generation),
            dust: dust::State::default(),
            reset_repairs_pending: false,
            viewport: Viewport::default(),
            fullscreen: None,
            started: false,
            commands,
            messages,
            id: 0,
            ready: None,
            pending_preview: None,
            selection_timing: None,
            busy: false,
            status: "Add a RAW to begin".into(),
            tasks: TaskActivity::default(),
            failure: None,
            document_failure: None,
            return_notice: None,
            dmin: None,
            dmax: None,
            floor: 0.,
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),
            adjustments: Adjustments::default(),
            roll_orientation: RollOrientation::default(),
            photo_orientation: PhotoOrientation::default(),
            crop: None,
            crop_edit: None,
            auto_crop_edit: None,
            auto_crop_requested: None,
            auto_crop_margin: crate::auto_crop::DEFAULT_MARGIN_PERCENT,
            mask_edit: None,
            last_layout: None,
            target: OutputTarget::DisplayP3,
            source: true,
            source_exposure: 0.,
            pick: None,
            pick_view: None,
            pick_return: None,
            region: None,
            pick_drag: None,
            tab: 0,
            preferences: preferences::Preferences::default(),
            preference_store: None,
            new_project_pending: false,
            document,
            current_source: None,
            next_request: 0,
            document_pending: None,
            export_pending: None,
            transition: None,
            after_save: None,
            resume_transition: None,
            project_overwrite: None,
            allow_close: false,
            selected: 0,
            roll_selection: RollSelection::default(),
            removal_pending: None,
            overwrite: None,
            correction: None,
            flat_edit: None,
        }
    }

    pub fn restore_ui_preferences(&mut self) {
        match preferences::Store::open_default() {
            Ok((store, preferences)) => {
                self.apply_preferences(preferences);
                self.preference_store = Some(store);
                self.persist_preferences();
                if self.document_pending.is_none() || self.new_project_pending {
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
            }
            Err(error) => eprintln!("UI preferences not loaded (file left unchanged): {error}"),
        }
    }

    fn apply_preferences(&mut self, mut preferences: preferences::Preferences) {
        let limits = *preferences
            .cache_limits
            .get_or_insert(self.thumbnails.limits);
        self.thumbnails.set_limits(limits);
        self.scopes.visible = preferences.scopes_visible;
        self.scopes.height_ratio = preferences.scope_height_ratio;
        self.preferences = preferences;
    }

    fn persist_preferences(&mut self) {
        if let Some(store) = &mut self.preference_store
            && let Err(error) = store.save(&self.preferences)
        {
            self.status = format!("Unable to save application preferences: {error}");
            eprintln!("{}", self.status);
        }
    }

    fn cache_preferences(&mut self, ui: &mut egui::Ui) {
        use cache_limits::{MAX_MIB, MIB};
        ui.set_min_width(330.);
        let mut limits = self.thumbnails.limits;
        let mut changed = false;
        for (label, bytes) in [
            ("CPU cache", &mut limits.cpu_bytes),
            ("GPU cache", &mut limits.gpu_bytes),
        ] {
            let mut mib = *bytes / MIB;
            let response = presentation::labelled_row(
                ui,
                label,
                Some("Optional retained storage; not total application memory."),
                |ui, _| {
                    ui.add(
                        egui::DragValue::new(&mut mib)
                            .range(0..=MAX_MIB)
                            .speed(32.)
                            .update_while_editing(false)
                            .suffix(" MiB"),
                    )
                },
            )
            .inner;
            response.widget_info(|| {
                let mut info = egui::WidgetInfo::drag_value(response.enabled(), mib as f64);
                info.label = Some(label.into());
                info
            });
            if response.changed() && response.enabled() && *bytes != mib * MIB {
                *bytes = mib * MIB;
                changed = true;
            }
        }
        if presentation::TextAction::new("Reset cache limits")
            .show(ui)
            .clicked()
        {
            limits = self.thumbnails.default_limits();
            changed = true;
        }
        ui.label(
            "0 disables optional retention. Selected and visible work may exceed these limits.",
        );
        if changed {
            self.thumbnails.set_limits(limits);
            self.preferences.cache_limits = Some(limits);
            self.persist_preferences();
        }
    }

    fn calibration(&self) -> Option<Calibration> {
        Calibration {
            dmin: self.dmin?,
            dmax: self.dmax?,
            input_floor: self.floor,
        }
        .validate()
        .ok()
    }

    fn seed_thumbnail(&mut self) {
        self.thumbnails.bind_sources(self.document.cache().clone());
        let Some(ready) = &mut self.ready else {
            return;
        };
        let Some((file, proxy)) = ready.thumbnail.take() else {
            return;
        };
        let Some(id) = self.current_source else {
            return;
        };
        let source = self.document.project.source(id).expect("accepted source");
        // Flat-field editing displays a reference while retaining the working source ID.
        let path = self
            .document
            .source_path(id)
            .expect("accepted source locator");
        if document::same_path(&path, &ready.record.path)
            && source.development == ready.record.development
            && self.document.project.development.as_ref() == Some(&ready.development)
        {
            self.thumbnails
                .seed_at(source, &ready.development, &ready.path, file, proxy);
        }
    }

    fn sync_document(&mut self) {
        let project = &mut self.document.project;
        project.roll = self.roll;
        project.orientation = self.roll_orientation;
        project.processing_model = self.roll.response.processing_model();
        project.dmin = self.dmin.map(|value| value.to_array());
        project.dmax = self.dmax.map(|value| value.to_array());
        if project.input_floor.is_some() || self.dmin.is_some() || self.dmax.is_some() {
            project.input_floor = (self.floor > 0.).then_some(self.floor);
        }
        project.output.tiff = match self.target {
            OutputTarget::Srgb => crate::project::TiffTarget::Srgb,
            OutputTarget::DisplayP3 => crate::project::TiffTarget::DisplayP3,
        };
        if let Some(id) = self.current_source {
            let source = project
                .source_mut(id)
                .expect("active source belongs to document");
            source.photo = self.photo;
            source.orientation = self.photo_orientation;
            source.crop = self.crop;
        }
    }

    fn request_id(&mut self) -> u64 {
        self.next_request += 1;
        self.next_request
    }

    fn start_document(&mut self, work: DocumentWork) {
        self.finish_auto_crop(false);
        self.finish_dust_cleanup();
        self.reset_repairs_pending = matches!(&work, DocumentWork::New | DocumentWork::Open(_));
        if self.analysis_preparing.take().is_some() {
            self.cancel_analysis();
        }
        if let Some(review) = self.analysis_review.take() {
            self.tab = review.return_tab;
            self.scopes.visible = review.return_scopes_visible;
            self.pick = None;
            self.region = None;
            self.pick_return = None;
            self.pick_view = None;
        }
        self.analysis_area_pending = None;
        self.pending_preview = None;
        self.selection_timing = None;
        if self.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        let id = self.request_id();
        self.source_epoch
            .store(id, std::sync::atomic::Ordering::Relaxed);
        self.original.invalidate();
        self.document_pending = Some(id);
        self.tasks.begin(match &work {
            DocumentWork::Import(..) => Task::Import(id),
            DocumentWork::Open(_) => Task::Open(id),
            DocumentWork::New => Task::New(id),
            DocumentWork::Save(..) => Task::Save(id),
            DocumentWork::Correction(..) => Task::Correction(id),
        });
        self.document_failure = match &work {
            DocumentWork::Save(..) => Some(FailureKind::Save),
            DocumentWork::Correction(..) => Some(FailureKind::FlatCorrection),
            _ => None,
        };
        self.new_project_pending = matches!(work, DocumentWork::New);
        self.busy = true;
        self.status = "Preparing project…".into();
        if self
            .commands
            .send(Command::Document(id, Box::new(work)))
            .is_err()
        {
            self.document_pending = None;
            self.new_project_pending = false;
            self.tasks.finish(TaskRequest::Document(id));
            self.busy = false;
            self.status = "RAW worker stopped".into();
            if let Some(kind) = self.document_failure.take() {
                self.report_failure(kind, self.status.clone());
            }
        }
    }

    fn add_raws(&mut self, paths: Vec<PathBuf>) {
        self.sync_document();
        let reuse = self
            .ready
            .as_ref()
            .and(self.current_source)
            .map(|source| (self.id, source));
        self.start_document(DocumentWork::Import(self.document.clone(), paths, reuse));
    }

    fn load(&mut self, path: PathBuf) {
        self.load_source(path, true);
    }

    fn load_source(&mut self, path: PathBuf, persistent: bool) {
        self.sync_document();
        if let Err(error) = self.validate_document_candidate(&self.document) {
            self.status = format!("Source not changed: {error}");
            return;
        }
        self.finish_dust_cleanup();
        self.dust.release_gpu();
        let started = std::time::Instant::now();
        let selecting_photo = persistent && self.pick.is_none() && self.flat_edit.is_none();
        self.analysis_area_pending = None;
        self.seed_thumbnail();
        self.pending_preview = None;
        self.selection_timing = None;
        if self.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        self.adjustments = Adjustments::default();
        self.mask_edit = None;
        self.current_source = self.document.find_source(&path);
        if selecting_photo {
            self.roll_selection
                .reconcile(&self.document.project.order, self.current_source);
        }
        self.photo = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .map(|source| source.photo)
            .unwrap_or_default();
        self.photo_orientation = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .map(|source| source.orientation)
            .unwrap_or_default();
        self.crop = self
            .current_source
            .and_then(|id| self.document.project.source(id).ok())
            .and_then(|source| source.crop);
        self.crop_edit = None;
        self.last_layout = None;
        let expected = self.current_source.map(|id| {
            Box::new(
                self.document
                    .project
                    .source(id)
                    .expect("registered source")
                    .clone(),
            )
        });
        let wb = self
            .document
            .project
            .development
            .as_ref()
            .map(|value| value.fixed_wb);
        self.original.invalidate();
        self.viewport.fit();
        self.overwrite = None;
        self.return_notice = None;
        self.pick_view = None;
        self.pick_return = None;
        self.id = self.request_id();
        self.source_epoch
            .store(self.id, std::sync::atomic::Ordering::Relaxed);
        self.ready = None;
        self.selection_timing =
            selecting_photo.then(|| Arc::new(SelectionTiming::new(self.id, started)));
        self.region = None;
        self.pick = None;
        self.source = self
            .auto_crop_edit
            .as_ref()
            .map_or(self.calibration().is_none(), |d| d.source_view);
        self.busy = true;
        self.status = "Developing full-resolution RAW…".into();
        self.tasks.begin(Task::Photo(self.id));
        let cache = LoadCache::new(&path, expected.as_deref(), wb, &self.document);
        if self
            .commands
            .send(Command::Load(
                self.id,
                path,
                wb,
                expected,
                self.correction.clone(),
                self.document.project.camera_basis.clone(),
                cache,
            ))
            .is_err()
        {
            self.busy = false;
            self.tasks.finish(TaskRequest::Photo(self.id));
            self.selection_timing = None;
            self.status = "RAW worker stopped".into();
        } else if selecting_photo
            && self.pick.is_none()
            && self.flat_edit.is_none()
            && let Some(source) = self.current_source
        {
            match self.thumbnails.selected_preview(
                self.id,
                &self.document,
                source,
                self.correction.as_ref(),
            ) {
                Ok(preview) => {
                    println!(
                        "selected_switch request={} cache_hit={} boundary=admission elapsed_ms={:.6}",
                        self.id,
                        preview.is_some(),
                        started.elapsed().as_secs_f64() * 1000.
                    );
                    self.pending_preview = preview;
                }
                Err(error) => eprintln!(
                    "selected_preview request={} unavailable reason={error}",
                    self.id
                ),
            }
        }
    }

    fn receive(&mut self) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
                Message::DocumentProgress(id, done, total) => {
                    if self.document_pending == Some(id) {
                        self.tasks.report(
                            TaskRequest::Document(id),
                            TaskProgress::Counted {
                                completed: done,
                                total,
                            },
                        );
                    }
                }
                Message::AnalysisSupport(request, loaded, result) => {
                    self.analysis_support_result(request, loaded, result)
                }
                Message::AnalysisAreas(id, result) => self.analysis_areas_result(id, result),
                Message::AnalysisProgress(id, done, total) => {
                    if let Some(pending) = self
                        .analysis_pending
                        .as_mut()
                        .or(self.analysis_preparing.as_mut())
                        && pending.id == id
                    {
                        self.tasks.report(
                            TaskRequest::Analyze(id),
                            TaskProgress::Counted {
                                completed: done,
                                total,
                            },
                        );
                        if self
                            .analysis_generation
                            .load(std::sync::atomic::Ordering::Relaxed)
                            == pending.request.revision
                        {
                            self.status = if done < total {
                                format!("Analyzing frame {} of {total}…", done + 1)
                            } else {
                                "Fitting gamma/WB/exposures and validating proposal…".into()
                            };
                        }
                    }
                }
                Message::Analyzed(id, result) => self.analysis_result(id, result),
                Message::Dust(request, result) => {
                    self.receive_dust(request, result.map(|r| r.map(|r| *r)))
                }
                Message::Refined(revision, result) => {
                    if let Some(error) = self.original.receive(revision, result) {
                        self.status = error;
                    }
                }
                Message::Ready(id, result) if id == self.id && self.document_pending.is_none() => {
                    self.tasks.finish(TaskRequest::Photo(id));
                    self.pending_preview = None;
                    if let Some(timing) = &self.selection_timing {
                        println!(
                            "selected_switch request={id} success={} boundary=backend_ready elapsed_ms={:.6}",
                            result.is_ok(),
                            timing.started.elapsed().as_secs_f64() * 1000.
                        );
                    }
                    self.busy = false;
                    match result {
                        Ok(ready) => {
                            if self.floor == 0. {
                                self.floor = ready.floor;
                            }
                            if self.current_source.is_none() {
                                let mut record = ready.record.clone();
                                record.photo = self.photo;
                                record.orientation = self.photo_orientation;
                                record.crop = self.crop;
                                match self.document.register(record, ready.development.clone()) {
                                    Ok(id) => self.current_source = Some(id),
                                    Err(error) => {
                                        self.status = error.to_string();
                                        continue;
                                    }
                                }
                            }
                            self.status = format!(
                                "{} • {} × {}",
                                ready.path.file_name().unwrap_or_default().to_string_lossy(),
                                ready.area.width,
                                ready.area.height,
                            );
                            if let Some(notice) = self.return_notice.take() {
                                self.status = notice;
                            }
                            self.ready = Some(*ready);
                        }
                        Err(error) => {
                            self.selection_timing = None;
                            self.status = error;
                        }
                    }
                }
                Message::Picked(id, kind, result)
                    if id == self.id && self.document_pending.is_none() =>
                {
                    self.tasks.finish(TaskRequest::Measurement(id, kind));
                    self.busy = false;
                    match result {
                        Ok(Measurement::Reference(value)) => {
                            let other = if kind == Pick::Dmin {
                                self.dmax
                            } else {
                                self.dmin
                            };
                            let valid = other
                                .map(|other| {
                                    let (dmin, dmax) = if kind == Pick::Dmin {
                                        (value, other)
                                    } else {
                                        (other, value)
                                    };
                                    Calibration {
                                        dmin,
                                        dmax,
                                        input_floor: self.floor,
                                    }
                                    .validate()
                                    .and_then(|cal| {
                                        self.validate_processing(cal, self.roll, self.photo)
                                            .map(|_| cal)
                                    })
                                })
                                .transpose();
                            if let Err(error) = valid {
                                self.status = error.to_string();
                                self.finish_pick();
                                continue;
                            }
                            if kind == Pick::Dmin {
                                self.dmin = Some(value);
                            } else {
                                self.dmax = Some(value);
                            }
                            self.status = format!("{kind:?} measured");
                        }
                        Ok(Measurement::Balance(balance)) => {
                            let mut roll = self.roll;
                            let mut photo = self.photo;
                            match kind {
                                Pick::Gamma => roll.balance = balance,
                                Pick::RollWb => roll.wb = balance,
                                _ => photo.wb = balance,
                            }
                            if let Some(cal) = self.calibration()
                                && let Err(error) = self.validate_processing(cal, roll, photo)
                            {
                                self.status = format!("Pick rejected: {error}");
                                self.finish_pick();
                                continue;
                            }
                            self.roll = roll;
                            self.photo = photo;
                            self.status = format!("{kind:?} neutral correction applied");
                        }
                        Err(error) => self.status = error,
                    }
                    self.finish_pick();
                }
                Message::Prepared(id, result) if self.document_pending == Some(id) => {
                    self.tasks.finish(TaskRequest::Document(id));
                    let new_project = std::mem::take(&mut self.new_project_pending);
                    let failure_kind = self.document_failure.take();
                    self.document_pending = None;
                    self.busy = false;
                    match result {
                        Ok(mut prepared) => {
                            let flat_context = self.flat_edit.take();
                            let previous_cache = self.document.cache().clone();
                            self.thumbnails
                                .bind_sources(prepared.document.cache().clone());
                            prepared.document.accept_cache(&previous_cache);
                            if !Arc::ptr_eq(&previous_cache, prepared.document.cache()) {
                                previous_cache.close();
                            }
                            if self.reset_repairs_pending {
                                self.dust = dust::State::default();
                            }
                            self.reset_repairs_pending = false;
                            self.correction = prepared.correction;
                            self.document = prepared.document;
                            self.adjustments = Adjustments::default();
                            self.mask_edit = None;
                            self.grades.clear();
                            if new_project {
                                self.tab = 0;
                                self.preferences
                                    .sections
                                    .insert("roll-calibration".into(), true);
                            }
                            self.id = id;
                            self.current_source = prepared.selected;
                            if !prepared.reuse_ready && !prepared.preserve_view {
                                self.roll_selection = RollSelection::default();
                            }
                            self.roll_selection
                                .reconcile(&self.document.project.order, self.current_source);
                            self.selected = prepared
                                .selected
                                .and_then(|id| {
                                    self.document
                                        .project
                                        .order
                                        .iter()
                                        .position(|candidate| *candidate == id)
                                })
                                .unwrap_or(0);
                            self.roll = self.document.project.roll;
                            self.roll_orientation = self.document.project.orientation;
                            self.dmin = self.document.project.dmin.map(DVec3::from_array);
                            self.dmax = self.document.project.dmax.map(DVec3::from_array);
                            self.floor = self.document.project.input_floor.unwrap_or_else(|| {
                                prepared
                                    .ready
                                    .as_ref()
                                    .map_or(self.floor, |ready| ready.floor)
                            });
                            if prepared.selected.is_none() {
                                self.floor = self.document.project.input_floor.unwrap_or(0.);
                            }
                            self.photo = prepared
                                .selected
                                .and_then(|id| self.document.project.source(id).ok())
                                .map(|source| source.photo)
                                .unwrap_or_default();
                            self.photo_orientation = prepared
                                .selected
                                .and_then(|id| self.document.project.source(id).ok())
                                .map(|source| source.orientation)
                                .unwrap_or_default();
                            self.crop = prepared
                                .selected
                                .and_then(|id| self.document.project.source(id).ok())
                                .and_then(|source| source.crop);
                            self.crop_edit = None;
                            self.last_layout = None;
                            self.target = match self.document.project.output.tiff {
                                crate::project::TiffTarget::Srgb => OutputTarget::Srgb,
                                crate::project::TiffTarget::DisplayP3 => OutputTarget::DisplayP3,
                            };
                            if !prepared.reuse_ready {
                                self.ready = prepared.ready;
                                if !prepared.preserve_view {
                                    self.viewport.fit();
                                    self.source = self.calibration().is_none();
                                }
                            }
                            self.pick = None;
                            self.pick_view = None;
                            self.pick_return = None;
                            self.region = None;
                            self.pick_drag = None;
                            self.return_notice = None;
                            self.overwrite = None;
                            self.original.invalidate();
                            if let Some(context) = flat_context {
                                self.viewport = context.viewport;
                                self.source = context.source;
                            }
                            self.status = if self.document.project.flat_field.is_some()
                                || prepared.preserve_view
                            {
                                "Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup."
                            } else {
                                "Project ready"
                            }
                            .into();
                            if let Some(kind) = failure_kind {
                                self.resolve_failure(kind);
                            }
                        }
                        Err(error) => {
                            self.status = format!("Project: {error}");
                            self.finish_flat_edit();
                            self.source_epoch
                                .store(self.id, std::sync::atomic::Ordering::Relaxed);
                            if let Some(kind) = failure_kind {
                                self.report_failure(kind, error);
                            }
                        }
                    }
                }
                Message::FlatPreview(id, result) if id == self.id && self.flat_edit.is_some() => {
                    self.tasks.finish(TaskRequest::Reference(id));
                    self.busy = false;
                    match result {
                        Ok(preview) => {
                            self.region = self.region.or(preview.region.map(Coverage::rectangle));
                            self.ready = Some(preview.ready);
                            self.resolve_failure(FailureKind::FlatReference);
                            self.status = if self.region.is_some() {
                                "Proposed illuminated rectangle: adjust edges, then Apply."
                            } else {
                                "Illuminated area is ambiguous; draw a rectangle around the usable light."
                            }
                            .into();
                        }
                        Err(error) => {
                            self.status = error.clone();
                            self.finish_flat_edit();
                            self.report_failure(FailureKind::FlatReference, error);
                        }
                    }
                }
                Message::Saved(id, result) if self.document_pending == Some(id) => {
                    // Saving retires document work, not the authoritative developed source.
                    self.source_epoch
                        .store(self.id, std::sync::atomic::Ordering::Relaxed);
                    self.tasks.finish(TaskRequest::Document(id));
                    self.document_pending = None;
                    self.document_failure = None;
                    self.busy = false;
                    match result {
                        Ok(document) => {
                            self.document.accept_save(*document);
                            self.status = "Project saved".into();
                            self.resolve_failure(FailureKind::Save);
                            self.resume_transition = self.after_save.take();
                        }
                        Err(error) => {
                            self.status = format!("Save failed: {error}");
                            self.transition = self.after_save.take();
                            self.report_failure(FailureKind::Save, error);
                        }
                    }
                }
                Message::Exported(id, result) if self.export_pending == Some(id) => {
                    self.export_pending = None;
                    let foreground = self.tasks.finish(TaskRequest::Export(id));
                    if !foreground {
                        // The captured file still completed, but a newer operation owns busy/status.
                        if let Err(error) = result
                            && self.failure.is_none()
                        {
                            self.report_failure(FailureKind::Export, error);
                        }
                        continue;
                    }
                    self.busy = false;
                    match result {
                        Ok(path) => {
                            self.status = format!("Exported {}", path.display());
                            self.resolve_failure(FailureKind::Export);
                        }
                        Err(error) => {
                            self.status = error.clone();
                            self.report_failure(FailureKind::Export, error);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn finish_pick(&mut self) {
        if self.analysis_review.is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review();
            return;
        }
        self.analysis_area_pending = None;
        if self.flat_edit.is_some() {
            self.finish_flat_edit();
            return;
        }
        let view = self.pick_view.take();
        let source = view.map_or(self.calibration().is_none(), |view| view.source);
        let returning = self.pick_return.is_some();
        self.pick = None;
        self.region = None;
        self.pick_drag = None;
        if let Some(context) = self.pick_return.take() {
            if self
                .ready
                .as_ref()
                .is_none_or(|ready| ready.path != context.path)
            {
                let notice = self.status.clone();
                self.load(context.path);
                self.return_notice = Some(notice);
            }
            self.selected = context.selected;
            if let Some((source, target)) = context.adjustment
                && self.current_source == Some(source)
                && self.adjustment_target() != target
            {
                self.select_adjustment(target);
            }
        }
        if let Some(view) = view {
            if returning || self.viewport == view.anchored {
                self.viewport = view.viewport;
            } else if let Some((layout, pixels_per_point)) = self.last_layout {
                self.viewport
                    .reframe(layout, layout.composition, pixels_per_point);
            }
        }
        self.source = self.calibration().is_none() || source;
    }

    fn begin_pick(&mut self, kind: Pick) {
        self.finish_dust_cleanup();
        if self.pick.is_some() {
            self.finish_pick();
        }
        if self.busy
            || self.crop_edit.is_some()
            || self.auto_crop_edit.is_some()
            || self.mask_edit.is_some()
        {
            return;
        }
        self.thumbnails.cancel();
        let viewport = self.viewport;
        // Missing calibration forces source display, not an explicit inspection choice.
        let source = self.calibration().is_some() && self.source;
        self.pick = Some(kind);
        self.region = None;
        self.pick_drag = None;
        self.source = matches!(kind, Pick::Dmin | Pick::Dmax | Pick::Analysis);
        if self.source {
            self.pick_return = self.ready.as_ref().map(|ready| PickReturn {
                path: ready.path.clone(),
                selected: self.selected,
                adjustment: self
                    .current_source
                    .map(|source| (source, self.adjustment_target())),
            });
        }
        if let Some((layout, pixels_per_point)) = self.last_layout {
            let next = if kind == Pick::Analysis {
                Composition::new(
                    layout.composition.area(),
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                )
                .expect("loaded source has a valid active area")
            } else if self.source {
                Composition::new(
                    layout.composition.area(),
                    self.roll_orientation,
                    self.photo_orientation,
                )
                .expect("loaded source has a valid active area")
            } else {
                layout.composition
            };
            if next != layout.composition {
                self.viewport.reframe(layout, next, pixels_per_point);
            }
        }
        self.pick_view = Some(PickView {
            source,
            viewport,
            anchored: self.viewport,
        });
        if kind == Pick::Analysis {
            if self.analysis_review.is_some() {
                self.refresh_review_region();
            } else {
                self.request_analysis_support();
            }
        }
    }

    fn pick_roll_source(&mut self, id: SourceId) {
        let Some(index) = self
            .document
            .project
            .order
            .iter()
            .position(|candidate| *candidate == id)
        else {
            return;
        };
        let path = self.document.source_path(id).expect("selected roll source");
        let kind = self.pick;
        let context = self.pick_return.take();
        let view = self.pick_view;
        if self.ready.as_ref().is_none_or(|ready| ready.path != path) {
            self.load(path);
        }
        self.selected = index;
        self.pick = kind;
        self.pick_return = context;
        self.pick_view = view;
        self.region = None;
        self.source = true;
    }

    fn apply_pick(&mut self) {
        if self.analysis_review.is_some() {
            self.confirm_analysis_review();
            return;
        }
        if self.pick == Some(Pick::Analysis) {
            self.apply_analysis_area();
            return;
        }
        if self.pick == Some(Pick::Flat) {
            self.apply_flat_edit();
            return;
        }
        if let (Some(kind), Some(region)) = (self.pick, self.region) {
            self.thumbnails.cancel();
            self.busy = true;
            self.status = "Measuring full-resolution patch…".into();
            self.tasks.begin(Task::Measurement(self.id, kind));
            if self
                .commands
                .send(Command::Pick(
                    self.id,
                    kind,
                    region,
                    self.calibration(),
                    self.roll,
                    self.photo,
                ))
                .is_err()
            {
                self.busy = false;
                self.tasks.finish(TaskRequest::Measurement(self.id, kind));
                self.finish_pick();
                self.status = "RAW worker stopped".into();
            }
        }
    }

    fn export(&mut self, path: PathBuf, format: OutputFormat, overwrite: Overwrite) {
        if self.auto_crop_edit.is_some() {
            self.status = "Apply or cancel auto-crop before exporting".into();
            return;
        }
        if self
            .document
            .path
            .as_ref()
            .is_some_and(|project| document::same_path(project, &path))
        {
            self.status = "Export destination is the current project document".into();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.sync_document();
        if let (Some(calibration), Some(ready)) = (self.calibration(), &self.ready) {
            let grade = (|| -> Result<PhotoGradeSnapshot, Error> {
                self.validate_document_candidate(&self.document)?;
                let id = self
                    .current_source
                    .ok_or_else(|| Error::Project("no accepted source for export".into()))?;
                let source = self.document.project.source(id)?;
                source.grade_snapshot()
            })();
            let grade = match grade {
                Ok(grade) => grade,
                Err(error) => {
                    self.status = error.to_string();
                    return;
                }
            };
            let composition = Composition::with_crop(
                ready.area,
                self.roll_orientation,
                self.photo_orientation,
                self.crop,
            )
            .expect("accepted crop belongs to the loaded source");
            let request = self.request_id();
            self.export_pending = Some(request);
            self.thumbnails.cancel();
            self.busy = true;
            self.status = "Exporting full-resolution source; preview pixels are not used…".into();
            self.tasks.begin(Task::Export(request));
            if self
                .commands
                .send(Command::Export(
                    ExportIdentity {
                        source: self.id,
                        request,
                    },
                    path,
                    format,
                    calibration,
                    self.roll,
                    grade,
                    composition,
                    overwrite,
                ))
                .is_err()
            {
                self.busy = false;
                self.export_pending = None;
                self.tasks.finish(TaskRequest::Export(request));
                self.status = "RAW worker stopped".into();
                self.report_failure(FailureKind::Export, self.status.clone());
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) -> (Option<ScopeSlot>, bool) {
        use controls::{ColorControl, Parameter};
        use presentation::SectionHeader;

        let reviewing = self.analysis_review.is_some();
        let previous_orientation = (self.roll_orientation, self.photo_orientation);
        presentation::restore_expansion(ui.ctx(), self.preferences.sections.clone());
        let mut analyze = false;
        let scope_slot = self.scope_panel(ui);
        ui.add_space(8.);

        let mut next_tab = self.tab;
        presentation::stage_tabs(ui, &mut next_tab);
        if next_tab != self.tab {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
            ui.ctx().request_repaint();
        }

        if self.tab == 0 {
            analyze = self.roll_tool_shelf(ui);
        } else if self.tab == 1 {
            ui.add_enabled_ui(!reviewing, |ui| self.photo_tool_shelf(ui));
        }

        let editable = !self.busy
            && !reviewing
            && self.auto_crop_edit.is_none()
            && self.mask_edit.is_none()
            && self.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.calibration().is_some();
        let can_pick = !self.busy
            && !reviewing
            && self.auto_crop_edit.is_none()
            && self.mask_edit.is_none()
            && self.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.ready.is_some();
        let mut start_pick = None;
        if self.tab == 1 {
            // A conditional indicator must not change the following inputs' auto-ID sequence.
            ui.push_id("adjustment-target-indicator", |ui| {
                self.adjustment_target_label(ui)
            });
        }
        egui::ScrollArea::vertical()
            .id_salt(("inspector", self.tab))
            .show(ui, |ui| {
                self.auto_crop_controls(ui);
                if self.tab == 0 {
                    ui.push_id("roll", |ui| {
                        ui.add_enabled_ui(!reviewing, |ui| self.flat_controls(ui));
                        self.analysis_controls(ui);
                        let calibration_summary = match (self.dmin.is_some(), self.dmax.is_some()) {
                            (true, true) => "Dmin + Dmax measured",
                            (false, false) => "Dmin + Dmax not measured",
                            (false, true) => "Dmin not measured",
                            (true, false) => "Dmax not measured",
                        };
                        presentation::section(
                            ui,
                            "roll-calibration",
                            "Calibration",
                            SectionHeader {
                                summary: calibration_summary,
                                changed: false,
                                reset: None,
                                default_open: Some(true),
                            },
                            |ui| {
                                if controls::calibration(
                                    ui,
                                    "dmin",
                                    "Dmin (film base)",
                                    self.dmin,
                                    can_pick,
                                    "Measure an unexposed area of film base.",
                                ) {
                                    start_pick = Some(Pick::Dmin);
                                }
                                if controls::calibration(
                                    ui,
                                    "dmax",
                                    "Dmax",
                                    self.dmax,
                                    can_pick,
                                    "Measure the densest film reference.",
                                ) {
                                    start_pick = Some(Pick::Dmax);
                                }
                            },
                        );
                        ui.add_enabled_ui(editable, |ui| {
                            let inversion_changed = self.roll.gamma
                                != crate::processing::DEFAULT_GAMMA
                                || self.roll.balance != Balance::default();
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-gamma-balance",
                                "Gamma & balance",
                                SectionHeader {
                                    summary: &format!(
                                        "Gamma {:.2} · Balance {}",
                                        self.roll.gamma,
                                        if self.roll.balance == Balance::default() {
                                            "neutral"
                                        } else {
                                            "adjusted"
                                        }
                                    ),
                                    changed: inversion_changed,
                                    reset: Some(inversion_changed),
                                    default_open: Some(true),
                                },
                                |ui| {
                                    controls::parameter(ui, Parameter::Gamma, &mut self.roll.gamma);
                                    if controls::color(
                                        ui,
                                        ColorControl::GammaBalance,
                                        &mut self.roll.balance,
                                        can_pick,
                                    ) {
                                        start_pick = Some(Pick::Gamma);
                                    }
                                },
                            );
                            if reset {
                                self.roll.gamma = crate::processing::DEFAULT_GAMMA;
                                self.roll.balance = Balance::default();
                            }
                            let response_summary = match self.roll.response {
                                crate::processing::Response::Original => "Classic curve".into(),
                                crate::processing::Response::Film { toe, shoulder } => {
                                    format!("Toe {toe:.2} · Shoulder {shoulder:.4}")
                                }
                            };
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-response",
                                "Toe & shoulder",
                                SectionHeader {
                                    summary: &response_summary,
                                    changed: self.roll.response
                                        != crate::processing::Response::film(),
                                    reset: Some(
                                        !self.roll.response.is_original()
                                            && self.roll.response
                                                != crate::processing::Response::film(),
                                    ),
                                    default_open: Some(false),
                                },
                                |ui| match &mut self.roll.response {
                                    crate::processing::Response::Original => {
                                        ui.label("Classic curve · camera model 4");
                                        if presentation::TextAction::new("Use adjustable response")
                                            .help(
                                                "Start with toe 2.5 and shoulder off; keep other adjustments.",
                                            )
                                            .show(ui)
                                            .clicked()
                                        {
                                            self.roll.response =
                                                crate::processing::Response::film();
                                        }
                                    }
                                    crate::processing::Response::Film { toe, shoulder } => {
                                        controls::parameter(ui, Parameter::Toe, toe);
                                        controls::parameter(ui, Parameter::Shoulder, shoulder);
                                        ui.label("Shoulder: 0 = off");
                                    }
                                },
                            );
                            if reset {
                                self.roll.response = crate::processing::Response::film();
                            }
                            if controls::white_balance(
                                ui,
                                "roll-white-balance",
                                &mut self.roll.wb,
                                can_pick,
                            ) {
                                start_pick = Some(Pick::RollWb);
                            }
                            let (_, reset) = presentation::section(
                                ui,
                                "roll-exposure",
                                "Exposure",
                                SectionHeader {
                                    summary: &format!("{:+.2} EV", self.roll.exposure),
                                    changed: self.roll.exposure != 0.,
                                    reset: Some(self.roll.exposure != 0.),
                                    default_open: Some(false),
                                },
                                |ui| {
                                    controls::parameter(
                                        ui,
                                        Parameter::Exposure,
                                        &mut self.roll.exposure,
                                    );
                                },
                            );
                            if reset {
                                self.roll.exposure = 0.;
                            }
                        });
                    });
                } else if self.tab == 1 {
                    ui.add_enabled_ui(!reviewing, |ui| {
                        self.crop_controls(ui);
                        self.dust_controls(ui);
                    });
                    if self.adjustment_controls(ui, editable, can_pick) {
                        start_pick = Some(Pick::Wb);
                    }
                } else {
                    ui.add_enabled_ui(
                        !reviewing
                            && self.crop_edit.is_none()
                            && self.auto_crop_edit.is_none()
                            && self.flat_edit.is_none()
                            && self.mask_edit.is_none(),
                        |ui| {
                            let changed = self.target != OutputTarget::DisplayP3;
                            let (_, reset) = presentation::section(
                                ui,
                                "rendering-target",
                                "Target",
                                SectionHeader {
                                    summary: if self.target == OutputTarget::DisplayP3 {
                                        "Display P3 · 100 nit"
                                    } else {
                                        "sRGB · 100 nit"
                                    },
                                    changed,
                                    reset: Some(changed),
                                    default_open: Some(true),
                                },
                                |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label("Target");
                                        egui::ComboBox::from_id_salt("rendering-target")
                                            .width(150.)
                                            .selected_text(
                                                if self.target == OutputTarget::DisplayP3 {
                                                    "Display P3"
                                                } else {
                                                    "sRGB"
                                                },
                                            )
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(
                                                    &mut self.target,
                                                    OutputTarget::DisplayP3,
                                                    "Display P3",
                                                );
                                                ui.selectable_value(
                                                    &mut self.target,
                                                    OutputTarget::Srgb,
                                                    "sRGB",
                                                );
                                            });
                                    });
                                    ui.small("Preview + TIFF · 100 nit").on_hover_text(
                                        "Choose the output colour space, not the monitor profile. EXR is unaffected.",
                                    );
                                },
                            );
                            if reset {
                                self.target = OutputTarget::DisplayP3;
                            }
                        },
                    );
                }
            });

        self.preferences.sections = presentation::collect_expansion(ui.ctx());
        if !reviewing {
            self.preferences.scopes_visible = self.scopes.visible;
        }
        self.preferences.scope_height_ratio = self.scopes.height_ratio;
        if previous_orientation != (self.roll_orientation, self.photo_orientation) {
            self.viewport.fit();
        }
        if let Some(kind) = start_pick {
            self.begin_pick(kind);
        }

        // Render outgoing numeric controls once to commit focus loss before switching stages.
        self.tab = next_tab;
        (scope_slot, analyze)
    }

    fn export_dialog(&mut self, format: OutputFormat) {
        let Some(ready) = &self.ready else {
            return;
        };
        let extension = if matches!(format, OutputFormat::Tiff(_)) {
            "tif"
        } else {
            "exr"
        };
        let filename = format!(
            "{}.{}",
            ready.path.file_stem().unwrap_or_default().to_string_lossy(),
            extension
        );
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name(filename)
            .add_filter("Output", &[extension])
            .save_file()
        {
            if path.exists() {
                self.overwrite = Some((path, format));
            } else {
                self.export(path, format, Overwrite::Refuse);
            }
        }
    }
}

impl host::App for Editor {
    fn photo_fullscreen(&self) -> bool {
        self.fullscreen_requested()
    }

    fn photo_fullscreen_changed(&mut self, active: bool) {
        self.fullscreen_changed(active);
    }

    fn submission_gate(&self) -> &crate::platform::SubmissionGate {
        &self.submissions
    }

    fn processing_revision(&self) -> Option<(u64, u64)> {
        Some((
            self.id,
            self.working
                .revision()
                .unwrap_or_else(|| self.original.revision()),
        ))
    }

    fn on_exit(&mut self) {
        self.working.clear();
        if let Some(developments) = &self.developments {
            developments.close();
        }
        self.document.cache().close();
        self.thumbnails.cancel();
        if let Some(store) = &mut self.preference_store
            && let Err(error) = store.save(&self.preferences)
        {
            eprintln!("UI preferences not saved: {error}");
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        if self.initialize_ui(ui) {
            return;
        }
        // egui popups close on Escape without consuming it; preserve their input ownership.
        let dialog_escape_available = !egui::Popup::is_any_open(ui.ctx());
        self.receive_frame();
        let fullscreen_was_active = self.fullscreen.is_some();
        self.update_fullscreen(ui.ctx());
        let available = self.file_actions_available();
        let mut file_action = self.file_shortcuts(ui, available);
        self.frame_shortcuts(
            ui,
            &mut file_action,
            fullscreen_was_active,
            dialog_escape_available,
        );

        let previous_roll = self.roll;
        let previous_photo = self.photo;
        let was_dirty = self.document.dirty();
        self.disable_blocked_workspace(ui);
        let export_action = self.workspace_chrome(ui, available, &mut file_action);
        let (scope_slot, analyze) = self.workspace_controls(ui);
        self.validate_frame_edits(previous_roll, previous_photo);
        if self.analysis_review.is_some() && self.analysis_pending.is_none() {
            self.stash_review_region();
        }

        let zoom_action = self.viewer_bar(ui);
        let fullscreen = self.fullscreen.is_some();
        presentation::viewer_surface(ui, fullscreen, |ui| {
            self.viewer_canvas(ui, zoom_action, fullscreen);
        });
        let text_focused = ui.ctx().text_edit_focused();
        ui.ctx()
            .data_mut(|data| data.insert_temp(ui.id().with("editor-text-focus"), text_focused));
        self.paint_scope(scope_slot);
        self.sync_document();
        if self.document.dirty() != was_dirty {
            ui.ctx().request_repaint();
        }

        // Deferred actions must see validated edits and committed numeric focus loss.
        self.dispatch_frame_actions(ui, file_action, export_action, analyze);
        self.frame_dialogs(ui, dialog_escape_available);
    }
}
