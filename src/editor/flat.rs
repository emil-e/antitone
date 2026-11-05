use super::*;
use crate::{
    cancellation::Cancellation,
    gpu::{flat::GpuFlatField, source::SourceGeometry},
    project::FlatFieldSettings,
    viewport::CanvasLayout,
};
use std::{cell::RefCell, rc::Rc};

pub(super) struct Correction {
    pub settings: FlatFieldSettings,
    pub gpu: Arc<GpuFlatField>,
    pub(super) maximum_gain: f64,
}

impl Correction {
    pub fn prepare(
        device: &wgpu::Device,
        document: &Document,
        previous: Option<&Arc<Self>>,
    ) -> Result<Option<Arc<Self>>, Error> {
        document.validate_for_device(device)?;
        let Some(settings) = document.project.active_flat_field() else {
            return Ok(None);
        };
        if let Some(previous) = previous
            && previous.settings == *settings
        {
            return Ok(Some(previous.clone()));
        }
        let gpu = Arc::new(GpuFlatField::new(device, settings.map.clone())?);
        for source in &document.project.sources {
            if document.project.order.contains(&source.id) {
                settings.require_source(&source.development)?;
                let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
                let continuous = source.development.lens_correction.is_some();
                crate::gpu::parameters::CameraMatrix::require_source(matrix, continuous)?;
                gpu.validate_matrix(
                    matrix,
                    if continuous {
                        crate::processing::MAX_SOURCE_GAIN
                    } else {
                        1.
                    },
                )?;
                SourceGeometry::new(source.development.active_area, [1, 1])?;
            }
        }
        let maximum_gain = settings.map.bounds().1.max_element();
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
            maximum_gain,
        })))
    }
}

/// Prepared reference inputs owned by the Flat interaction.
pub(super) struct FlatEdit {
    pub services: Option<source::Services>,
    pub view: super::canvas::SourceView,
    pub region: Option<Coverage>,
    pub task: Option<async_executor::Task<()>>,
}

pub(super) struct FlatPreview {
    pub ready: Ready,
    pub region: Option<Rect>,
}

impl Editor {
    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let editing = self
            .current_tool()
            .is_some_and(|tool| tool.is::<FlatTool>());
        let settings = self.document.state.project.flat_field();
        let configured = settings.is_some();
        let available = self.tool_start_available();
        let summary = match settings {
            Some(settings) if settings.enabled => "Enabled",
            Some(_) => "Disabled",
            None => "Not configured",
        };
        let (_, action) = presentation::section(
            ui,
            "roll-flat-field",
            "Flat-field correction",
            presentation::SectionHeader {
                summary,
                changed: configured,
                reset: Some(configured && !self.interaction_blocked() && !self.tool_interacting()),
                tool: None,
                default_open: if editing { None } else { Some(false) },
            },
            |ui| {
                if editing {
                    return;
                }
                ui.add_enabled_ui(available, |ui| {
                    if let Some(settings) = self.document.state.project.flat_field() {
                        let path = self.document.source_path(settings.reference).ok();
                        let mut enabled = settings.enabled;
                        if ui.add_enabled(!self.tool_interacting(), egui::Checkbox::new(&mut enabled, "Enabled")).changed() {
                            self.control_command(editing::Command::Correction(Some(enabled)), cx);
                        }
                        let name = path
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Reference RAW".into());
                        let response = ui.add(egui::Label::new(name).truncate());
                        if let Some(path) = path {
                            response.on_hover_text(path.display().to_string());
                        }
                    } else if presentation::TextAction::new("Set up…")
                        .help("Choose a film-free RAW of the light source from the same scanning setup.")
                        .show(ui)
                        .clicked()
                        && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                            rfd::FileDialog::new()
                                .set_title("Bare light · same lens, focus, aperture and light · without film")
                                .add_filter("Camera RAW files", files::RAW_EXTENSIONS)
                                .pick_file()
                        })
                    {
                        self.control_command(editing::Command::FlatEdit(path, None), cx);
                    }
                });
            },
        );
        if action.reset {
            self.control_command(editing::Command::Correction(None), cx);
        }
    }
}

pub(super) struct FlatTool {
    state: Rc<RefCell<FlatEdit>>,
    viewport: Viewport,
    fullscreen_viewport: Option<Viewport>,
    drag: Option<PickDrag>,
}

