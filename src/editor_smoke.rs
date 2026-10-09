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
    pub result: Arc<std::sync::Mutex<Option<String>>>,
}

impl SmokeEditor {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        arguments: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if arguments.len() != 11 {
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
        Ok(Self {
            editor: Editor::new(device, queue, Some(PathBuf::from(&arguments[0]))),
            dmin_source: PathBuf::from(&arguments[0]),
            dmax_source: PathBuf::from(&arguments[5]),
            photo: PathBuf::from(&arguments[10]),
            dmin_region: rect(1)?,
            dmax_region: rect(6)?,
            phase: 0,
            frames: 0,
            start: std::time::Instant::now(),
            layout_start: None,
            result: Arc::new(std::sync::Mutex::new(None)),
        })
    }
    fn pick(&mut self, kind: Pick, region: Rect) {
        self.editor.busy = true;
        self.editor
            .commands
            .send(Command::Pick(
                self.editor.id,
                kind,
                region,
                self.editor.calibration(),
                self.editor.roll,
                self.editor.photo,
            ))
            .unwrap();
    }
}
impl host::App for SmokeEditor {
    fn ui(&mut self, ui: &mut egui::Ui) {
        self.editor.ui(ui);
        if self.start.elapsed() > Duration::from_secs(45) {
            *self.result.lock().unwrap() =
                Some(format!("editor smoke timed out: {}", self.editor.status));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if self.editor.busy {
            return;
        }
        if self.editor.ready.is_none() {
            *self.result.lock().unwrap() = Some(self.editor.status.clone());
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
                self.editor.target = if self.frames < 8 {
                    OutputTarget::DisplayP3
                } else {
                    OutputTarget::Srgb
                };
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
                    self.editor.pick_source(self.dmin_source.clone());
                    self.phase = 7;
                }
            }
            7 => {
                assert_eq!(self.editor.ready.as_ref().unwrap().path, self.dmin_source);
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
                self.phase = if std::env::var_os("ANTITONE_SMOKE_CONTROLS").is_some() {
                    9
                } else {
                    13
                };
                self.frames = 0;
            }
            9..=12 => {
                let index = self.phase - 9;
                if self.layout_start.is_none() {
                    self.editor.tab = usize::from(index % 2);
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                    let narrow = index >= 2;
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
                    self.layout_start = Some(std::time::Instant::now());
                }
                let elapsed = self.layout_start.unwrap().elapsed();
                if elapsed > Duration::from_secs(1) && self.frames == 0 {
                    println!(
                        "controls_layout stage={}",
                        ["roll-normal", "photo-normal", "roll-narrow", "photo-narrow"]
                            [usize::from(index)]
                    );
                    self.frames = 1;
                }
                if elapsed > Duration::from_secs(4) {
                    self.phase += 1;
                    self.layout_start = None;
                    self.frames = 0;
                }
            }
            13 => {
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
