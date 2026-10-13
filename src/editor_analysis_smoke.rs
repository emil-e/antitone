//! Scripted RAW/Metal area and atomic Analyze checks, optionally with a correction reference.
//! Requires explicit source RAWs and a desktop; errors, timeout and early close fail.
//! Exercises production workers and settled previews, without saving grades or editing sources.
//! Direct editor actions do not establish widget/physical-input or photographic acceptance.
use super::*;

pub struct AnalysisSmoke {
    editor: Editor,
    reference: Option<PathBuf>,
    needs_correction: bool,
    phase: u8,
    started: std::time::Instant,
    hold: std::time::Instant,
    before: Option<crate::project::Project>,
    pub result: Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}
impl AnalysisSmoke {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        args: &[String],
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if args.len() < 2 {
            return Err("--smoke-analysis REFERENCE_OR_- RAW...".into());
        }
        let mut editor = Editor::new(device, queue, None);
        editor.add_raws(args[1..].iter().map(PathBuf::from).collect());
        Ok(Self {
            editor,
            needs_correction: args[0] != "-",
            reference: (args[0] != "-").then(|| PathBuf::from(&args[0])),
            phase: 0,
            started: std::time::Instant::now(),
            hold: std::time::Instant::now(),
            before: None,
            result: Arc::new(std::sync::Mutex::new(None)),
        })
    }
    fn finish(&mut self, ui: &egui::Ui, result: Result<(), String>) {
        *self.result.lock().unwrap() = Some(result);
        self.editor.allow_close = true;
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
    }
}
impl host::App for AnalysisSmoke {
    fn submission_gate(&self) -> &crate::platform::SubmissionGate {
        self.editor.submission_gate()
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.editor.ui(ui);
        ui.ctx().request_repaint_after(Duration::from_millis(50));
        if self.started.elapsed() > Duration::from_secs(180) {
            self.finish(
                ui,
                Err(format!("analysis smoke timed out: {}", self.editor.status)),
            );
            return;
        }
        if self.editor.busy {
            return;
        }
        if self.editor.ready.is_none() {
            self.finish(ui, Err(self.editor.status.clone()));
            return;
        }
        match self.phase {
            0 => {
                if let Some(path) = self.reference.take() {
                    self.editor.begin_flat_edit(path, None);
                    self.phase = 1;
                } else {
                    self.phase = 2;
                }
            }
            1 => {
                if self.editor.region.is_none() {
                    self.finish(ui, Err(self.editor.status.clone()));
                    return;
                }
                self.editor.apply_pick();
                self.phase = 2;
            }
            2 => {
                if self.needs_correction && self.editor.correction.is_none() {
                    self.finish(ui, Err(self.editor.status.clone()));
                    return;
                }
                self.before = Some(self.editor.document.project.clone());
                let Some(picture) = self.editor.analysis_picture() else {
                    self.editor.begin_analysis();
                    self.phase = 4;
                    return;
                };
                self.editor.begin_pick(Pick::Analysis);
                self.editor.region = Some(Coverage::rectangle(Rect {
                    width: picture.width - 16,
                    ..picture
                }));
                self.hold = std::time::Instant::now();
                self.phase = 3;
            }
            3 if self.hold.elapsed() >= Duration::from_secs(1) => {
                self.editor.apply_pick();
                if self.editor.pick.is_some() {
                    self.finish(ui, Err(self.editor.status.clone()));
                    return;
                }
                let before = self.before.as_ref().unwrap();
                assert_eq!(self.editor.roll, before.roll);
                assert_eq!(self.editor.document.project.dmin, before.dmin);
                assert_eq!(self.editor.document.project.dmax, before.dmax);
                self.editor.begin_analysis();
                self.phase = 4;
            }
            4 => {
                if !self.editor.status.starts_with("Analyzed ") {
                    self.finish(ui, Err(self.editor.status.clone()));
                    return;
                }
                let before = self.before.as_ref().unwrap();
                assert_eq!(self.editor.roll.gamma, before.roll.gamma);
                assert_eq!(self.editor.roll.response, before.roll.response);
                for (mut after, prior) in self
                    .editor
                    .document
                    .project
                    .sources
                    .clone()
                    .into_iter()
                    .zip(&before.sources)
                {
                    assert_eq!(after.photo.wb, Balance::default());
                    after.photo = prior.photo;
                    assert_eq!(&after, prior);
                }
                self.editor.target = OutputTarget::Srgb;
                self.hold = std::time::Instant::now();
                self.phase = 5;
            }
            5 if self.editor.settled.is_some() && self.hold.elapsed() >= Duration::from_secs(2) => {
                self.editor.target = OutputTarget::DisplayP3;
                self.hold = std::time::Instant::now();
                self.phase = 6;
            }
            6 if self.editor.settled.is_some() && self.hold.elapsed() >= Duration::from_secs(2) => {
                println!(
                    "analysis onscreen: {} frames, editable statistics area, atomic install, both settled targets; Dmin={:?} Dmax={:?} balance={:?} EV={}",
                    self.editor.document.project.order.len(),
                    self.editor.dmin,
                    self.editor.dmax,
                    self.editor.roll.balance,
                    self.editor.roll.exposure
                );
                self.finish(ui, Ok(()));
                self.phase = 7;
            }
            _ => {}
        }
    }
}