impl FlatTool {
    pub(super) fn open(
        editor: &mut Editor,
        path: PathBuf,
        region: Option<Rect>,
        cx: &UiContext<'_, Editor>,
    ) {
        if !editor.prepare_tool() {
            return;
        }
        editor.sync_document();
        editor.document.thumbnails.cancel();
        let activity = editor
            .activities
            .start(ActivityOptions::new("Preparing reference…").modal());
        let context = FlatEdit {
            services: editor.source_services.clone(),
            view: Default::default(),
            region: region.map(Coverage::rectangle),
            task: None,
        };
        let tool = Self::new(context);

        editor.status = "Preparing uncorrected flat-field reference…".into();

        let request = Self::preview_input(editor, path);
        let task = setup(
            request,
            editor.source_services.clone(),
            std::rc::Rc::downgrade(&tool.state),
            activity,
            cx,
        );
        if let Some(task) = task {
            tool.state.borrow_mut().task = Some(task);
            tool.install(editor);
        } else {
            editor.status = "Application stopped".into();
            editor.report_failure(FailureKind::FlatReference, editor.status.clone());
        }
    }

    fn preview_input(editor: &Editor, path: PathBuf) -> source::Request {
        let expected = editor
            .document
            .find_source(&path)
            .and_then(|id| editor.document.state.project.source(id).ok())
            .cloned();
        // A retained record is either an ordered photo or the current flat-field reference;
        // both belong to the current stage. Use its saved decode identity when present.
        let wb = editor
            .document
            .state
            .project
            .acquisition_wb(expected.as_ref());
        let file = editor.document.file_key(&path);
        source::Request {
            reference: expected.as_ref().map_or_else(Arc::default, |source| {
                editor
                    .document
                    .exposure_slot(&editor.document.exposures, source.id)
            }),
            input: editor.input(),
            path,
            wb,
            expected,
            file,
            sources: editor.source_provider(),
            correction: None,
        }
    }

    fn new(state: FlatEdit) -> Self {
        Self {
            state: Rc::new(RefCell::new(state)),
            viewport: Default::default(),
            fullscreen_viewport: None,
            drag: None,
        }
    }

    fn install(self, editor: &mut Editor) {
        editor
            .document
            .set_hosted_tool(tool::Placement::Document, Box::new(self));
    }
}

impl tool::Tool for FlatTool {
    fn presentation(&self, editor: &Editor) -> tool::Presentation {
        let state = self.state.borrow();
        tool::Presentation {
            identity: presentation::ToolIdentity {
                icon: icons::Icon::Crop,
                title: "Flat-field",
            },
            primary: Some(tool::Primary {
                label: "Apply",
                enabled: state.view.ready.is_some()
                    && state.region.is_some()
                    && !editor.interaction_blocked(),
            }),
            dismiss: "Cancel",
            warning: None,
            help: "Select the illuminated area, excluding the holder and including the image centre.",
            navigation_help: None,
            fit_label: "Fit",
        }
    }

    fn viewport<'a>(&'a mut self, editor: &'a mut Editor) -> &'a mut Viewport {
        if editor.fullscreen.is_some() {
            self.fullscreen_viewport.get_or_insert(self.viewport)
        } else {
            self.fullscreen_viewport = None;
            &mut self.viewport
        }
    }

    fn filmstrip(&mut self, ui: &mut egui::Ui, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        editor.filmstrip_readonly(ui, cx);
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui, _editor: &mut Editor) {
        ui.input_mut(|input| {
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
            input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
        });
    }

    fn drawer(&mut self, ui: &mut egui::Ui, _editor: &mut Editor, _cx: &UiContext<'_, Editor>) {
        presentation::ToolIdentity { icon: icons::Icon::Crop, title: "Flat-field" }.inspector(ui, |_| false, |ui, _| {
            if let Some(ready) = self.state.borrow().view.ready.as_ref() {
                ui.add(egui::Label::new(ready.path.file_name().unwrap_or_default().to_string_lossy()).truncate()).on_hover_text(ready.path.display().to_string());
            }
            ui.small("Select the illuminated area, excluding the holder and including the image centre.");
            ui.small("Dmin/Dmax stay unchanged; re-pick them after applying correction.");
        });
    }

