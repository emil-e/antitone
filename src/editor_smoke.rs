//! Scripted RAW/Metal editor, project and capture scenarios with explicit diagnostic anchors.
//! Uses production workers/rendering but drives editor state directly; project files are temporary.
//! Asserts executed phases and emits layout markers; visible frames alone do not prove completion.
//! Requires a desktop for presentation; does not test physical input or photographic quality.
use super::*;
pub struct SmokeEditor {
    editor: Editor,
    dmin_source: PathBuf,
    dmax_source: PathBuf,
    photo: PathBuf,
    dmin_region: Rect,
    dmax_region: Rect,
    phase: u8,
    frames: u32,
    start: std::time::Instant,
    layout_start: Option<std::time::Instant>,
    project_directory: Option<tempfile::TempDir>,
    unvisited: Option<PathBuf>,
    saved_project: Option<crate::project::Project>,
    comparison: Option<(DVec3, Vec<glam::Vec3>, Vec<glam::Vec3>)>,
    pub result: Arc<std::sync::Mutex<Option<String>>>,
}

impl SmokeEditor {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arguments: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if arguments.len() != 11 && arguments.len() != 12 {
            return Err("--smoke DMIN.ARW x y w h DMAX.ARW x y w h PHOTO.ARW".into());
        }
        let rect = |offset: usize| -> Result<Rect, Box<dyn std::error::Error>> {
            Ok(Rect {
                x: arguments[offset].parse()?,
                y: arguments[offset + 1].parse()?,
                width: arguments[offset + 2].parse()?,
                height: arguments[offset + 3].parse()?,
            })
        };
        let project_directory = (arguments.len() == 12)
            .then(tempfile::tempdir)
            .transpose()?;
        let mut editor = Editor::new(device, queue, None);
        if project_directory.is_some() {
            editor.document.project.development = Some(Development::new(
                crate::raw::version(),
                [2173., 1024., 1731., 1024.],
            ));
        }
        editor.add_raws(vec![PathBuf::from(&arguments[0])]);
        Ok(Self {
            editor,
            dmin_source: PathBuf::from(&arguments[0]),
            dmax_source: PathBuf::from(&arguments[5]),
            photo: PathBuf::from(&arguments[10]),
            dmin_region: rect(1)?,
            dmax_region: rect(6)?,
            phase: 0,
            frames: 0,
            start: std::time::Instant::now(),
            layout_start: None,
            project_directory,
            unvisited: arguments.get(11).map(PathBuf::from),
            saved_project: None,
            comparison: None,
            result: Arc::new(std::sync::Mutex::new(None)),
        })
    }
    fn compare_pixel(&self) -> Result<(DVec3, Vec<glam::Vec3>, Vec<glam::Vec3>), Error> {
        let document = &self.editor.document;
        let id = self.editor.current_source.expect("smoke selected source");
        let source = document.project.source(id)?;
        let scan = Scan::open(
            &document.source_path(id)?,
            document
                .project
                .development
                .as_ref()
                .map(|value| value.fixed_wb),
        )?;
        let pixel = scan.pixel(
            scan.area.x + scan.area.width / 2,
            scan.area.y + scan.area.height / 2,
        )?;
        let working = crate::processing::Pipeline::new(
            document.project.calibration()?.expect("smoke calibration"),
            document.project.roll,
            source.photo,
            scan.camera_to_ap1,
        )?
        .apply(pixel)?
        .rgb;
        Ok((
            working,
            crate::rendering::CpuRenderer::new(OutputTarget::Srgb)?.render(&[working])?,
            crate::rendering::CpuRenderer::new(OutputTarget::DisplayP3)?.render(&[working])?,
        ))
    }
    fn pick(&mut self, kind: Pick, region: Rect) {
        self.editor.busy = true;
        self.editor
            .commands
            .send(Command::Pick(
                self.editor.id,
                kind,
                Coverage::rectangle(region),
                self.editor.calibration(),
                self.editor.roll,
                self.editor.photo,
            ))
            .unwrap();
    }
}
impl host::App for SmokeEditor {
    fn submission_gate(&self) -> &crate::platform::SubmissionGate {
        self.editor.submission_gate()
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.editor.ui(ui);
        if self.start.elapsed()
            > Duration::from_secs(if self.project_directory.is_some() {
                120
            } else {
                45
            })
        {
            *self.result.lock().unwrap() =
                Some(format!("editor smoke timed out: {}", self.editor.status));
            self.editor.allow_close = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.editor.busy {
            return;
        }
        if self.editor.ready.is_none() && !matches!(self.phase, 17 | 18) {
            *self.result.lock().unwrap() = Some(self.editor.status.clone());
            self.editor.allow_close = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        match self.phase {
            0 => {
                self.pick(Pick::Dmin, self.dmin_region);
                self.phase = 1;
            }
            1 if self.editor.dmin.is_some() => {
                self.editor.load(self.dmax_source.clone());
                self.phase = 2;
            }
            2 => {
                self.pick(Pick::Dmax, self.dmax_region);
                self.phase = 3;
            }
            3 if self.editor.calibration().is_some() => {
                self.editor.load(self.photo.clone());
                self.phase = 4;
            }
            4 => {
                self.frames += 1;
                self.editor.source = self.frames >= 16;
                self.editor.source_exposure = if self.editor.source { 2. } else { 0. };
                self.editor.target = if self.frames < 8 {
                    OutputTarget::DisplayP3
                } else {
                    OutputTarget::Srgb
                };
                if std::env::var_os("ANTITONE_SMOKE_SCOPES").is_some() {
                    self.editor.scopes.vector = (8..16).contains(&self.frames);
                    self.editor.scopes.waveform = if self.frames < 4 {
                        crate::scopes::Plot::Rgb
                    } else {
                        crate::scopes::Plot::Luma
                    };
                    self.editor.scopes.amplification = [1, 2, 4, 8][(self.frames as usize % 8) / 2];
                }
                self.editor.roll.gamma = 0.75;
                self.editor.photo.exposure = 0.5;
                self.editor.photo.wb = Balance {
                    warmth: 0.1,
                    tint: -0.05,
                };
                if self.frames >= 24 {
                    self.editor.source = false;
                    self.editor.target = OutputTarget::DisplayP3;
                    self.phase = 5;
                }
            }
            5 => {
                if self.editor.settled.is_some() {
                    if std::env::var_os("ANTITONE_SMOKE_SCOPES").is_some() {
                        let gpu = self
                            .editor
                            .scopes
                            .gpu
                            .as_ref()
                            .expect("smoke scope GPU resources");
                        let (image, parameters, frame) = self.editor.scope_input().unwrap();
                        let settings = crate::scopes::Settings {
                            target: self.editor.target,
                            plot: self.editor.scopes.waveform,
                            amplification: 1,
                        };
                        let Some(result) = gpu.status(&image, parameters, frame, settings) else {
                            return;
                        };
                        let counts = result.expect("smoke scope diagnostics");
                        assert_eq!(counts.invalid, 0);
                        assert_eq!(counts.valid, frame.output[0] * frame.output[1]);
                        assert!(gpu.reduction_count() >= 3);
                        println!(
                            "scope_smoke target={:?} samples={} reductions={} invalid={}",
                            self.editor.target,
                            counts.valid,
                            gpu.reduction_count(),
                            counts.invalid
                        );
                    }
                    self.editor.viewport.zoom = Some(1.);
                    self.phase = 6;
                } else if self.editor.refinement_error.is_some() {
                    *self.result.lock().unwrap() = Some(self.editor.status.clone());
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            6 => {
                if let Some((_, view, _)) = &self.editor.settled {
                    assert_eq!(view.output, [view.area.width, view.area.height]);
                    assert!(view.area.width < self.editor.ready.as_ref().unwrap().area.width);
                    println!(
                        "editor_smoke 100_percent source_region={:?} output={:?}",
                        view.area, view.output
                    );
                    self.editor.begin_pick(Pick::Dmin);
                    assert_eq!(self.editor.ready.as_ref().unwrap().path, self.photo);
                    assert!(self.editor.region.is_none());
                    assert!(!self.editor.busy);
                    let source = self
                        .editor
                        .document
                        .find_source(&self.dmin_source)
                        .expect("smoke roll source");
                    self.editor.pick_roll_source(source);
                    self.phase = 7;
                }
            }
            7 => {
                assert!(document::same_path(
                    &self.editor.ready.as_ref().unwrap().path,
                    &self.dmin_source
                ));
                assert_eq!(
                    self.editor.current_source,
                    self.editor.document.find_source(&self.dmin_source)
                );
                self.editor.status = "Diagnostic reference pick cancelled".into();
                self.editor.finish_pick();
                self.phase = 8;
            }
            8 => {
                assert_eq!(self.editor.ready.as_ref().unwrap().path, self.photo);
                assert_eq!(self.editor.viewport.zoom, Some(1.));
                assert!(!self.editor.source);
                assert_eq!(self.editor.photo.exposure, 0.5);
                assert_eq!(
                    self.editor.photo.wb,
                    Balance {
                        warmth: 0.1,
                        tint: -0.05
                    }
                );
                self.phase = if std::env::var_os("ANTITONE_SMOKE_TOOLS").is_some() {
                    25
                } else {
                    13
                };
                self.frames = 0;
            }
            25..=30 => {
                let index = self.phase - 25;
                let stages = [
                    "crop-normal",
                    "dmin-normal",
                    "wb-normal",
                    "crop-narrow",
                    "dmin-narrow",
                    "wb-narrow",
                ];
                if self.layout_start.is_none() {
                    let narrow = index >= 3;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(if narrow {
                            egui::vec2(800., 600.)
                        } else {
                            egui::vec2(1100., 800.)
                        }));
                    if let Some(mut panel) = egui::containers::panel::PanelState::load(
                        ui.ctx(),
                        egui::Id::new("controls"),
                    ) {
                        panel.outer_rect.min.x =
                            panel.outer_rect.max.x - if narrow { 270. } else { 315. };
                        ui.ctx().data_mut(|data| {
                            data.insert_persisted(egui::Id::new("controls"), panel)
                        });
                    }
                    self.editor.viewport.fit();
                    self.editor.tab = if index.is_multiple_of(3) { 1 } else { 2 };
                    self.editor.scopes.visible = !index.is_multiple_of(3);
                    let area = self.editor.ready.as_ref().unwrap().area;
                    if index.is_multiple_of(3) {
                        self.editor.begin_crop();
                        self.editor.crop_edit.as_mut().unwrap().draft = Crop::from_region(Rect {
                            x: area.x + area.width / 8,
                            y: area.y + area.height / 8,
                            width: area.width * 3 / 4,
                            height: area.height * 3 / 4,
                        })
                        .straightened(area, 0.04)
                        .unwrap();
                    } else {
                        self.editor
                            .begin_pick(if index % 3 == 1 { Pick::Dmin } else { Pick::Wb });
                        self.editor.region = Some(Coverage::rectangle(Rect {
                            x: area.x + area.width / 2,
                            y: area.y + area.height / 2,
                            width: 128,
                            height: 128,
                        }));
                        self.editor.source_exposure = 2.;
                    }
                    self.layout_start = Some(std::time::Instant::now());
                }
                let expected_mode = match index % 3 {
                    0 => self.editor.crop_edit.is_some(),
                    1 => self.editor.pick == Some(Pick::Dmin),
                    _ => self.editor.pick == Some(Pick::Wb),
                };
                if !expected_mode {
                    *self.result.lock().unwrap() = Some(format!(
                        "tools smoke interrupted during {}: {}",
                        stages[usize::from(index)],
                        self.editor.status
                    ));
                    self.editor.allow_close = true;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                let elapsed = self.layout_start.unwrap().elapsed();
                if elapsed > Duration::from_secs(1) && self.frames == 0 {
                    println!("tools_layout stage={}", stages[usize::from(index)]);
                    self.frames = 1;
                }
                if elapsed > Duration::from_millis(2500) {
                    if self.editor.crop_edit.is_some() {
                        self.editor.finish_crop(false);
                    } else {
                        self.editor.finish_pick();
                    }
                    self.phase = if index == 5 { 13 } else { self.phase + 1 };
                    self.layout_start = None;
                    self.frames = 0;
                }
            }
            13 if self.project_directory.is_some() => {
                self.editor
                    .add_raws(vec![self.unvisited.clone().expect("unvisited RAW")]);
                self.phase = 14;
            }
            14 => {
                let unvisited = self
                    .editor
                    .document
                    .find_source(self.unvisited.as_ref().unwrap())
                    .unwrap();
                assert_eq!(
                    self.editor
                        .document
                        .project
                        .source(unvisited)
                        .unwrap()
                        .photo,
                    PhotoSettings::default()
                );
                self.editor.photo.exposure = 7.123456789012345;
                self.editor.roll.response = crate::processing::Response::Film {
                    toe: 2.789123456789,
                    shoulder: 0.003123456789,
                };
                let current = self.editor.current_source.unwrap();
                let position = self
                    .editor
                    .document
                    .project
                    .order
                    .iter()
                    .position(|id| *id == current)
                    .unwrap();
                self.editor.document.project.order.swap(0, position);
                self.editor.selected = 0;
                self.editor.sync_document();
                self.comparison = Some(self.compare_pixel().unwrap());
                self.saved_project = Some(self.editor.document.project.clone());
                let path = self
                    .project_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("film.antitone.json");
                self.editor
                    .queue_save(path, crate::project::Save::New, None);
                self.phase = 15;
            }
            15 => {
                assert!(!self.editor.document.dirty());
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                self.editor.start_document(DocumentWork::New);
                self.phase = 17;
            }
            17 => {
                assert!(self.editor.document.project.development.is_none());
                self.editor.add_raws(vec![self.dmax_source.clone()]);
                self.phase = 18;
            }
            18 => {
                let wb = self
                    .editor
                    .document
                    .project
                    .development
                    .as_ref()
                    .unwrap()
                    .fixed_wb;
                assert_eq!(
                    wb,
                    crate::raw::RawFile::open(&self.dmax_source)
                        .unwrap()
                        .metadata
                        .as_shot_wb()
                );
                assert_ne!(
                    wb,
                    self.saved_project
                        .as_ref()
                        .unwrap()
                        .development
                        .as_ref()
                        .unwrap()
                        .fixed_wb
                );
                self.editor.start_document(DocumentWork::Open(
                    self.project_directory
                        .as_ref()
                        .unwrap()
                        .path()
                        .join("film.antitone.json"),
                ));
                self.phase = 19;
            }
            19 => {
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                assert!(!self.editor.document.dirty());
                assert_eq!(
                    self.editor.ready.as_ref().unwrap().development.fixed_wb,
                    self.editor
                        .document
                        .project
                        .development
                        .as_ref()
                        .unwrap()
                        .fixed_wb
                );
                if self.editor.settled.is_some() {
                    assert_eq!(
                        self.compare_pixel().unwrap(),
                        *self.comparison.as_ref().unwrap()
                    );
                    self.editor.roll.response = crate::processing::Response::Original;
                    self.editor.sync_document();
                    self.comparison = Some(self.compare_pixel().unwrap());
                    self.saved_project = Some(self.editor.document.project.clone());
                    self.editor.queue_save(
                        self.project_directory
                            .as_ref()
                            .unwrap()
                            .path()
                            .join("original.antitone.json"),
                        crate::project::Save::New,
                        None,
                    );
                    self.phase = 20;
                }
            }
            20 => {
                self.editor.start_document(DocumentWork::Open(
                    self.project_directory
                        .as_ref()
                        .unwrap()
                        .path()
                        .join("original.antitone.json"),
                ));
                self.phase = 21;
            }
            21 => {
                assert!(self.editor.roll.response.is_original());
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                assert!(!self.editor.document.dirty());
                assert_eq!(
                    self.compare_pixel().unwrap(),
                    *self.comparison.as_ref().unwrap()
                );
                let mut changed = self.editor.document.project.clone();
                changed
                    .source_mut(changed.order[0])
                    .unwrap()
                    .development
                    .dimensions[0] += 1;
                let path = self
                    .project_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("changed.antitone.json");
                changed.save(&path, crate::project::Save::New).unwrap();
                self.editor.start_document(DocumentWork::Open(path));
                self.phase = 22;
            }
            22 => {
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                assert!(self.editor.status.contains("development identity differs"));
                let mut missing = self.editor.document.project.clone();
                missing.source_mut(missing.order[0]).unwrap().path = self
                    .project_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("missing.ARW");
                let path = self
                    .project_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("missing.antitone.json");
                missing.save(&path, crate::project::Save::New).unwrap();
                self.editor.start_document(DocumentWork::Open(path));
                self.phase = 23;
            }
            23 => {
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                assert!(self.editor.status.contains("missing.ARW"));
                assert!(self.editor.ready.is_some());
                let mut incompatible = self.editor.document.project.clone();
                incompatible
                    .source_mut(incompatible.order[0])
                    .unwrap()
                    .development
                    .dimensions[0] += 2;
                let path = self
                    .project_directory
                    .as_ref()
                    .unwrap()
                    .path()
                    .join("incompatible.antitone.json");
                incompatible.save(&path, crate::project::Save::New).unwrap();
                self.editor.start_document(DocumentWork::Open(path));
                self.phase = 24;
            }
            24 => {
                assert_eq!(
                    self.editor.document.project,
                    *self.saved_project.as_ref().unwrap()
                );
                assert!(self.editor.status.contains("development identity differs"));
                assert!(self.editor.ready.is_some());
                if self
                    .editor
                    .thumbnails
                    .ready_count(&self.editor.document, self.editor.target)
                    < self.editor.document.project.order.len().min(16)
                {
                    return;
                }
                println!(
                    "thumbnail_smoke ready={} retained selected RAW/document after background jobs",
                    self.editor
                        .thumbnails
                        .ready_count(&self.editor.document, self.editor.target)
                );
                println!(
                    "project_smoke passed: stable IDs/order/unvisited RAW, custom saved WB after New/other-WB import, exact camera-model-4 response state and working/sRGB/P3 pixel reopen, live settled GPU preview, development-mismatched/missing/incompatible ordered RAW atomicity"
                );
                self.editor.allow_close = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            13 => {
                self.editor.allow_close = true;
                println!(
                    "editor_smoke passed: full-resolution calibration, GPU edits/both targets/source bypass, fit and native-pixel settled views, RGB-only Dmin/Dmax, current-photo re-pick, explicit reference source switching and working-photo return; elapsed_seconds={:.2}",
                    self.start.elapsed().as_secs_f64()
                );
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            _ => {
                *self.result.lock().unwrap() = Some(self.editor.status.clone());
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        ui.ctx().request_repaint();
    }
}
