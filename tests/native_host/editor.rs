use super::*;
use antitone::{
    editor::Editor,
    project::{Development, Project, Save, Source, SourceDevelopment, SourceId},
};
use std::path::{Path, PathBuf};

pub(super) fn fixture_project(directory: &Path) -> host::Result<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    let metadata = antitone::raw::RawFile::open(&path)?.metadata;
    let area = metadata.image_area()?;
    let mut project = Project {
        development: Some(Development::new(
            antitone::raw::version(),
            metadata.as_shot_wb(),
        )),
        input_floor: Some(antitone::processing::CAMERA_INPUT_FLOOR),
        dmin: Some([3000. / 4031.; 3]),
        dmax: Some([128. / 4031.; 3]),
        ..Default::default()
    };
    project.add_source(Source {
        id: SourceId(0),
        path,
        development: SourceDevelopment {
            lens_correction: None,
            dimensions: [metadata.width, metadata.height],
            active_area: [area.x, area.y, area.width, area.height],
            camera_model: metadata.camera_model(),
            camera_to_ap1: metadata.camera_to_ap1()?.to_cols_array_2d(),
            input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
        },
        reference_only: false,
        photo: Default::default(),
        local_adjustments: None,
        repairs: Default::default(),
        orientation: Default::default(),
        crop: None,
    })?;
    let path = directory.join("native-editor.antitone.json");
    project.save(&path, Save::New)?;
    Ok(path)
}

pub(super) struct EditorApp {
    pub editor: Editor,
    pub report: Arc<Mutex<Report>>,
    pub start: Instant,
    pub requested: bool,
    pub index: usize,
}

impl host::App for EditorApp {
    fn menu_state(&self) -> Option<antitone::platform::menu::State> {
        self.editor.menu_state()
    }

    fn menu_action(&mut self, action: antitone::platform::menu::Action) {
        self.editor.menu_action(action);
    }

    fn menu_paste_requested(&self) -> bool {
        self.editor.menu_paste_requested()
    }

    fn menu_input(&mut self, clipboard: Option<String>) -> Vec<egui::Event> {
        host::App::menu_input(&mut self.editor, clipboard)
    }

    fn native_menu_installed(&mut self) {
        self.editor.native_menu_installed();
    }

    fn quit_state(&self) -> Option<host::QuitState> {
        self.editor.quit_state()
    }

    fn request_quit(&mut self) {
        self.editor.request_quit();
    }

    fn submission_gate(&self) -> &SubmissionGate {
        self.editor.submission_gate()
    }

    fn processing_revision(&self) -> Option<(u64, u64)> {
        host::App::processing_revision(&self.editor)
    }

    fn photo_fullscreen(&self) -> bool {
        self.editor.photo_fullscreen()
    }

    fn photo_fullscreen_changed(&mut self, active: bool) {
        self.editor.photo_fullscreen_changed(active);
    }

    fn on_exit(&mut self) {
        self.editor.on_exit();
        self.report.lock().unwrap().exited = true;
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.editor.ui(ui);
        if ui.input(|input| input.viewport().close_requested()) {
            return;
        }
        if !self.requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
            // Editor requests its normal startup size; this runner owns the test sizes.
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                    SIZES[0][0],
                    SIZES[0][1],
                )));
            self.requested = true;
        }
        if self.start.elapsed() >= Duration::from_secs(10) {
            self.report.lock().unwrap().timed_out = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let (size, focused) = ui.input(|input| {
            (
                input.viewport().inner_rect.map(|r| r.size()),
                input.viewport().focused == Some(true),
            )
        });
        let mut report = self.report.lock().unwrap();
        report.focused |= focused;
        if self.index < SIZES.len()
            && size.is_some_and(|size| {
                (size - egui::vec2(SIZES[self.index][0], SIZES[self.index][1]))
                    .abs()
                    .max_elem()
                    <= 1.
            })
        {
            if report.painted[self.index] {
                self.index += 1;
                report.sizes = self.index;
                if self.index < SIZES.len() {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            SIZES[self.index][0],
                            SIZES[self.index][1],
                        )));
                }
            } else {
                // The selected-image callback is in the upper-left canvas. Exclude scopes
                // on the right and filmstrip callbacks below; acknowledge its surface pass,
                // not merely Editor UI execution or a discarded layout pass.
                let image = ui.ctx().graphics(|graphics| {
                    graphics.get(ui.layer_id()).and_then(|list| {
                        list.all_entries()
                            .filter_map(|shape| match &shape.shape {
                                egui::Shape::Callback(callback)
                                    if callback.rect.center().x < SIZES[self.index][0] / 2.
                                        && callback.rect.center().y < SIZES[self.index][1] / 2. =>
                                {
                                    Some((shape.clip_rect, callback.rect))
                                }
                                _ => None,
                            })
                            .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
                    })
                });
                if let Some((clip, image)) = image {
                    assert!(image.area() > 0.);
                    ui.painter()
                        .with_clip_rect(clip)
                        .add(egui_wgpu::Callback::new_paint_callback(
                            image,
                            PaintAck {
                                report: self.report.clone(),
                                index: self.index,
                            },
                        ));
                }
            }
        }
        if self.index == SIZES.len() && report.focused {
            report.completed = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}
