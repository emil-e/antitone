//! Scripted RAW/Metal correction, picker, preview and temporary-project checks.
//! Requires explicit negative/reference RAWs and a desktop; errors, timeout and early close fail.
//! Optional layout holds emit capture markers without accepting a map, saving or exporting.
//! Direct editor/worker actions do not establish native-dialog, physical-input or photographic acceptance.
use super::*;
pub struct FlatFieldSmoke {
    editor: Editor,
    negative: PathBuf,
    reference: PathBuf,
    phase: u8,
    start: std::time::Instant,
    hold: std::time::Instant,
    directory: tempfile::TempDir,
    before: Option<crate::project::Project>,
    saved: Option<crate::project::Project>,
    scan: Option<Scan>,
    regions: Vec<Rect>,
    anchors: Option<[(Rect, DVec3); 2]>,
    balance: Option<Balance>,
    viewport: Option<Viewport>,
    layout_capture: bool,
    layout_announced: bool,
    pub result: Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}
impl FlatFieldSmoke {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        args: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if args.len() != 2 {
            return Err("--smoke-flat-field NEGATIVE.ARW REFERENCE.ARW".into());
        }
        let negative = PathBuf::from(&args[0]);
        let reference = PathBuf::from(&args[1]);
        let mut editor = Editor::new(device, queue, None);
        editor.add_raws(vec![negative.clone()]);
        Ok(Self {
            editor,
            negative,
            reference,
            phase: 0,
            start: std::time::Instant::now(),
            hold: std::time::Instant::now(),
            directory: tempfile::tempdir()?,
            before: None,
            saved: None,
            scan: None,
            regions: Vec::new(),
            anchors: None,
            balance: None,
            viewport: None,
            layout_capture: std::env::var_os("ANTITONE_SMOKE_FLAT_LAYOUT").is_some(),
            layout_announced: false,
            result: Arc::new(std::sync::Mutex::new(None)),
        })
    }
    fn settled(&self) -> bool {
        self.editor.settled.is_some() && self.hold.elapsed() > Duration::from_millis(500)
    }
    fn next(&mut self) {
        self.phase += 1;
        self.hold = std::time::Instant::now();
    }
    fn geometry(
        &self,
        ui: &egui::Ui,
    ) -> Result<crate::viewport::CanvasLayout, Box<dyn std::error::Error>> {
        let canvas = ui.ctx().graphics(|graphics| {
            graphics.get(ui.layer_id()).and_then(|list| {
                list.all_entries()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Callback(callback) => Some((shape.clip_rect, callback.rect)),
                        _ => None,
                    })
                    .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            })
        });
        let (canvas, image) = canvas.ok_or("smoke acquired no image callback")?;
        if !canvas.expand(0.1).contains_rect(image) {
            return Err("Fit image exceeds visible canvas".into());
        }
        Ok(self
            .editor
            .last_layout
            .ok_or("missing shared canvas layout")?
            .0)
    }
    fn correction(&mut self, enabled: Option<bool>) {
        let mut document = self.editor.document.clone();
        match enabled {
            Some(value) => document.project.flat_field.as_mut().unwrap().enabled = value,
            None => document.project.flat_field = None,
        }
        self.editor.start_document(DocumentWork::Correction(
            document,
            self.editor.current_source,
        ));
    }
    fn pick(&mut self, kind: Pick, region: Rect) {
        self.editor.begin_pick(kind);
        self.editor.region = Some(Coverage::rectangle(region));
        self.editor.apply_pick();
    }
    fn calibration_regions(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut scan = Scan::open(
            &self.negative,
            Some(
                self.editor
                    .document
                    .project
                    .development
                    .as_ref()
                    .unwrap()
                    .fixed_wb,
            ),
        )?;
        scan.set_flat_field(Some(
            self.editor
                .correction
                .as_ref()
                .ok_or("missing accepted field")?
                .settings
                .map
                .clone(),
        ))?;
        let [x, y, w, h] = scan.flat_field().unwrap().area();
        let mut patches = Vec::new();
        for gy in 0..9 {
            for gx in 0..9 {
                let region = Rect {
                    x: x + 32 + gx * (w - 96) / 8,
                    y: y + 32 + gy * (h - 96) / 8,
                    width: 32,
                    height: 32,
                };
                if let Ok(rgb) = scan.reference(region) {
                    patches.push((region, rgb));
                }
            }
        }
        let dmin = *patches
            .iter()
            .max_by(|a, b| a.1.element_sum().total_cmp(&b.1.element_sum()))
            .ok_or("no usable calibration patches")?;
        let dmax = *patches
            .iter()
            .filter(|p| (dmin.1 / p.1).min_element() > 1.00001)
            .max_by(|a, b| {
                (dmin.1 / a.1)
                    .min_element()
                    .total_cmp(&(dmin.1 / b.1).min_element())
            })
            .ok_or("no ordered RGB calibration pair")?;
        self.regions = patches.into_iter().map(|p| p.0).collect();
        self.anchors = Some([dmin, dmax]);
        self.scan = Some(scan);
        Ok(())
    }
    fn advance(&mut self, ui: &egui::Ui) -> Result<(), Box<dyn std::error::Error>> {
        if self.start.elapsed() > Duration::from_secs(if self.layout_capture { 45 } else { 150 }) {
            return Err(format!(
                "flat-field smoke timed out in phase {}: {}",
                self.phase, self.editor.status
            )
            .into());
        }
        if self.editor.busy {
            return Ok(());
        }
        if self.editor.ready.is_none() {
            return Err(self.editor.status.clone().into());
        }
        match self.phase {
            0 => {
                self.before = Some(self.editor.document.project.clone());
                self.editor.begin_flat_edit(self.reference.clone(), None);
                self.next();
                if self.layout_capture {
                    self.editor.tab = 0;
                    self.editor.scopes.visible = false;
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            1100., 800.,
                        )));
                    self.phase = 20;
                }
            }
            20..=21 if self.settled() => {
                if self.editor.pick != Some(Pick::Flat) || self.editor.flat_edit.is_none() {
                    return Err(format!(
                        "flat-field layout smoke interrupted: {}",
                        self.editor.status
                    )
                    .into());
                }
                let geometry = self.geometry(ui)?;
                if !geometry.canvas.expand(0.1).contains_rect(
                    geometry.screen_coverage(self.editor.region.ok_or("missing support")?),
                ) {
                    return Err("layout capture clipped support".into());
                }
                if !self.layout_announced {
                    println!(
                        "tools_layout stage=flat-{}",
                        if self.phase == 20 { "normal" } else { "narrow" }
                    );
                    self.layout_announced = true;
                    self.hold = std::time::Instant::now();
                }
                if self.hold.elapsed() > Duration::from_millis(2500) {
                    if self.phase == 20 {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                                800., 600.,
                            )));
                        if let Some(mut panel) = egui::containers::panel::PanelState::load(
                            ui.ctx(),
                            egui::Id::new("controls"),
                        ) {
                            panel.outer_rect.min.x = panel.outer_rect.max.x - 270.;
                            ui.ctx().data_mut(|data| {
                                data.insert_persisted(egui::Id::new("controls"), panel)
                            });
                        }
                        self.editor.viewport.fit();
                        self.phase = 21;
                        self.layout_announced = false;
                        self.hold = std::time::Instant::now();
                    } else {
                        self.editor.finish_pick();
                        if self.editor.document.project != *self.before.as_ref().unwrap() {
                            return Err("layout capture cancel changed accepted project".into());
                        }
                        println!(
                            "flat_layout passed: real-reference Fit/support, normal/narrow windows, cancel preserved project; elapsed_seconds={:.2}",
                            self.start.elapsed().as_secs_f64()
                        );
                        *self.result.lock().unwrap() = Some(Ok(()));
                        self.editor.allow_close = true;
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        self.phase = 22;
                    }
                }
            }
            1 if self.settled() => {
                let geometry = self.geometry(ui)?;
                let original = self
                    .editor
                    .region
                    .ok_or("automatic support was not proposed")?;
                if !geometry
                    .canvas
                    .expand(0.1)
                    .contains_rect(geometry.screen_coverage(original))
                {
                    return Err("support rectangle exceeds visible viewer".into());
                }
                let at = geometry.screen_coverage(original).center();
                let moved = PickDrag::begin(geometry, at, Some(original))
                    .update(geometry, at + egui::vec2(geometry.scale * 2., 0.));
                if moved == original {
                    return Err("rectangle move did not change support".into());
                }
                let corner = geometry.screen_coverage(moved).right_bottom();
                let resized = PickDrag::begin(geometry, corner, Some(moved)).update(
                    geometry,
                    corner - egui::vec2(geometry.scale * 4., geometry.scale * 4.),
                );
                let moved_rect = moved
                    .exact_rectangle(geometry.composition.area())
                    .ok_or("support move is not source rectangular")?;
                let resized_rect = resized
                    .exact_rectangle(geometry.composition.area())
                    .ok_or("support resize is not source rectangular")?;
                if resized_rect.width >= moved_rect.width
                    || resized_rect.height >= moved_rect.height
                {
                    return Err("rectangle resize did not shrink support".into());
                }
                self.editor.region = Some(resized);
                self.editor.finish_pick();
                if self.editor.document.project != *self.before.as_ref().unwrap() {
                    return Err("cancel changed accepted project".into());
                }
                println!("flat_smoke reference fit/move/resize/cancel passed (scripted geometry)");
                self.editor.begin_flat_edit(self.reference.clone(), None);
                self.next();
            }
            2 if self.settled() => {
                self.geometry(ui)?;
                let area = self.editor.ready.as_ref().ok_or("missing scan")?.area;
                let mut region = self
                    .editor
                    .region
                    .ok_or("missing support")?
                    .exact_rectangle(area)
                    .ok_or("support is not source rectangular")?;
                region.width -= 2;
                region.height -= 2;
                self.editor.region = Some(Coverage::rectangle(region));
                self.editor.apply_pick();
                self.next();
            }
            3 if self.settled() => {
                if self.editor.document.project.order != self.before.as_ref().unwrap().order
                    || self.editor.dmin.is_some()
                    || self.editor.dmax.is_some()
                {
                    return Err("accept changed filmstrip/calibration".into());
                }
                self.calibration_regions()?;
                self.pick(Pick::Dmin, self.anchors.unwrap()[0].0);
                self.next();
            }
            4 => {
                if self.editor.dmin != Some(self.anchors.unwrap()[0].1) {
                    return Err(format!(
                        "corrected Dmin differs from CPU reference: {}",
                        self.editor.status
                    )
                    .into());
                }
                self.pick(Pick::Dmax, self.anchors.unwrap()[1].0);
                self.next();
            }
            5 => {
                if self.editor.dmax != Some(self.anchors.unwrap()[1].1) {
                    return Err(format!(
                        "corrected Dmax differs from CPU reference: {}",
                        self.editor.status
                    )
                    .into());
                }
                let calibration = self.editor.calibration().unwrap();
                let scan = self.scan.as_mut().unwrap();
                let (region, balance) = self
                    .regions
                    .iter()
                    .find_map(|r| {
                        scan.roll_neutral(*r, calibration, self.editor.roll)
                            .ok()
                            .map(|b| (*r, b))
                    })
                    .ok_or("no usable roll-neutral patch")?;
                self.balance = Some(balance);
                self.pick(Pick::Gamma, region);
                self.next();
            }
            6 => {
                if self.editor.roll.balance != self.balance.unwrap() {
                    return Err(
                        format!("corrected roll fit differs: {}", self.editor.status).into(),
                    );
                }
                let calibration = self.editor.calibration().unwrap();
                let scan = self.scan.as_mut().unwrap();
                let (region, balance) = self
                    .regions
                    .iter()
                    .find_map(|r| {
                        scan.photo_neutral(*r, calibration, self.editor.roll, self.editor.photo)
                            .ok()
                            .map(|b| (*r, b))
                    })
                    .ok_or("no usable photo-neutral patch")?;
                self.balance = Some(balance);
                self.pick(Pick::Wb, region);
                self.next();
            }
            7 => {
                if self.editor.photo.wb != self.balance.unwrap() {
                    return Err(
                        format!("corrected photo fit differs: {}", self.editor.status).into(),
                    );
                }
                self.editor.source = false;
                self.editor.target = OutputTarget::Srgb;
                self.next();
                println!(
                    "flat_smoke full-resolution corrected Dmin/Dmax/Gamma/WB agree exactly with CPU"
                );
            }
            8 if self.settled()
                && self
                    .editor
                    .thumbnails
                    .ready_count(&self.editor.document, self.editor.target)
                    > 0 =>
            {
                self.geometry(ui)?;
                self.editor.target = OutputTarget::DisplayP3;
                self.next();
            }
            9 if self.settled()
                && self
                    .editor
                    .thumbnails
                    .ready_count(&self.editor.document, self.editor.target)
                    > 0 =>
            {
                self.geometry(ui)?;
                self.editor.source = true;
                self.editor.source_exposure = 2.;
                self.editor.viewport.zoom = Some(1.);
                self.viewport = Some(self.editor.viewport);
                self.correction(Some(false));
                self.next();
            }
            10 => {
                if self.editor.correction.is_some()
                    || !self.editor.source
                    || self.editor.source_exposure != 2.
                    || format!("{:?}", self.editor.viewport)
                        != format!("{:?}", self.viewport.unwrap())
                {
                    return Err("Enabled changed inspection/view context".into());
                }
                self.correction(Some(true));
                self.next();
            }
            11 if self.settled() => {
                if self.editor.correction.is_none() {
                    return Err("Enabled did not restore map".into());
                }
                let (_, view, _) = self.editor.settled.as_ref().unwrap();
                if view.area.width >= self.editor.ready.as_ref().unwrap().area.width {
                    return Err(
                        "native corrected inspection did not refine a visible source region".into(),
                    );
                }
                println!(
                    "flat_smoke corrected +2EV native inspection refined source={:?} output={:?}",
                    view.area, view.output
                );
                self.editor.source_exposure = 0.;
                self.editor.source = false;
                self.editor.viewport.fit();
                self.editor.sync_document();
                self.saved = Some(self.editor.document.project.clone());
                self.editor.queue_save(
                    self.directory.path().join("roll.antitone.json"),
                    crate::project::Save::New,
                    None,
                );
                self.next();
            }
            12 => {
                if self.editor.document.dirty() {
                    return Err(self.editor.status.clone().into());
                }
                self.correction(None);
                self.next();
            }
            13 => {
                if self.editor.correction.is_some()
                    || self.editor.document.project.flat_field.is_some()
                {
                    return Err("Clear did not remove field".into());
                }
                self.editor.start_document(DocumentWork::Open(
                    self.directory.path().join("roll.antitone.json"),
                ));
                self.next();
            }
            14 if self.settled()
                && self
                    .editor
                    .thumbnails
                    .ready_count(&self.editor.document, self.editor.target)
                    > 0 =>
            {
                if self.editor.document.project != *self.saved.as_ref().unwrap()
                    || self.editor.correction.is_none()
                {
                    return Err("corrected project reopen differs".into());
                }
                self.geometry(ui)?;
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(800., 600.)));
                let [x, y, width, height] = self
                    .editor
                    .document
                    .project
                    .flat_field
                    .as_ref()
                    .unwrap()
                    .map
                    .area();
                self.editor.begin_flat_edit(
                    self.reference.clone(),
                    Some(Rect {
                        x,
                        y,
                        width,
                        height,
                    }),
                );
                self.next();
            }
            15 if self.settled() => {
                let geometry = self.geometry(ui)?;
                if !geometry.canvas.expand(0.1).contains_rect(
                    geometry.screen_coverage(self.editor.region.ok_or("missing support")?),
                ) {
                    return Err("narrow Fit clipped support".into());
                }
                self.editor.finish_pick();
                self.next();
            }
            16 if self.settled() => {
                if self.editor.document.project != *self.saved.as_ref().unwrap() {
                    return Err("Edit area cancel changed saved state".into());
                }
                println!(
                    "flat_smoke passed: accepted map, both target previews/refinement/thumbnails, Enabled/Clear, exact Save/Open, narrow Fit; elapsed_seconds={:.2}",
                    self.start.elapsed().as_secs_f64()
                );
                *self.result.lock().unwrap() = Some(Ok(()));
                self.editor.allow_close = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                self.phase = 17;
            }
            _ => {}
        }
        Ok(())
    }
}
impl host::App for FlatFieldSmoke {
    fn submission_gate(&self) -> &crate::platform::SubmissionGate {
        self.editor.submission_gate()
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.editor.ui(ui);
        if let Err(error) = self.advance(ui) {
            *self.result.lock().unwrap() = Some(Err(error.to_string()));
            self.editor.allow_close = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}
