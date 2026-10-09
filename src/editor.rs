use crate::{
    Error,
    color::Balance,
    export::{self, ExrPrecision, ImageSize, Overwrite},
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
#[path = "editor_smoke.rs"]
mod smoke;
pub use smoke::SmokeEditor;
#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pick {
    Dmin,
    Dmax,
    Gamma,
    Wb,
}
struct PickReturn {
    path: PathBuf,
    selected: usize,
    viewport: Viewport,
}
struct Ready {
    path: PathBuf,
    area: Rect,
    floor: f64,
    images: [Arc<GpuImage>; 2],
}
enum Command {
    Refine(u64, u64, ProcessingParameters, OutputTarget, ViewRequest),
    Load(u64, PathBuf),
    Pick(
        u64,
        Pick,
        Rect,
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
    Refined(u64, Result<Option<Arc<GpuImage>>, String>),
    Ready(u64, Result<Ready, String>),
    Picked(u64, Pick, Result<Measurement, String>),
    Exported(u64, Result<PathBuf, String>),
}

fn backend(
    device: wgpu::Device,
    queue: wgpu::Queue,
    commands: mpsc::Receiver<Command>,
    messages: mpsc::Sender<Message>,
    generation: Arc<std::sync::atomic::AtomicU64>,
) {
    let mut source: Option<(u64, Scan)> = None;
    let mut fixed_wb = None;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Refine(id, revision, parameters, target, view) => {
                let result = match &source {
                    Some((loaded, scan)) if *loaded == id => crate::refinement::refine(
                        &device,
                        &queue,
                        scan,
                        parameters,
                        target,
                        (&generation, revision),
                        view,
                    )
                    .map_err(|e| e.to_string()),
                    _ => Ok(None),
                };
                if messages.send(Message::Refined(revision, result)).is_err() {
                    break;
                }
            }
            Command::Load(id, path) => {
                let result = (|| -> Result<Ready, Error> {
                    let scan = Scan::open(&path, fixed_wb)?;
                    let (width, height, pixels) = scan.preview_source(1024)?;
                    let images = [
                        Arc::new(pollster::block_on(GpuImage::new(
                            &device,
                            &queue,
                            width,
                            height,
                            &pixels,
                            scan.camera_to_ap0,
                            OutputTarget::Srgb,
                        ))?),
                        Arc::new(pollster::block_on(GpuImage::new(
                            &device,
                            &queue,
                            width,
                            height,
                            &pixels,
                            scan.camera_to_ap0,
                            OutputTarget::DisplayP3,
                        ))?),
                    ];
                    let ready = Ready {
                        path,
                        area: scan.area,
                        floor: scan.input_floor,
                        images,
                    };
                    fixed_wb = Some(scan.fixed_wb);
                    source = Some((id, scan));
                    Ok(ready)
                })()
                .map_err(|e| e.to_string());
                if messages.send(Message::Ready(id, result)).is_err() {
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
                            scan.reference(region).map(Measurement::Reference)
                        }
                        Pick::Gamma => scan
                            .roll_neutral(
                                region,
                                calibration.ok_or(Error::UnusableSample(
                                    "both film references are required",
                                ))?,
                                roll,
                            )
                            .map(Measurement::Balance),
                        Pick::Wb => scan
                            .photo_neutral(
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
            Command::Export(id, path, format, calibration, roll, photo, overwrite) => {
                let result = (|| -> Result<PathBuf, Error> {
                    let (loaded, scan) = source
                        .as_ref()
                        .ok_or(Error::UnusableSample("no developed source"))?;
                    if id != *loaded {
                        return Err(Error::UnusableSample("source changed before export"));
                    }
                    let pipeline = crate::processing::Pipeline::new(calibration, roll, photo)?;
                    let area = scan.area;
                    let size = ImageSize {
                        width: area.width,
                        height: area.height,
                    };
                    let pixel = |x, y| {
                        pipeline
                            .apply(scan.pixel(area.x + x, area.y + y)?)
                            .map(|p| p.rgb)
                    };
                    match format {
                        OutputFormat::Tiff(target) => {
                            export::tiff(&path, size, target, overwrite, roll.response, pixel)?
                        }
                        OutputFormat::Exr(precision) => {
                            export::exr(&path, size, precision, overwrite, roll.response, pixel)?
                        }
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
    generation: Arc<std::sync::atomic::AtomicU64>,
    signature: Option<(
        [u8; ProcessingParameters::BYTE_LEN],
        OutputTarget,
        ViewRequest,
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
    busy: bool,
    status: String,
    return_notice: Option<String>,
    dmin: Option<DVec3>,
    dmax: Option<DVec3>,
    floor: f64,
    roll: RollSettings,
    photo: PhotoSettings,
    target: OutputTarget,
    source: bool,
    pick: Option<Pick>,
    pick_view: Option<bool>,
    pick_return: Option<PickReturn>,
    region: Option<Rect>,
    pick_drag: Option<PickDrag>,
    tab: usize,
    files: Vec<PathBuf>,
    photo_settings: std::collections::HashMap<PathBuf, PhotoSettings>,
    selected: usize,
    overwrite: Option<(PathBuf, OutputFormat)>,
}
impl Editor {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, initial: Option<PathBuf>) -> Self {
        let (commands, receiver) = mpsc::channel();
        let (send, messages) = mpsc::channel();
        let device = device.clone();
        let queue = queue.clone();
        let generation = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let worker_generation = generation.clone();
        std::thread::spawn(move || backend(device, queue, receiver, send, worker_generation));
        let mut editor = Self::state(commands, messages, generation);
        if let Some(path) = initial {
            editor.files.push(path.clone());
            editor.load(path);
        }
        editor
    }
    fn state(
        commands: mpsc::Sender<Command>,
        messages: mpsc::Receiver<Message>,
        generation: Arc<std::sync::atomic::AtomicU64>,
    ) -> Self {
        Self {
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
            busy: false,
            status: "Add a RAW to begin".into(),
            return_notice: None,
            dmin: None,
            dmax: None,
            floor: 0.,
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),
            target: OutputTarget::DisplayP3,
            source: true,
            pick: None,
            pick_view: None,
            pick_return: None,
            region: None,
            pick_drag: None,
            tab: 0,
            files: Vec::new(),
            photo_settings: std::collections::HashMap::new(),
            selected: 0,
            overwrite: None,
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
    fn load(&mut self, path: PathBuf) {
        if let Some(ready) = &self.ready {
            self.photo_settings.insert(ready.path.clone(), self.photo);
        }
        self.photo = self.photo_settings.get(&path).copied().unwrap_or_default();
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.signature = None;
        self.settled = None;
        self.viewport.fit();
        self.overwrite = None;
        self.return_notice = None;
        self.pick_view = None;
        self.pick_return = None;
        self.id += 1;
        self.ready = None;
        self.region = None;
        self.pick = None;
        self.source = self.calibration().is_none();
        self.busy = true;
        self.status = "Developing full-resolution RAW…".into();
        if self.commands.send(Command::Load(self.id, path)).is_err() {
            self.busy = false;
            self.status = "RAW worker stopped".into();
        }
    }
    fn receive(&mut self) {
        while let Ok(message) = self.messages.try_recv() {
            match message {
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
                Message::Ready(id, result) if id == self.id => {
                    self.busy = false;
                    match result {
                        Ok(ready) => {
                            if self.floor == 0. {
                                self.floor = ready.floor;
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
                            self.ready = Some(ready);
                        }
                        Err(error) => self.status = error,
                    }
                }
                Message::Picked(id, kind, result) if id == self.id => {
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
                                        ProcessingParameters::positive(cal, self.roll, self.photo)
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
                            if kind == Pick::Gamma {
                                roll.balance = balance;
                            } else {
                                photo.wb = balance;
                            }
                            if let Some(cal) = self.calibration()
                                && let Err(error) = ProcessingParameters::positive(cal, roll, photo)
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
                Message::Exported(id, result) if id == self.id => {
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
        let source = self
            .pick_view
            .take()
            .unwrap_or(self.calibration().is_none());
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
            self.viewport = context.viewport;
        }
        self.source = source;
    }
    fn begin_pick(&mut self, kind: Pick) {
        if self.pick.is_some() {
            self.finish_pick();
        }
        if self.busy {
            return;
        }
        self.pick_view = self.calibration().is_some().then_some(self.source);
        self.pick = Some(kind);
        self.region = None;
        self.pick_drag = None;
        self.source = matches!(kind, Pick::Dmin | Pick::Dmax);
        if self.source {
            self.pick_return = self.ready.as_ref().map(|ready| PickReturn {
                path: ready.path.clone(),
                selected: self.selected,
                viewport: self.viewport,
            });
        }
    }
    fn pick_source(&mut self, path: PathBuf) {
        let kind = self.pick;
        let context = self.pick_return.take();
        let view = self.pick_view;
        if self.ready.as_ref().is_none_or(|ready| ready.path != path) {
            self.load(path.clone());
        }
        if let Some(index) = self.files.iter().position(|file| *file == path) {
            self.selected = index;
        }
        self.pick = kind;
        self.pick_return = context;
        self.pick_view = view;
        self.region = None;
        self.source = true;
    }
    fn apply_pick(&mut self) {
        if let (Some(kind), Some(region)) = (self.pick, self.region) {
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
        if let Some(calibration) = self.calibration() {
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
                    overwrite,
                ))
                .is_err()
            {
                self.busy = false;
                self.status = "RAW worker stopped".into();
            }
        }
    }
    fn controls(&mut self, ui: &mut egui::Ui) {
        use controls::{ColorControl, Parameter};
        let editable = !self.busy && self.calibration().is_some();
        let can_pick = !self.busy && self.ready.is_some();
        let mut start_pick = None;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.tab, 0, "Roll");
            ui.selectable_value(&mut self.tab, 1, "Photo");
        });
        // Reserve four rows plus section chrome before the panel has a cached size.
        let output_height =
            4. * ui.spacing().interact_size.y + 3. * ui.spacing().item_spacing.y + 36.;
        egui::Panel::bottom("control-output")
            .default_size(output_height)
            .resizable(false)
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                let (_, reset) = controls::section(
                    ui, "output", "Output", Some(self.target != OutputTarget::DisplayP3),
                    |ui| {
                        ui.label("Preview / TIFF").on_hover_text(
                            "100-nit ACES 2 SDR rendering with matching TIFF encoding/ICC. Preview is converted to tagged extended sRGB for OS monitor management. EXR remains scene-linear AP0.",
                        );
                        ui.radio_value(&mut self.target, OutputTarget::DisplayP3, "Display P3 / D65");
                        ui.radio_value(&mut self.target, OutputTarget::Srgb, "sRGB / D65");
                    },
                );
                if reset {
                    self.target = OutputTarget::DisplayP3;
                }
            });
        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.tab == 0 {
                ui.push_id("roll", |ui| {
                    controls::section(ui, "calibration", "Calibration", None, |ui| {
                        if controls::calibration(
                            ui, "dmin", "Dmin (film base)", self.dmin, can_pick,
                            "Minimum-density film base. Linear AP0/D60 RGB, not optical-density numbers.",
                        ) {
                            start_pick = Some(Pick::Dmin);
                        }
                        if controls::calibration(
                            ui, "dmax", "Dmax", self.dmax, can_pick,
                            "Maximum-density film reference. Linear AP0/D60 RGB, not optical-density numbers.",
                        ) {
                            start_pick = Some(Pick::Dmax);
                        }
                    });
                    ui.add_enabled_ui(editable, |ui| {
                        let inversion_changed = self.roll.gamma != crate::processing::DEFAULT_GAMMA
                            || self.roll.balance != Balance::default();
                        let (_, reset) = controls::section(
                            ui, "gamma-balance", "Gamma & balance", Some(inversion_changed),
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
                        let (_, reset) = controls::section(
                            ui, "response", "Toe & shoulder",
                            Some(!self.roll.response.is_original() && self.roll.response != crate::processing::Response::film()),
                            |ui| {
                                match &mut self.roll.response {
                                    crate::processing::Response::Original => {
                                        ui.label("Original response · model 2");
                                        if ui.button("Use adjustable response").on_hover_text(
                                            "Explicitly use toe 2.5 with shoulder off. Calibration, gamma, balance and exposure are kept; this changes the response, not RAW development or ACES.",
                                        ).clicked() {
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
                        let (_, reset) = controls::section(
                            ui, "exposure", "Exposure", Some(self.roll.exposure != 0.),
                            |ui| {
                                controls::parameter(ui, Parameter::Exposure, &mut self.roll.exposure);
                            },
                        );
                        if reset {
                            self.roll.exposure = 0.;
                        }
                    });
                });
            } else {
                ui.push_id("photo", |ui| {
                    ui.add_enabled_ui(editable, |ui| {
                        if controls::color(
                            ui, ColorControl::WhiteBalance, &mut self.photo.wb, can_pick,
                        ) {
                            start_pick = Some(Pick::Wb);
                        }
                        let (_, reset) = controls::section(
                            ui, "exposure", "Exposure", Some(self.photo.exposure != 0.),
                            |ui| {
                                controls::parameter(ui, Parameter::Exposure, &mut self.photo.exposure);
                            },
                        );
                        if reset {
                            self.photo.exposure = 0.;
                        }
                    });
                });
            }
        });
        if let Some(kind) = start_pick {
            self.begin_pick(kind);
        }
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
    fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.started {
            self.started = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1100., 800.)));
        }
        self.receive();
        if self.busy {
            if ui.input(|input| input.viewport().close_requested()) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.status = "Work in progress; wait before closing this prototype".into();
            }
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        if !self.busy && self.overwrite.is_none() && !ui.ctx().egui_wants_keyboard_input() {
            if self.pick.is_some()
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
            {
                self.status = "Pick cancelled; processing unchanged".into();
                self.finish_pick();
                self.region = None;
            }
            if self.pick.is_none() && !self.files.is_empty() {
                let previous = ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)
                });
                let next = ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)
                });
                let index = if previous {
                    self.selected.saturating_sub(1)
                } else if next {
                    (self.selected + 1).min(self.files.len() - 1)
                } else {
                    self.selected
                };
                if index != self.selected {
                    self.selected = index;
                    self.load(self.files[index].clone());
                }
            }
        }
        let previous_roll = self.roll;
        let previous_photo = self.photo;
        let calibration = self.calibration();
        if self.overwrite.is_some() {
            ui.disable();
        }
        egui::Panel::top("header").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("antitone");
                if ui
                    .add_enabled(!self.busy, egui::Button::new("Add RAWs…"))
                    .clicked()
                    && let Some(paths) = rfd::FileDialog::new()
                        .add_filter("Camera RAW", &["arw"])
                        .pick_files()
                {
                    let first = self.files.len();
                    self.files.extend(paths);
                    if self.selected >= self.files.len() || self.ready.is_none() {
                        self.selected = first;
                        if let Some(path) = self.files.get(first).cloned() {
                            self.load(path);
                        }
                    }
                }
                ui.add_enabled_ui(
                    !self.busy && calibration.is_some() && self.ready.is_some(),
                    |ui| {
                        if ui.button("Export TIFF…").clicked() {
                            self.export_dialog(OutputFormat::Tiff(self.target));
                        }
                        if ui.button("EXR half…").clicked() {
                            self.export_dialog(OutputFormat::Exr(ExrPrecision::Half));
                        }
                        if ui.button("EXR full…").clicked() {
                            self.export_dialog(OutputFormat::Exr(ExrPrecision::Full));
                        }
                    },
                );
                ui.separator();
                ui.weak("Unsaved").on_hover_text("This session is not saved. Project Save/Open and recovery are not implemented yet.");
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.status);
        });
        egui::Panel::bottom("filmstrip").show(ui, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let mut select = None;
                    for (index, path) in self.files.iter().enumerate() {
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::selectable(
                                    index == self.selected
                                        && self
                                            .ready
                                            .as_ref()
                                            .is_some_and(|ready| ready.path == *path),
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                ),
                            )
                            .clicked()
                        {
                            select = Some(index);
                        }
                    }
                    if let Some(index) = select {
                        let path = self.files[index].clone();
                        if matches!(self.pick, Some(Pick::Dmin | Pick::Dmax)) {
                            self.pick_source(path);
                        } else {
                            self.selected = index;
                            self.load(path);
                        }
                    }
                });
            });
        });
        egui::Panel::right("controls")
            .default_size(315.)
            .min_size(270.)
            .show(ui, |ui| self.controls(ui));
        if let Some(calibration) = self.calibration()
            && let Err(error) = ProcessingParameters::positive(calibration, self.roll, self.photo)
        {
            self.roll = previous_roll;
            self.photo = previous_photo;
            self.status = format!("Edit rejected: {error}");
        }
        egui::CentralPanel::default().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.source, "Developed source");
                if calibration.is_none() {
                    self.source = true;
                    ui.label("Dmin and Dmax required for a positive preview");
                }
                if matches!(self.pick, Some(Pick::Dmin | Pick::Dmax))
                    && ui
                        .add_enabled(!self.busy, egui::Button::new("Choose reference RAW…"))
                        .clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Camera RAW", &["arw"])
                        .pick_file()
                {
                    self.pick_source(path);
                }
                if let Some(pick) = self.pick {
                    if let Some(ready) = &self.ready {
                        ui.label(format!("Sampling {}", ready.path.display()));
                    }
                    ui.label(format!(
                        "{pick:?}: click for a patch or drag a rectangle; adjust before applying"
                    ));
                    if ui
                        .add_enabled(!self.busy, egui::Button::new("Cancel pick"))
                        .clicked()
                    {
                        self.status = "Pick cancelled; processing unchanged".into();
                        self.finish_pick();
                        self.region = None;
                    }
                }
            });
            let Some(ready) = &self.ready else {
                ui.centered_and_justified(|ui| {
                    ui.label(if self.busy {
                        "Developing…"
                    } else {
                        "Add a RAW to begin"
                    });
                });
                return;
            };
            ui.small(if self.settled.is_some() {
                "Preview ready"
            } else if self.refining.is_some() {
                "Refining preview…"
            } else {
                "Approximate preview"
            }).on_hover_text("Interactive preview uses nearest-center samples. Settled preview processes every visible full-resolution source pixel before linear-target area averaging. Calibration and export always use the full source.");
            let area = ready.area;
            let provisional = ready.images[self.target as usize].clone();
            let pixels_per_point = ui.ctx().pixels_per_point();
            let mut zoom_action = None;
            ui.horizontal(|ui| {
                if ui.button("Fit").clicked() {
                    self.viewport.fit();
                }
                if ui.button("100%").clicked() {
                    zoom_action = Some(1.);
                }
                if ui.button("−").clicked() {
                    zoom_action = Some(-0.8);
                }
                if ui.button("+").clicked() {
                    zoom_action = Some(-1.25);
                }
                ui.label("?").on_hover_text("Wheel: zoom. Middle/space drag: pan.");
            });
            let available = (ui.available_size()
                - egui::vec2(0., if self.pick.is_some() { 36. } else { 0. }))
            .max(egui::vec2(1., 1.));
            let (canvas, response) =
                ui.allocate_exact_size(available, egui::Sense::click_and_drag());
            let mut geometry = self.viewport.layout(canvas, area, pixels_per_point);
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
            if response.hovered() {
                let (wheel, gesture, pointer) = ui.input(|input| {
                    (
                        input.smooth_scroll_delta().y,
                        input.zoom_delta(),
                        input.pointer.hover_pos(),
                    )
                });
                if let Some(pointer) = pointer
                    && (wheel != 0. || gesture != 1.)
                {
                    self.viewport.zoom_at(
                        geometry,
                        pixels_per_point,
                        geometry.scale * pixels_per_point * gesture * (wheel * 0.002).exp(),
                        pointer,
                    );
                }
            }
            let panning = response.dragged_by(egui::PointerButton::Middle)
                || response.dragged_by(egui::PointerButton::Primary)
                    && (self.pick.is_none() || ui.input(|input| input.key_down(egui::Key::Space)));
            if panning {
                self.viewport
                    .pan(geometry, pixels_per_point, response.drag_delta());
            }
            geometry = self.viewport.layout(canvas, area, pixels_per_point);
            let rect = geometry.image.intersect(canvas);
            let view = ViewRequest {
                area: geometry.visible_area(),
                output: geometry.output_size(geometry.visible_area(), pixels_per_point),
            };
            let parameters = if self.source {
                ProcessingParameters::source()
            } else {
                let Some(cal) = self.calibration() else {
                    ui.label("Positive preview requires valid calibration");
                    return;
                };
                match ProcessingParameters::positive(cal, self.roll, self.photo) {
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
                && self.refining.is_none()
                && self.settled.is_none()
                && self.refinement_error != Some(revision)
                && self.last_edit.elapsed() >= Duration::from_millis(250)
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
            let (image, parameters, sampled) = match &self.settled {
                Some((settled_revision, settled_view, image)) if *settled_revision == revision => {
                    (image.clone(), None, settled_view.area)
                }
                _ => (provisional, Some(parameters), area),
            };
            ui.painter()
                .with_clip_rect(canvas)
                .add(egui_wgpu::Callback::new_paint_callback(
                    rect,
                    ImagePaint::new(image, parameters, geometry.uv(rect, sampled)),
                ));
            if self.pick.is_some()
                && !self.busy
                && !panning
                && !ui.input(|input| input.key_down(egui::Key::Space))
            {
                if response.clicked_by(egui::PointerButton::Primary)
                    && let Some(point) = response.interact_pointer_pos()
                    && geometry.image.contains(point)
                {
                    self.region = Some(geometry.patch(point, 32));
                }
                if response.drag_started_by(egui::PointerButton::Primary) {
                    self.pick_drag = response
                        .interact_pointer_pos()
                        .filter(|point| geometry.image.contains(*point))
                        .map(|point| PickDrag::begin(geometry, point, self.region));
                }
                if response.dragged()
                    && let (Some(drag), Some(end)) =
                        (self.pick_drag, response.interact_pointer_pos())
                {
                    self.region = Some(drag.update(geometry, end));
                }
                if let Some(region) = self.region {
                    ui.painter().with_clip_rect(canvas).rect_stroke(
                        geometry.screen_region(region),
                        0.,
                        egui::Stroke::new(1., egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                    let screen = geometry.screen_region(region);
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
                    ui.horizontal(|ui| {
                        ui.label("Source rectangle");
                        let region = self.region.as_mut().unwrap();
                        ui.add(
                            egui::DragValue::new(&mut region.x)
                                .range(area.x..=area.x + area.width - 1)
                                .prefix("x "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut region.y)
                                .range(area.y..=area.y + area.height - 1)
                                .prefix("y "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut region.width)
                                .range(1..=area.x + area.width - region.x)
                                .prefix("w "),
                        );
                        ui.add(
                            egui::DragValue::new(&mut region.height)
                                .range(1..=area.y + area.height - region.y)
                                .prefix("h "),
                        );
                        if ui.button("Apply pick").clicked() {
                            self.apply_pick();
                        }
                    });
                }
            }
        });
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