    fn canvas(
        &mut self,
        viewer: &mut canvas::Viewer<'_>,
        editor: &mut Editor,
        cx: &UiContext<'_, Editor>,
    ) -> Option<CanvasLayout> {
        viewer.show(
            self,
            editor,
            cx,
            Editor::source_controls,
            |tool, surface, editor, cx| {
                let (area, proxy) = tool
                    .state
                    .borrow()
                    .view
                    .ready
                    .as_ref()
                    .map(|ready| (ready.area, ready.proxy.clone()))?;
                let composition = Composition::new(area, Default::default(), Default::default())
                    .expect("prepared source area");
                let mut canvas = surface.image(composition, proxy);
                let rect = canvas.response.rect;
                let ppp = canvas.ui.ctx().pixels_per_point();
                let relayout =
                    |viewport: &mut Viewport| viewport.layout_composition(rect, composition, ppp);
                let mut response = canvas.response.clone();
                if editor.fullscreen.is_none() {
                    navigation::consume_primary(&mut response);
                }
                let viewport = tool.viewport(editor);
                let layout = canvas.layout(viewport, relayout);
                let (layout, panning) =
                    navigation::navigate_view(viewport, canvas.ui, &response, layout, relayout);
                let services = tool.state.borrow().services.clone();
                tool.state.borrow_mut().view.paint(
                    &mut canvas,
                    layout,
                    editor,
                    services.as_ref(),
                    cx,
                );
                let selecting = editor.fullscreen.is_none()
                    && !editor.interaction_blocked()
                    && canvas.ui.is_enabled()
                    && !panning
                    && !canvas.ui.input(|input| input.key_down(egui::Key::Space));
                if selecting {
                    let mut state = tool.state.borrow_mut();
                    if canvas.response.clicked_by(egui::PointerButton::Primary)
                        && let Some(point) = canvas
                            .response
                            .interact_pointer_pos()
                            .filter(|point| layout.image.contains(*point))
                    {
                        state.region = Some(layout.pick_patch(point, 32));
                    }
                    if canvas
                        .response
                        .drag_started_by(egui::PointerButton::Primary)
                    {
                        tool.drag = canvas
                            .ui
                            .input(|input| input.pointer.press_origin())
                            .filter(|point| layout.image.contains(*point))
                            .map(|point| PickDrag::begin(layout, point, state.region));
                    }
                    if canvas.response.dragged_by(egui::PointerButton::Primary)
                        && let (Some(drag), Some(point)) =
                            (tool.drag, canvas.response.interact_pointer_pos())
                    {
                        state.region = Some(drag.update(layout, point));
                    }
                    if (canvas.response.hovered() || canvas.response.dragged())
                        && let Some(point) = canvas.ui.input(|input| input.pointer.hover_pos())
                    {
                        canvas.ui.ctx().set_cursor_icon(navigation::picker_cursor(
                            layout,
                            point,
                            state.region,
                            tool.drag,
                            canvas.response.dragged_by(egui::PointerButton::Primary),
                        ));
                    }
                }
                if !canvas
                    .ui
                    .input(|input| input.pointer.button_down(egui::PointerButton::Primary))
                {
                    tool.drag = None;
                }
                if editor.fullscreen.is_none()
                    && let Some(region) = tool.state.borrow().region
                {
                    picking::paint_selection(canvas.ui, layout, region);
                }
                Some(layout)
            },
        )
    }

    fn action(&mut self, action: tool::Action, editor: &mut Editor, cx: &UiContext<'_, Editor>) {
        match action {
            tool::Action::Primary
                if !editor.interaction_blocked() && editor.document_pending.is_none() =>
            {
                editor.sync_document();
                let state = self.state.borrow();
                let Some(reference) = &state.view.ready else {
                    return;
                };
                let Some(coverage) = state.region else { return };
                let Some(region) = coverage.exact_rectangle(reference.area) else {
                    editor.status = "Flat-field support must be an integer source rectangle".into();
                    editor.report_failure(FailureKind::FlatSupport, editor.status.clone());
                    return;
                };
                let input = CorrectionInput {
                    services: state.services.clone(),
                    sources: state
                        .services
                        .as_ref()
                        .map(|services| services.provider(Some(editor.document.cache.clone()))),
                    region,
                    document: editor.document.capture(),
                    selected: editor.document.visit.current_source,
                    source: source::Context {
                        source: reference.developed.clone(),
                        reference: reference.reference.clone(),
                        correction: None,
                    },
                    record: reference.record.clone(),
                    correction: editor.document.correction.clone(),
                };
                correct(editor, input, cx);
            }
            tool::Action::Dismiss => {
                editor.status = "Flat-field setup cancelled; correction unchanged".into();
                editor.end_tool();
            }
            _ => {}
        }
    }
}

fn setup<'scope>(
    request: source::Request,
    services: Option<source::Services>,
    owner: std::rc::Weak<std::cell::RefCell<FlatEdit>>,
    activity: Activity,
    cx: &UiContext<'scope, Editor>,
) -> Option<async_executor::Task<()>> {
    let cancelled = Cancellation::default();
    let cancel_when_dropped = cancelled.cancel_on_drop();
    cx.spawn(move |cx| async move {
        let _cancel_when_dropped = cancel_when_dropped;
        let reference = async {
            let services = services
                .as_ref()
                .ok_or_else(|| "Reference services unavailable".to_owned())?;
            preview(services.clone(), request, cancelled.clone()).await
        }
        .await;
        drop(activity);
        let Some(owner) = owner.upgrade() else { return };
        let reference = match reference {
            Ok(reference) => reference,
            Err(error) => {
                fail(&cx, FailureKind::FlatReference, error);
                return;
            }
        };
        cx.update(|editor| {
            let mut edit = owner.borrow_mut();
            edit.region = edit.region.or(reference.region.map(Coverage::rectangle));
            edit.view.ready = Some(reference.ready.clone());

            editor.resolve_failure(FailureKind::FlatReference);
            editor.status = if edit.region.is_some() {
                "Proposed illuminated rectangle: adjust edges, then Apply."
            } else {
                "Illuminated area is ambiguous; draw a rectangle around the usable light."
            }
            .into();
        });
    })
}

