use crate::{
    Error,
    color::Balance,
    coverage::Coverage,
    crop::Crop,
    export::{self, ExrPrecision, Overwrite},
    geometry::{Composition, PhotoOrientation, RollOrientation},
    gpu::ProcessingParameters,
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

#[path = "editor_controls.rs"]
mod controls;
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
#[path = "editor_thumbnails.rs"]
mod thumbnails;
use thumbnails::Thumbnails;
#[path = "editor_selected_preview.rs"]
mod selected_preview;
use selected_preview::{SelectionPaint, SelectionTiming};
#[path = "editor_scopes.rs"]
mod scopes;
use scopes::{Scopes, Slot as ScopeSlot};
#[path = "editor_analysis.rs"]
mod analysis;
#[path = "editor_analysis_smoke.rs"]
mod analysis_smoke;
pub use analysis_smoke::AnalysisSmoke;
#[path = "editor_flat_field.rs"]
mod flat;
use flat::{Correction, FlatEdit, FlatPreview};
#[path = "editor_crop.rs"]
mod crop_editor;
use crop_editor::CropEdit;
#[path = "editor_memory.rs"]
mod memory;
#[path = "editor_navigation.rs"]
mod navigation;
#[path = "editor_smoke.rs"]
mod smoke;
#[path = "editor_viewer.rs"]
mod viewer_ui;
pub use smoke::SmokeEditor;
#[path = "editor_flat_field_smoke.rs"]
mod flat_smoke;
pub use flat_smoke::FlatFieldSmoke;
#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;

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
}
#[derive(Clone, Copy)]
struct PickView {
    source: bool,
    viewport: Viewport,
    anchored: Viewport,
}
struct Ready {
    thumbnail: Option<(thumbnails::FileKey, Arc<thumbnails::Proxy>)>,
    path: PathBuf,
    area: Rect,
    analysis_support: Option<Rect>,
    floor: f64,
    images: [Arc<GpuImage>; 2],
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
enum Command {
    Analyze(u64, Box<analysis::Request>),
    Refine(u64, u64, ProcessingParameters, OutputTarget, ViewRequest),
    Load(
        u64,
        PathBuf,
        Option<[f64; 4]>,
        Option<Box<Source>>,
        Option<Arc<Correction>>,
        Option<crate::project::CameraBasis>,
    ),
    FlatPreview(
        u64,
        PathBuf,
        Option<[f64; 4]>,
        Option<Box<Source>>,
        Option<crate::project::CameraBasis>,
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
        u64,
        PathBuf,
        OutputFormat,
        Calibration,
        RollSettings,
        PhotoSettings,
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
    AnalysisProgress(u64, usize, usize),
    Analyzed(u64, Result<Option<Box<analysis::Outcome>>, String>),
    Refined(u64, Result<Option<Arc<GpuImage>>, String>),
    Ready(u64, Result<Box<Ready>, String>),
    Picked(u64, Pick, Result<Measurement, String>),
    Exported(u64, Result<PathBuf, String>),
    Prepared(u64, Result<Box<Prepared>, String>),
    Saved(u64, Result<Box<Document>, String>),
    FlatPreview(u64, Result<Box<FlatPreview>, String>),
}

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
    let started = std::time::Instant::now();
    let thumbnail_file = thumbnails::FileKey::new(&path);
    let (record, raw) = document::open_source(&path)?;
    let registration_seconds = started.elapsed().as_secs_f64();
    if let Some(basis) = basis {
        basis.require_source(&record.development)?;
    }
    if let Some(expected) = expected {
        document::verify_source(expected, &record)?;
    }
    let tick = std::time::Instant::now();
    let mut scan = Scan::from_raw(raw, wb)?;
    let development_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    document::verify_scan(&record, &scan)?;
    let verification_seconds = tick.elapsed().as_secs_f64();
    if let Some(correction) = correction {
        correction.settings.require_source(&record.development)?;
        scan.set_flat_field(Some(correction.settings.map.clone()))?;
    }
    let tick = std::time::Instant::now();
    let (width, height, pixels) = scan.preview_source(1024)?;
    let thumbnail = Some((
        thumbnail_file,
        Arc::new(thumbnails::Proxy::from_scan(&scan)?),
    ));
    let proxy_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let renderer = renderers.get(device, queue)?;
    let renderer_seconds = tick.elapsed().as_secs_f64();
    let tick = std::time::Instant::now();
    let make_image = |target| -> Result<Arc<GpuImage>, Error> {
        let mut image = pollster::block_on(GpuImage::new_shared(
            &renderer,
            device,
            width,
            height,
            &pixels,
            scan.camera_to_ap1,
            target,
        ))?;
        if let Some(correction) = correction {
            let a = scan.area;
            image.set_flat_field(
                Some(correction.gpu.clone()),
                crate::gpu::SourceGeometry::new([a.x, a.y, a.width, a.height], [width, height])?,
            )?;
        }
        Ok(Arc::new(image))
    };
    let images = [
        make_image(OutputTarget::Srgb)?,
        make_image(OutputTarget::DisplayP3)?,
    ];
    println!(
        "selected_source seconds={:.6} registration_seconds={registration_seconds:.6} development_seconds={development_seconds:.6} verification_seconds={verification_seconds:.6} proxy_seconds={proxy_seconds:.6} renderer_seconds={renderer_seconds:.6} images_seconds={:.6}",
        started.elapsed().as_secs_f64(),
        tick.elapsed().as_secs_f64()
    );
    let analysis_support = scan.propose_flat_field_area()?;
    let ready = Ready {
        thumbnail,
        path,
        area: scan.area,
        analysis_support,
        floor: scan.input_floor,
        images,
        record,
        development: Development::new(crate::raw::version(), scan.fixed_wb),
    };
    Ok((ready, scan))
}
#[allow(clippy::too_many_arguments)]
fn prepare_document(
    renderers: &crate::renderer::RendererCache,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &mut Option<(u64, Scan)>,
    id: u64,
    work: DocumentWork,
    previous_correction: Option<&Arc<Correction>>,
) -> Result<Prepared, Error> {
    let preserve_view = matches!(work, DocumentWork::Correction(..));
    let selected_override = match &work {
        DocumentWork::Correction(_, selected) => Some(*selected),
        _ => None,
    };
    let (document, reuse) = match work {
        DocumentWork::Import(mut document, paths, reuse) => {
            document.import(&paths)?;
            (document, reuse)
        }
        DocumentWork::Open(path) => (Document::open(&path)?, None),
        DocumentWork::New => (Document::default(), None),
        DocumentWork::Correction(document, _) => (document, None),
        DocumentWork::Save(..) => unreachable!("save does not prepare a RAW"),
    };
    document.validate()?;
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
        let (ready, scan) = prepare_source(
            renderers,
            device,
            queue,
            path,
            wb,
            Some(document.project.source(selected)?),
            correction.as_ref(),
            document.project.camera_basis.as_ref(),
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

fn backend(
    renderers: Arc<crate::renderer::RendererCache>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    commands: mpsc::Receiver<Command>,
    messages: mpsc::Sender<Message>,
    generation: Arc<std::sync::atomic::AtomicU64>,
    analysis_generation: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut source: Option<(u64, Scan)> = None;
    let mut correction: Option<Arc<Correction>> = None;
    let mut flat_preview: Option<(u64, Scan, Source)> = None;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Analyze(id, request) => {
                let result = if source
                    .as_ref()
                    .is_some_and(|(loaded, _)| *loaded == request.loaded)
                {
                    analysis::run(
                        &device,
                        &request,
                        &analysis_generation,
                        correction.as_ref(),
                        |done, total| {
                            let _ = messages.send(Message::AnalysisProgress(id, done, total));
                        },
                    )
                    .map(|result| result.map(Box::new))
                    .map_err(|e| e.to_string())
                } else {
                    Err("source changed before analysis".into())
                };
                if messages.send(Message::Analyzed(id, result)).is_err() {
                    break;
                }
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
                                &queue,
                                context,
                                parameters,
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
            Command::Load(id, path, wb, expected, field, basis) => {
                let result = prepare_source(
                    &renderers,
                    &device,
                    &queue,
                    path,
                    wb,
                    expected.as_deref(),
                    field.as_ref(),
                    basis.as_ref(),
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
            Command::FlatPreview(id, path, wb, expected, basis) => {
                let result = prepare_source(
                    &renderers,
                    &device,
                    &queue,
                    path,
                    wb,
                    expected.as_deref(),
                    None,
                    basis.as_ref(),
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
                    let prepared = prepare_document(
                        &renderers,
                        &device,
                        &queue,
                        &mut source,
                        id,
                        DocumentWork::Correction(*document, selected),
                        correction.as_ref(),
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
                            .save(&path, policy)
                            .map(Box::new)
                            .map_err(|error| error.to_string()),
                    ),
                    work => Message::Prepared(
                        id,
                        prepare_document(
                            &renderers,
                            &device,
                            &queue,
                            &mut source,
                            id,
                            work,
                            correction.as_ref(),
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
            Command::Export(id, path, format, calibration, roll, photo, composition, overwrite) => {
                let result = (|| -> Result<PathBuf, Error> {
                    let (loaded, scan) = source
                        .as_ref()
                        .ok_or(Error::UnusableSample("no developed source"))?;
                    if id != *loaded {
                        return Err(Error::UnusableSample("source changed before export"));
                    }
                    let pipeline = crate::processing::Pipeline::new(
                        scan.processing_calibration(calibration)?,
                        roll,
                        photo,
                        scan.camera_to_ap1,
                    )?;
                    if composition.area() != scan.area {
                        return Err(Error::InvalidParameter(
                            "export composition belongs to another source area",
                        ));
                    }
                    let pixel = |x, y| pipeline.apply(scan.pixel(x, y)?).map(|p| p.rgb);
                    match format {
                        OutputFormat::Tiff(target) => export::tiff_composed(
                            &path,
                            composition,
                            target,
                            overwrite,
                            roll.response,
                            pixel,
                        )?,
                        OutputFormat::Exr(precision) => export::exr_composed(
                            &path,
                            composition,
                            precision,
                            overwrite,
                            roll.response,
                            pixel,
                        )?,
                    }
                    Ok(path)
                })()
                .map_err(|e| e.to_string());
                if messages.send(Message::Exported(id, result)).is_err() {
                    break;
                }
            }
        }
    }
}

pub struct Editor {
    started: bool,
    analysis_generation: Arc<std::sync::atomic::AtomicU64>,
    analysis_pending: Option<analysis::Pending>,
    analysis_confirmation: Option<analysis::Request>,
    thumbnails: Thumbnails,
    scopes: Scopes,
    generation: Arc<std::sync::atomic::AtomicU64>,
    signature: Option<(
        [u8; ProcessingParameters::BYTE_LEN],
        OutputTarget,
        Option<ViewRequest>,
        Composition,
    )>,
    settled: Option<(u64, ViewRequest, Arc<GpuImage>)>,
    refining: Option<(u64, ViewRequest)>,
    viewport: Viewport,
    last_edit: std::time::Instant,
    refinement_error: Option<u64>,
    commands: mpsc::Sender<Command>,
    messages: mpsc::Receiver<Message>,
    id: u64,
    ready: Option<Ready>,
    pending_preview: Option<thumbnails::SelectedPreview>,
    selection_timing: Option<Arc<SelectionTiming>>,
    busy: bool,
    status: String,
    return_notice: Option<String>,
    dmin: Option<DVec3>,
    dmax: Option<DVec3>,
    floor: f64,
    roll: RollSettings,
    photo: PhotoSettings,
    roll_orientation: RollOrientation,
    photo_orientation: PhotoOrientation,
    crop: Option<Crop>,
    crop_edit: Option<CropEdit>,
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
    transition: Option<Transition>,
    after_save: Option<Transition>,
    resume_transition: Option<Transition>,
    project_overwrite: Option<(PathBuf, Option<Transition>)>,
    allow_close: bool,
    selected: usize,
    overwrite: Option<(PathBuf, OutputFormat)>,
    correction: Option<Arc<Correction>>,
    flat_edit: Option<FlatEdit>,
}
impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, initial: Option<PathBuf>) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        renderers.prewarm(device.clone(), queue.clone());
        let thumbnails = Thumbnails::new(device.clone(), queue.clone(), renderers.clone());
        let scopes = Scopes::new(device);
        let device = device.clone();
        let queue = queue.clone();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_generation = generation.clone();
        let analysis_generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_analysis_generation = analysis_generation.clone();
        std::thread::spawn(move || {
            backend(
                renderers,
                device,
                queue,
                receiver,
                send,
                worker_generation,
                worker_analysis_generation,
            )
        });
        let mut editor = Self::state(commands, messages, generation);
        editor.analysis_generation = analysis_generation;
        editor.thumbnails = thumbnails;
        editor.scopes = scopes;
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
        Self {
            analysis_generation: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            analysis_pending: None,
            analysis_confirmation: None,
            thumbnails: Thumbnails::default(),
            scopes: Scopes::default(),
            generation,
            signature: None,
            settled: None,
            refining: None,
            viewport: Viewport::default(),
            last_edit: std::time::Instant::now(),
            refinement_error: None,
            started: false,
            commands,
            messages,
            id: 0,
            ready: None,
            pending_preview: None,
            selection_timing: None,
            busy: false,
            status: "Add a RAW to begin".into(),
            return_notice: None,
            dmin: None,
            dmax: None,
            floor: 0.,
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),
            roll_orientation: RollOrientation::default(),
            photo_orientation: PhotoOrientation::default(),
            crop: None,
            crop_edit: None,
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
            document: Document::default(),
            current_source: None,
            next_request: 0,
            document_pending: None,
            transition: None,
            after_save: None,
            resume_transition: None,
            project_overwrite: None,
            allow_close: false,
            selected: 0,
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
                if self.document_pending.is_none() || self.new_project_pending {
                    self.preferences
                        .sections
                        .insert("roll-calibration".into(), true);
                }
            }
            Err(error) => eprintln!("UI preferences not loaded (file left unchanged): {error}"),
        }
    }
    fn apply_preferences(&mut self, preferences: preferences::Preferences) {
        self.scopes.visible = preferences.scopes_visible;
        self.scopes.height_ratio = preferences.scope_height_ratio;
        self.preferences = preferences;
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
                .seed(source, &ready.development, file, proxy);
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
        self.analysis_confirmation = None;
        self.pending_preview = None;
        self.selection_timing = None;
        if self.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        let id = self.request_id();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.document_pending = Some(id);
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
            self.busy = false;
            self.status = "RAW worker stopped".into();
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
        self.analysis_confirmation = None;
        let started = std::time::Instant::now();
        let selecting_photo = persistent && self.pick.is_none() && self.flat_edit.is_none();
        self.seed_thumbnail();
        self.pending_preview = None;
        self.selection_timing = None;
        if self.analysis_pending.take().is_some() {
            self.cancel_analysis();
        }
        self.thumbnails.cancel();
        self.sync_document();
        self.current_source = self.document.find_source(&path);
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
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.signature = None;
        self.settled = None;
        self.viewport.fit();
        self.overwrite = None;
        self.return_notice = None;
        self.pick_view = None;
        self.pick_return = None;
        self.id = self.request_id();
        self.ready = None;
        self.selection_timing =
            selecting_photo.then(|| Arc::new(SelectionTiming::new(self.id, started)));
        self.region = None;
        self.pick = None;
        self.source = self.calibration().is_none();
        self.busy = true;
        self.status = "Developing full-resolution RAW…".into();
        if self
            .commands
            .send(Command::Load(
                self.id,
                path,
                wb,
                expected,
                self.correction.clone(),
                self.document.project.camera_basis.clone(),
            ))
            .is_err()
        {
            self.busy = false;
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
                Message::AnalysisProgress(id, done, total) => {
                    if let Some(pending) = &mut self.analysis_pending
                        && pending.id == id
                    {
                        pending.progress = (done, total);
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
                Message::Refined(revision, result) => {
                    let view = self
                        .refining
                        .filter(|(running, _)| *running == revision)
                        .map(|(_, view)| view);
                    if view.is_some() {
                        self.refining = None;
                    }
                    if self.generation.load(std::sync::atomic::Ordering::Relaxed) == revision {
                        match result {
                            Ok(Some(image)) => {
                                self.settled = Some((revision, view.unwrap(), image));
                            }
                            Ok(None) => {}
                            Err(error) => {
                                self.refinement_error = Some(revision);
                                self.status = format!("Settled preview failed: {error}");
                            }
                        }
                    }
                }
                Message::Ready(id, result) if id == self.id && self.document_pending.is_none() => {
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
                                        self.positive_parameters(cal, self.roll, self.photo)
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
                                && let Err(error) = self.positive_parameters(cal, roll, photo)
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
                    let new_project = std::mem::take(&mut self.new_project_pending);
                    self.document_pending = None;
                    self.busy = false;
                    match result {
                        Ok(prepared) => {
                            let flat_context = self.flat_edit.take();
                            self.correction = prepared.correction;
                            self.document = prepared.document;
                            if new_project {
                                self.tab = 0;
                                self.preferences
                                    .sections
                                    .insert("roll-calibration".into(), true);
                            }
                            self.id = id;
                            self.current_source = prepared.selected;
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
                            self.signature = None;
                            self.settled = None;
                            self.refining = None;
                            self.refinement_error = None;
                            if let Some(context) = flat_context {
                                self.viewport = context.viewport;
                                self.source = context.source;
                            }
                            self.status = if self.document.project.flat_field.is_some() || prepared.preserve_view {"Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup."} else {"Project ready"}.into();
                        }
                        Err(error) => {
                            self.status = format!("Project: {error}");
                            self.finish_flat_edit();
                        }
                    }
                }
                Message::FlatPreview(id, result) if id == self.id && self.flat_edit.is_some() => {
                    self.busy = false;
                    match result {
                        Ok(preview) => {
                            self.region = self.region.or(preview.region.map(Coverage::rectangle));
                            self.ready = Some(preview.ready);
                            self.status=if self.region.is_some(){"Proposed illuminated rectangle: adjust edges, then Apply."}else{"Illuminated area is ambiguous; draw a rectangle around the usable light."}.into();
                        }
                        Err(error) => {
                            self.status = error;
                            self.finish_flat_edit();
                        }
                    }
                }
                Message::Saved(id, result) if self.document_pending == Some(id) => {
                    self.document_pending = None;
                    self.busy = false;
                    match result {
                        Ok(document) => {
                            self.document = *document;
                            self.status = "Project saved".into();
                            self.resume_transition = self.after_save.take();
                        }
                        Err(error) => {
                            self.status = format!("Save failed: {error}");
                            self.transition = self.after_save.take();
                        }
                    }
                }
                Message::Exported(id, result)
                    if id == self.id && self.document_pending.is_none() =>
                {
                    self.busy = false;
                    self.status = match result {
                        Ok(path) => format!("Exported {}", path.display()),
                        Err(error) => error,
                    };
                }
                _ => {}
            }
        }
    }
    fn finish_pick(&mut self) {
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
        }
        if let Some(view) = view {
            if returning || self.viewport == view.anchored {
                self.viewport = view.viewport;
            } else if let Some((layout, pixels_per_point)) = self.last_layout {
                self.viewport
                    .reframe(layout, layout.composition, pixels_per_point);
            }
        }
        self.source = source;
    }
    fn begin_pick(&mut self, kind: Pick) {
        if self.pick.is_some() {
            self.finish_pick();
        }
        if self.busy || self.crop_edit.is_some() {
            return;
        }
        self.thumbnails.cancel();
        let viewport = self.viewport;
        let source = self.calibration().is_none() || self.source;
        self.pick = Some(kind);
        self.region = None;
        self.pick_drag = None;
        self.source = matches!(kind, Pick::Dmin | Pick::Dmax | Pick::Analysis);
        if self.source {
            self.pick_return = self.ready.as_ref().map(|ready| PickReturn {
                path: ready.path.clone(),
                selected: self.selected,
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
                self.finish_pick();
                self.status = "RAW worker stopped".into();
            }
        }
    }
    fn export(&mut self, path: PathBuf, format: OutputFormat, overwrite: Overwrite) {
        if self
            .document
            .path
            .as_ref()
            .is_some_and(|project| document::same_path(project, &path))
        {
            self.status = "Export destination is the current project document".into();
            return;
        }
        if let (Some(calibration), Some(ready)) = (self.calibration(), &self.ready) {
            let composition = Composition::with_crop(
                ready.area,
                self.roll_orientation,
                self.photo_orientation,
                self.crop,
            )
            .expect("accepted crop belongs to the loaded source");
            self.thumbnails.cancel();
            self.busy = true;
            self.status = "Exporting full-resolution source; preview pixels are not used…".into();
            if self
                .commands
                .send(Command::Export(
                    self.id,
                    path,
                    format,
                    calibration,
                    self.roll,
                    self.photo,
                    composition,
                    overwrite,
                ))
                .is_err()
            {
                self.busy = false;
                self.status = "RAW worker stopped".into();
            }
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui) -> (Option<ScopeSlot>, bool) {
        use controls::{ColorControl, Parameter};
        use icons::Icon;
        use presentation::SectionHeader;
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
        let editable = !self.busy
            && self.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.calibration().is_some();
        let can_pick = !self.busy
            && self.crop_edit.is_none()
            && self.flat_edit.is_none()
            && self.ready.is_some();
        let mut start_pick = None;
        egui::ScrollArea::vertical().id_salt(("inspector", self.tab)).show(ui, |ui| {
            if self.tab == 0 {
                let geometry_editable = !self.busy
                    && self.crop_edit.is_none()
                    && self.ready.is_some()
                    && self.pick.is_none();
                ui.push_id("roll", |ui| {
                    ui.add_enabled_ui(geometry_editable, |ui| {
                        let changed = !self.roll_orientation.is_identity();
                        let (_, reset) = presentation::section(ui, "roll-orientation", "Scan orientation", SectionHeader { summary: "Shared scanning baseline", changed, reset: Some(changed), default_open: Some(false) }, |ui| {
                            ui.horizontal(|ui| {
                                if icons::button(ui, Icon::TurnLeft, true, "Turn left", "Turn the shared scan baseline 90° left.").clicked() { self.roll_orientation.turn_counterclockwise(); }
                                if icons::button(ui, Icon::TurnRight, true, "Turn right", "Turn the shared scan baseline 90° right.").clicked() { self.roll_orientation.turn_clockwise(); }
                                if icons::button(ui, Icon::FlipHorizontal, true, "Flip horizontal", "Flip the shared scan baseline left/right; photo exceptions apply afterward.").clicked() { self.roll_orientation.horizontal ^= true; }
                                if icons::button(ui, Icon::FlipVertical, true, "Flip vertical", "Flip the shared scan baseline top/bottom; photo exceptions apply afterward.").clicked() { self.roll_orientation.vertical ^= true; }
                            });
                        });
                        if reset { self.roll_orientation = RollOrientation::default(); }
                    });
                    self.flat_controls(ui);
                    analyze = self.analysis_controls(ui);
                    let calibration_summary = match (self.dmin.is_some(), self.dmax.is_some()) {
                        (true, true) => "Dmin + Dmax measured",
                        (false, false) => "Dmin + Dmax not measured",
                        (false, true) => "Dmin not measured",
                        (true, false) => "Dmax not measured",
                    };
                    presentation::section(ui, "roll-calibration", "Calibration", SectionHeader { summary: calibration_summary, changed: false, reset: None, default_open: Some(true) }, |ui| {
                        if controls::calibration(
                            ui, "dmin", "Dmin (film base)", self.dmin, can_pick,
                            "Minimum-density film base. Linear native-camera RGB, not optical-density numbers.",
                        ) {
                            start_pick = Some(Pick::Dmin);
                        }
                        if controls::calibration(
                            ui, "dmax", "Dmax", self.dmax, can_pick,
                            "Maximum-density film reference. Linear native-camera RGB, not optical-density numbers.",
                        ) {
                            start_pick = Some(Pick::Dmax);
                        }
                    });
                    ui.add_enabled_ui(editable, |ui| {
                        let inversion_changed = self.roll.gamma != crate::processing::DEFAULT_GAMMA
                            || self.roll.balance != Balance::default();
                        let (_, reset) = presentation::section(
                            ui, "roll-gamma-balance", "Gamma & balance",
                            SectionHeader {
                                summary: &format!("Gamma {:.2} · Balance {}", self.roll.gamma, if self.roll.balance == Balance::default() { "neutral" } else { "adjusted" }),
                                changed: inversion_changed, reset: Some(inversion_changed), default_open: Some(true),
                            },
                            |ui| {
                                controls::parameter(ui, Parameter::Gamma, &mut self.roll.gamma);
                                if controls::color(
                                    ui, ColorControl::GammaBalance, &mut self.roll.balance, can_pick,
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
                            crate::processing::Response::Film { toe, shoulder } => format!("Toe {toe:.2} · Shoulder {shoulder:.4}"),
                        };
                        let (_, reset) = presentation::section(
                            ui, "roll-response", "Toe & shoulder", SectionHeader {
                                summary: &response_summary,
                                changed: self.roll.response != crate::processing::Response::film(),
                                reset: Some(!self.roll.response.is_original() && self.roll.response != crate::processing::Response::film()),
                                default_open: Some(false),
                            },
                            |ui| {
                                match &mut self.roll.response {
                                    crate::processing::Response::Original => {
                                        ui.label("Classic curve · camera model 4");
                                        if presentation::TextAction::new("Use adjustable response").help(
                                            "Explicitly use toe 2.5 with shoulder off. Calibration, gamma, balance and exposure are kept; this changes the response, not RAW development or ACES.",
                                        ).show(ui).clicked() {
                                            self.roll.response = crate::processing::Response::film();
                                        }
                                    }
                                    crate::processing::Response::Film { toe, shoulder } => {
                                        controls::parameter(ui, Parameter::Toe, toe);
                                        controls::parameter(ui, Parameter::Shoulder, shoulder);
                                        ui.label("Shoulder: 0 = off").on_hover_text(
                                            "Settings with an inverse pole in the scan-floor domain or unsupported GPU precision/range are rejected. Above-base values use signed-root numerical continuation, not physical film exposure.",
                                        );
                                    }
                                }
                            },
                        );
                        if reset { self.roll.response = crate::processing::Response::film(); }
                        if controls::white_balance(ui, "roll-white-balance", &mut self.roll.wb, can_pick) {
                            start_pick = Some(Pick::RollWb);
                        }
                        let (_, reset) = presentation::section(
                            ui, "roll-exposure", "Exposure", SectionHeader { summary: &format!("{:+.2} EV", self.roll.exposure), changed: self.roll.exposure != 0., reset: Some(self.roll.exposure != 0.), default_open: Some(false) },
                            |ui| {
                                controls::parameter(ui, Parameter::Exposure, &mut self.roll.exposure);
                            },
                        );
                        if reset {
                            self.roll.exposure = 0.;
                        }
                    });
                });
            } else if self.tab == 1 {
                self.composition_controls(ui);
                ui.push_id("photo", |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        if controls::color(
                            ui, ColorControl::WhiteBalance, &mut self.photo.wb, can_pick,
                        ) {
                            start_pick = Some(Pick::Wb);
                        }
                        let (_, reset) = presentation::section(
                            ui, "photo-exposure", "Exposure", SectionHeader { summary: &format!("{:+.2} EV", self.photo.exposure), changed: self.photo.exposure != 0., reset: Some(self.photo.exposure != 0.), default_open: Some(true) },
                            |ui| {
                                controls::parameter(ui, Parameter::Exposure, &mut self.photo.exposure);
                            },
                        );
                        if reset {
                            self.photo.exposure = 0.;
                        }
                        let defaults = PhotoSettings::default();
                        let changed = self.photo.contrast != defaults.contrast
                            || self.photo.pivot != defaults.pivot
                            || self.photo.contrast_colour_mix != defaults.contrast_colour_mix;
                        let (_, reset) = presentation::section(ui, "photo-contrast", "Contrast", SectionHeader {
                            summary: &format!("{:.2} · Pivot {:.3} · Preserve {:.2}", self.photo.contrast, self.photo.pivot, self.photo.contrast_colour_mix),
                            changed, reset: Some(changed), default_open: Some(true),
                        }, |ui| {
                            controls::parameter(ui, Parameter::Contrast, &mut self.photo.contrast);
                            controls::parameter(ui, Parameter::Pivot, &mut self.photo.pivot);
                            ui.small("Colour behavior");
                            controls::parameter(ui, Parameter::ColourMix, &mut self.photo.contrast_colour_mix);
                        });
                        if reset {
                            self.photo.contrast = defaults.contrast;
                            self.photo.pivot = defaults.pivot;
                            self.photo.contrast_colour_mix = defaults.contrast_colour_mix;
                        }
                        let changed = self.photo.saturation != defaults.saturation
                            || self.photo.saturation_density != defaults.saturation_density;
                        let (_, reset) = presentation::section(ui, "photo-saturation", "Saturation", SectionHeader {
                            summary: &format!("{:.2} · Density {:.2}", self.photo.saturation, self.photo.saturation_density),
                            changed, reset: Some(changed), default_open: Some(true),
                        }, |ui| {
                            controls::parameter(ui, Parameter::Saturation, &mut self.photo.saturation);
                            ui.small("Colour behavior");
                            controls::parameter(ui, Parameter::Density, &mut self.photo.saturation_density);
                        });
                        if reset {
                            self.photo.saturation = defaults.saturation;
                            self.photo.saturation_density = defaults.saturation_density;
                        }
                    });
                });
            } else {
                ui.add_enabled_ui(self.crop_edit.is_none() && self.flat_edit.is_none(), |ui| {
                    let changed = self.target != OutputTarget::DisplayP3;
                    let (_, reset) = presentation::section(ui, "rendering-target", "Target", SectionHeader {
                        summary: if self.target == OutputTarget::DisplayP3 { "Display P3 · 100 nit" } else { "sRGB · 100 nit" },
                        changed, reset: Some(changed), default_open: Some(true),
                    }, |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Target");
                            egui::ComboBox::from_id_salt("rendering-target")
                                .width(150.)
                                .selected_text(if self.target == OutputTarget::DisplayP3 { "Display P3" } else { "sRGB" })
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.target, OutputTarget::DisplayP3, "Display P3");
                                    ui.selectable_value(&mut self.target, OutputTarget::Srgb, "sRGB");
                                });
                        });
                        ui.small("Preview + TIFF · 100 nit").on_hover_text(
                            "Shared ACES 2 SDR target for preview, thumbnails, scopes and TIFF encoding/ICC. Not the monitor profile: presentation is tagged extended sRGB for OS monitor management. EXR remains scene-linear AP0 and bypasses this target.",
                        );
                    });
                    if reset { self.target = OutputTarget::DisplayP3; }
                });
            }
        });
        self.preferences.sections = presentation::collect_expansion(ui.ctx());
        self.preferences.scopes_visible = self.scopes.visible;
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
    fn on_exit(&mut self) {
        if let Some(store) = &mut self.preference_store
            && let Err(error) = store.save(&self.preferences)
        {
            eprintln!("UI preferences not saved: {error}");
        }
    }
    fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.started {
            self.started = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1100., 800.)));
        }
        self.receive();
        self.seed_thumbnail();
        self.thumbnails.receive(&self.document);
        self.validate_pending_preview();
        let available = !self.busy
            && self.pick.is_none()
            && self.crop_edit.is_none()
            && self.overwrite.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.resume_transition.is_none()
            && self.analysis_confirmation.is_none();
        let mut file_action = None;
        if !self.allow_close && ui.input(|input| input.viewport().close_requested()) {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if available {
                file_action = Some(FileAction::Close);
            } else if self.transition.is_none() && self.project_overwrite.is_none() {
                self.status =
                    "Finish the current operation or cancel the pick before closing".into();
            }
        }
        if self.busy {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if available {
            file_action = file_action.or_else(|| {
                ui.input_mut(|input| {
                    [
                        (
                            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                            egui::Key::S,
                            FileAction::SaveAs,
                        ),
                        (egui::Modifiers::COMMAND, egui::Key::S, FileAction::Save),
                        (egui::Modifiers::COMMAND, egui::Key::O, FileAction::Open),
                        (egui::Modifiers::COMMAND, egui::Key::N, FileAction::New),
                    ]
                    .into_iter()
                    .find_map(|(modifiers, key, action)| {
                        input.consume_key(modifiers, key).then_some(action)
                    })
                })
            });
        }
        if file_action.is_some() {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        // egui clears text focus at pass start on Escape, before tool shortcuts run.
        let text_was_focused = ui.ctx().data(|data| {
            data.get_temp::<bool>(ui.id().with("editor-text-focus"))
                .unwrap_or(false)
        });
        if !self.busy
            && self.analysis_confirmation.is_none()
            && !(text_was_focused && ui.input(|input| input.key_pressed(egui::Key::Escape)))
            && self.overwrite.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && !ui.ctx().egui_wants_keyboard_input()
            && !egui::Popup::is_any_open(ui.ctx())
        {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Z)) {
                self.viewport.fit();
            }
            if self.pick.is_some()
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.status = "Pick cancelled; processing unchanged".into();
                self.finish_pick();
                self.region = None;
            }
            if self.pick.is_some()
                && self.region.is_some()
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
            {
                self.apply_pick();
            }
            if self.crop_edit.is_some() {
                if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    self.finish_crop(false);
                } else if ui
                    .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    self.finish_crop(true);
                }
            }
            if self.pick.is_none()
                && self.crop_edit.is_none()
                && !self.document.project.order.is_empty()
            {
                let previous = ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                });
                let next = ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                });
                let index = if previous {
                    self.selected.saturating_sub(1)
                } else if next {
                    (self.selected + 1).min(self.document.project.order.len() - 1)
                } else {
                    self.selected
                };
                if index != self.selected {
                    self.selected = index;
                    let id = self.document.project.order[index];
                    if let Ok(path) = self.document.source_path(id) {
                        self.load(path);
                    }
                }
            }
        }
        let previous_roll = self.roll;
        let previous_photo = self.photo;
        let was_dirty = self.document.dirty();
        let calibration = self.calibration();
        if self.overwrite.is_some()
            || self.document_pending.is_some()
            || self.transition.is_some()
            || self.project_overwrite.is_some()
            || self.resume_transition.is_some()
            || self.analysis_confirmation.is_some()
        {
            ui.disable();
        }
        let mut export_action = None;
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("antitone");
                ui.add_enabled_ui(available, |ui| {
                    ui.menu_button("File", |ui| {
                        for (label, action) in [
                            ("New", FileAction::New),
                            ("Open…", FileAction::Open),
                            ("Add RAWs…", FileAction::Add),
                            ("Save", FileAction::Save),
                            ("Save As…", FileAction::SaveAs),
                        ] {
                            if ui.button(label).clicked() {
                                file_action = Some(action);
                                ui.close();
                            }
                        }
                    });
                });
                ui.add_enabled_ui(
                    available && calibration.is_some() && self.ready.is_some(),
                    |ui| {
                        ui.menu_button("Export", |ui| {
                            for (label, format) in [
                                ("TIFF…", OutputFormat::Tiff(self.target)),
                                ("EXR half…", OutputFormat::Exr(ExrPrecision::Half)),
                                ("EXR full…", OutputFormat::Exr(ExrPrecision::Full)),
                            ] {
                                if ui.button(label).clicked() {
                                    export_action = Some(format);
                                    ui.close();
                                }
                            }
                        });
                    },
                );
                ui.separator();
                let name = self
                    .document
                    .path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Untitled".into());
                ui.add(
                    egui::Label::new(format!(
                        "{name}{}",
                        if self.document.dirty() {
                            " • Modified"
                        } else {
                            ""
                        }
                    ))
                    .truncate(),
                )
                .on_hover_text(
                    self.document
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "Save to create a project document".into()),
                );
            });
        });
        if file_action.is_some() {
            ui.ctx().memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.status);
        });
        self.filmstrip(ui);
        let (scope_slot, analyze) = egui::Panel::right("controls")
            .default_size(315.)
            .min_size(270.)
            .show(ui, |ui| self.controls(ui))
            .inner;
        if let Some(calibration) = self.calibration()
            && let Err(error) = self.positive_parameters(calibration, self.roll, self.photo)
        {
            self.roll = previous_roll;
            self.photo = previous_photo;
            self.status = format!("Edit rejected: {error}");
        }
        let zoom_action = self.viewer_bar(ui);
        let viewer = egui::CentralPanel::default()
            .frame(egui::Frame::central_panel(ui.style()).fill(viewer_ui::surface()));
        viewer.show(ui, |ui| {
            let available = ui
                .available_rect_before_wrap()
                .intersect(ui.clip_rect())
                .size()
                .max(egui::vec2(1., 1.));
            let (canvas, response) =
                ui.allocate_exact_size(available, egui::Sense::click_and_drag());
            let Some((area, provisional, pending)) = self.selected_view() else {
                ui.painter().text(
                    canvas.center(),
                    egui::Align2::CENTER_CENTER,
                    if self.busy {
                        "Developing…"
                    } else {
                        "Add a RAW to begin"
                    },
                    egui::FontId::proportional(14.),
                    ui.visuals().text_color(),
                );
                return;
            };
            let pixels_per_point = ui.ctx().pixels_per_point();
            let canvas_input = self.canvas_input_allowed(ui, canvas);
            let crop = if matches!(
                self.pick,
                Some(Pick::Dmin | Pick::Dmax | Pick::Flat | Pick::Analysis)
            ) {
                None
            } else {
                self.crop_edit.as_ref().map(|edit| edit.draft).or(self.crop)
            };
            let composition = if matches!(self.pick, Some(Pick::Flat | Pick::Analysis)) {
                Composition::new(
                    area,
                    RollOrientation::default(),
                    PhotoOrientation::default(),
                )
            } else {
                Composition::with_crop(area, self.roll_orientation, self.photo_orientation, crop)
            }
            .expect("accepted crop belongs to the loaded source");
            let inset = 20_f32.min((canvas.width().min(canvas.height()) - 1.).max(0.) * 0.5);
            let layout_canvas = if self.crop_edit.is_some() {
                canvas.shrink(inset)
            } else {
                canvas
            };
            let cropping = self.crop_edit.is_some();
            let mut geometry = if cropping {
                self.viewport
                    .layout_crop(layout_canvas, composition, pixels_per_point)
            } else {
                self.viewport
                    .layout_composition(layout_canvas, composition, pixels_per_point)
            };
            if let Some(zoom) = zoom_action {
                self.viewport.zoom_at(
                    geometry,
                    pixels_per_point,
                    if zoom > 0. {
                        zoom
                    } else {
                        geometry.scale * pixels_per_point * -zoom
                    },
                    canvas.center(),
                );
            }
            geometry = if cropping {
                self.viewport
                    .layout_crop(layout_canvas, composition, pixels_per_point)
            } else {
                self.viewport
                    .layout_composition(layout_canvas, composition, pixels_per_point)
            };
            geometry.canvas = canvas;
            if response.drag_started_by(egui::PointerButton::Primary) {
                self.pick_drag = None;
            }
            let (geometry, panning) = if !canvas_input {
                (geometry, false)
            } else if self.crop_edit.is_some() {
                (self.crop_interaction(ui, &response, geometry), false)
            } else {
                self.navigate(ui, &response, geometry, pixels_per_point)
            };
            if self.crop_edit.is_none() {
                self.last_layout = Some((geometry, pixels_per_point));
            }
            let composition = geometry.composition;
            let rect = geometry.image.intersect(canvas);
            // An editing frame may be offscreen at explicit zoom; no refinement raster is requested.
            let view = (!cropping).then(|| ViewRequest {
                area: geometry.visible_area(),
                output: if composition.is_exact() {
                    geometry.output_size(geometry.visible_area(), pixels_per_point)
                } else {
                    geometry.frame_raster(pixels_per_point).output
                },
                frame: (!composition.is_exact()).then(|| geometry.frame_raster(pixels_per_point)),
            });
            let parameters = if self.source {
                let source_parameters =
                    ProcessingParameters::source_inspection(self.source_exposure);
                match source_parameters {
                    Ok(parameters) => parameters,
                    Err(error) => {
                        ui.label(format!("Source inspection unavailable: {error}"));
                        return;
                    }
                }
            } else {
                let Some(cal) = self.calibration() else {
                    ui.label("Positive preview requires valid calibration");
                    return;
                };
                match self.positive_parameters(cal, self.roll, self.photo) {
                    Ok(p) => p,
                    Err(error) => {
                        ui.label(format!("Positive preview unavailable: {error}"));
                        return;
                    }
                }
            };
            let signature = (
                bytemuck::bytes_of(&parameters).try_into().unwrap(),
                self.target,
                view,
                composition,
            );
            if self.signature != Some(signature) {
                self.signature = Some(signature);
                self.generation
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.last_edit = std::time::Instant::now();
                self.settled = None;
            }
            let revision = self.generation.load(std::sync::atomic::Ordering::Relaxed);
            if !self.busy
                && self.analysis_confirmation.is_none()
                && self.crop_edit.is_none()
                && self.refining.is_none()
                && self.settled.is_none()
                && self.refinement_error != Some(revision)
                && self.last_edit.elapsed() >= Duration::from_millis(250)
                && let Some(view) = view
            {
                if self
                    .commands
                    .send(Command::Refine(
                        self.id,
                        revision,
                        parameters,
                        self.target,
                        view,
                    ))
                    .is_ok()
                {
                    self.refining = Some((revision, view));
                } else {
                    self.refinement_error = Some(revision);
                    self.status = "RAW worker stopped".into();
                }
            }
            if self.refining.is_some()
                || self.settled.is_none() && self.refinement_error != Some(revision)
            {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
            let draw = if cropping { canvas } else { rect };
            let (image, parameters, mapping) = match &self.settled {
                Some((settled_revision, settled_view, image))
                    if *settled_revision == revision && !cropping =>
                {
                    let mapping = if settled_view.frame.is_some() {
                        crate::geometry::TextureMapping::IDENTITY
                    } else {
                        geometry.uv(draw, settled_view.area)
                    };
                    (image.clone(), None, mapping)
                }
                _ => (provisional, Some(parameters), geometry.uv(draw, area)),
            };
            ui.painter()
                .with_clip_rect(canvas)
                .add(egui_wgpu::Callback::new_paint_callback(
                    draw,
                    SelectionPaint::new(
                        ImagePaint::new(image, parameters, mapping),
                        self.selection_timing.clone(),
                        pending,
                    ),
                ));
            if cropping {
                self.paint_crop(ui, geometry);
            }
            if self.pick.is_some()
                && !self.busy
                && canvas_input
                && !panning
                && !ui.input(|input| input.key_down(egui::Key::Space))
            {
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(point) = response.interact_pointer_pos()
                    && geometry.image.contains(point)
                {
                    self.region = Some(geometry.pick_patch(point, 32));
                }
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.pick_drag = response
                        .interact_pointer_pos()
                        .filter(|point| geometry.image.contains(*point))
                        .map(|point| PickDrag::begin(geometry, point, self.region));
                }
                if response.dragged_by(egui::PointerButton::Primary)
                    && let (Some(drag), Some(end)) =
                        (self.pick_drag, response.interact_pointer_pos())
                {
                    self.region = Some(drag.update(geometry, end));
                }
            }
            if let Some(region) = self.region {
                ui.painter().with_clip_rect(canvas).rect_stroke(
                    geometry.screen_coverage(region),
                    0.,
                    egui::Stroke::new(1., egui::Color32::WHITE),
                    egui::StrokeKind::Inside,
                );
                let screen = geometry.screen_coverage(region);
                for point in [
                    screen.min,
                    screen.max,
                    screen.left_bottom(),
                    screen.right_top(),
                ] {
                    ui.painter().with_clip_rect(canvas).rect_filled(
                        egui::Rect::from_center_size(point, egui::vec2(6., 6.)),
                        0.,
                        egui::Color32::WHITE,
                    );
                }
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                self.pick_drag = None;
            }
        });
        let text_focused = ui.ctx().text_edit_focused();
        ui.ctx()
            .data_mut(|data| data.insert_temp(ui.id().with("editor-text-focus"), text_focused));
        self.paint_scope(scope_slot);
        self.sync_document();
        if self.document.dirty() != was_dirty {
            ui.ctx().request_repaint();
        }
        if let Some(transition) = self.resume_transition.take() {
            self.perform_transition(transition, ui.ctx());
        } else if let Some(action) = file_action {
            self.file_action(action, ui.ctx());
        } else if let Some(format) = export_action {
            self.export_dialog(format);
        } else if analyze {
            // Inspect the validated, focus-committed Photo state before confirming a reset.
            self.request_analysis();
        }
        self.document_dialogs(ui.ctx());
        self.analysis_dialog(ui.ctx());
        if let Some((path, format)) = self.overwrite.clone() {
            egui::Window::new("Replace existing output?")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(path.display().to_string());
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.overwrite = None;
                        }
                        if ui.button("Replace").clicked() {
                            self.overwrite = None;
                            self.export(path, format, Overwrite::Confirmed);
                        }
                    });
                });
        }
    }
}