fn correct(editor: &mut Editor, input: CorrectionInput, cx: &UiContext<'_, Editor>) {
    let before = editor.document.state.project.clone();

    editor.end_tool();
    editor.cancel_spatial();
    editor.finish_gamma_edit();
    editor.control_commands.clear();
    let id = editor.begin_document_operation(false);
    editor.history_pending = Some(crate::editor::history::Pending::Edit {
        before: Box::new(before),
        label: "Correction",
        photo: None,
    });
    editor.document_failure = Some(FailureKind::FlatCorrection);
    let activity = editor
        .activities
        .start(ActivityOptions::new("Applying correction…").modal());
    let services = input.services.clone();
    let cancelled = Cancellation::default();
    let retire = cancelled.cancel_on_drop();
    let Some(task) = cx.spawn(move |cx| async move {
        let _retire = retire;
        let result = async {
            let services = services.ok_or_else(|| "Correction services unavailable".to_owned())?;
            let request = estimate(input, cancelled.clone()).await?;
            crate::editor::document::PreparationServices(services)
                .prepare(request, cancelled, |_, _| {})
                .await
        }
        .await;
        drop(activity);
        cx.update(|editor| {
            if editor.document_pending == Some(id) {
                editor.document.flat_correction = None;
                editor.document_prepared(id, result.map(Box::new));
                editor.acquire_working(&cx);
            }
        });
    }) else {
        editor.document_prepared(id, Err("Application stopped".into()));
        return;
    };
    editor.document.flat_correction = Some(task);
}

fn fail(cx: &UiContext<'_, Editor>, kind: FailureKind, error: String) {
    cx.update(|editor| {
        editor.end_tool();
        editor.status = error.clone();
        editor.report_failure(kind, error);
    });
}

pub(super) struct CorrectionInput {
    pub services: Option<source::Services>,
    pub region: Rect,
    pub sources: Option<Arc<dyn crate::source::SourceProvider>>,
    pub document: Document,
    pub selected: Option<SourceId>,
    pub source: source::Context,
    pub record: Source,
    pub correction: Option<Arc<crate::editor::Correction>>,
}

impl CorrectionInput {
    fn estimate(mut self) -> Result<document::Request, Error> {
        if self.source.correction.is_some() {
            return Err(Error::InvalidParameter(
                "flat-field reference must be uncorrected",
            ));
        }
        let mut scan = self.source.scan()?;
        let map = Arc::new(scan.estimate_flat_field(self.region)?);
        let reference = self.document.register_reference(self.record)?;
        let enabled = self
            .document
            .project
            .flat_field()
            .is_none_or(|field| field.enabled);
        let mut settings =
            crate::project::FlatFieldSettings::new(self.document.project.source(reference)?, map);
        settings.enabled = enabled;
        self.document.project.set_flat_field(Some(settings));
        let exposures = Arc::new(crate::exposure_metadata::Store::restored(
            &self.document.exposure_metadata,
            |id| self.document.exposure_origin(id),
        ));
        Ok(document::Request {
            work: document::Preparation::Correction(self.document, self.selected),
            source: None,
            sources: self.sources,
            exposures: Some(exposures),
            correction: self.correction,
        })
    }
}

async fn estimate(
    input: CorrectionInput,
    cancelled: Cancellation,
) -> Result<document::Request, String> {
    crate::background::run(move |_| {
        if cancelled.is_cancelled() {
            return Err(Error::UnusableSample("correction retired"));
        }
        input.estimate()
    })
    .await
    .map_err(|_| "Reference estimation worker panicked".to_owned())?
    .map_err(|error| error.to_string())
}

async fn preview(
    services: source::Services,
    request: source::Request,
    cancelled: Cancellation,
) -> Result<FlatPreview, String> {
    if request.correction.is_some() {
        return Err("Flat-field reference must be uncorrected".into());
    }
    let ready = services.prepare(request, cancelled.clone()).await?;
    crate::background::run(move |_| {
        if cancelled.is_cancelled() {
            return Err(Error::UnusableSample("reference retired"));
        }
        let scan = source::Context {
            source: ready.developed.clone(),
            reference: ready.reference.clone(),
            correction: None,
        }
        .scan()?;
        let region = scan.propose_flat_field_area()?;
        Ok(FlatPreview { ready, region })
    })
    .await
    .map_err(|_| "Reference support worker panicked".to_owned())?
    .map_err(|error: Error| error.to_string())
}

#[cfg(test)]
mod operation_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;
