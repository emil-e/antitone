use super::*;
use crate::{
    cancellation::Cancellation,
    gpu::{flat::GpuFlatField, source::SourceGeometry},
    project::FlatFieldSettings,
    viewport::CanvasLayout,
};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::{cell::RefCell, rc::Rc};

struct Change {
    edit: editing::DocumentEdit,
    device: wgpu::Device,
    previous: Option<Arc<Correction>>,
    activity: Activity,
}

type PreparedCorrection = Result<Option<Arc<Correction>>, String>;

/// Captured readers share preparation and its result, independently of the live UI waiter.
#[derive(Clone)]
pub(super) enum Resources {
    Ready(PreparedCorrection),
    Preparing(Shared<BoxFuture<'static, PreparedCorrection>>),
}

impl Default for Resources {
    fn default() -> Self {
        Self::Ready(Ok(None))
    }
}

impl From<Option<Arc<Correction>>> for Resources {
    fn from(correction: Option<Arc<Correction>>) -> Self {
        Self::Ready(Ok(correction))
    }
}

impl Resources {
    fn prepare(document: Document, device: Option<wgpu::Device>) -> Self {
        Self::Preparing(
            async move {
                let device = device.ok_or_else(|| "Correction resources unavailable".to_owned())?;
                crate::background::run(move |_| Correction::prepare(&device, &document, None))
                    .await
                    .map_err(|_| "Correction preparation worker panicked".to_owned())?
                    .map_err(|error| error.to_string())
            }
            .boxed()
            .shared(),
        )
    }

    fn result(&self) -> Option<&PreparedCorrection> {
        match self {
            Self::Ready(result) => Some(result),
            Self::Preparing(preparation) => preparation.peek(),
        }
    }

    pub(super) fn prepared(&self) -> Option<&Arc<Correction>> {
        self.result()?.as_ref().ok()?.as_ref()
    }

    pub(super) fn error(&self) -> Option<&str> {
        self.result()?.as_ref().err().map(String::as_str)
    }

    pub(super) fn pending(&self) -> bool {
        self.result().is_none()
    }

    pub(super) async fn acquire(self) -> PreparedCorrection {
        match self {
            Self::Ready(result) => result,
            Self::Preparing(preparation) => preparation.await,
        }
    }
}

pub(super) struct Correction {
    pub settings: FlatFieldSettings,
    pub gpu: Arc<GpuFlatField>,
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
                let matrix = glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1);
                crate::gpu::parameters::CameraMatrix::require_source(matrix, true)?;
                gpu.validate_matrix(matrix, crate::processing::MAX_SOURCE_GAIN)?;
                SourceGeometry::new(source.development.active_area, [1, 1])?;
            }
        }
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
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
    pub(super) fn restore_correction(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.project.active_flat_field().is_none() {
            return;
        }
        let resources = Resources::prepare(
            self.document.capture(),
            self.document.visit.working_view.device().cloned(),
        );
        self.document.correction = resources.clone();
        let activity = self
            .activities
            .start(ActivityOptions::new("Preparing correction…"));
        // Only this presentation continuation belongs to the document's live UI.
        self.document.correction_restore = cx.spawn(move |cx| async move {
            let result = resources.acquire().await;
            drop(activity);
            cx.update(|editor| {
                editor.document.correction_restore = None;
                match result {
                    Ok(_) => {
                        editor.observe_gamma_pivot();
                        editor.acquire_working(&cx);
                    }
                    Err(_) => {
                        editor.document.source = true;
                        editor
                            .preferences
                            .sections
                            .insert("roll-flat-field".into(), true);
                    }
                }
            });
        });
    }

    fn change_correction(&mut self, enabled: Option<bool>, cx: &UiContext<'_, Self>) {
        self.cancel_control_edit();
        let mut candidate = self.document.capture();
        if let Some(enabled) = enabled {
            candidate
                .project
                .flat_field_mut()
                .expect("accepted correction")
                .enabled = enabled;
        } else {
            candidate.project.set_flat_field(None);
        }
        self.start_correction(candidate, cx);
    }

    pub(super) fn start_correction(&mut self, candidate: Document, cx: &UiContext<'_, Self>) {
        let change = match Change::begin(self) {
            Ok(change) => change,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        self.document.flat_correction = cx.spawn(move |cx| async move {
            let result = change.prepare(candidate).await;
            change.publish(result, &cx);
        });
    }

    pub(super) fn flat_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let editing = self
            .current_tool()
            .is_some_and(|tool| tool.is::<FlatTool>());
        let settings = self.document.state.project.flat_field();
        let configured = settings.is_some();
        let available = self.tool_start_available();
        let summary = match settings {
            Some(settings) if settings.enabled && self.document.correction.error().is_some() => {
                "Unavailable"
            }
            Some(settings) if settings.enabled && self.document.correction.pending() => {
                "Preparing…"
            }
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
                if let Some(error) = self.document.correction.error() {
                    let status = presentation::ContextualStatus::new(
                        presentation::StatusLevel::Error,
                        error,
                    );
                    ui.horizontal(|ui| {
                        let (_, response) = ui.allocate_exact_size(
                            egui::Vec2::splat(presentation::INSPECTOR_ROW_HEIGHT),
                            egui::Sense::hover(),
                        );
                        presentation::status_icon(
                            ui,
                            response,
                            "Correction error",
                            status.level,
                            &status.reason,
                        );
                        ui.add(egui::Label::new(&status.reason).wrap());
                    });
                }
                ui.add_enabled_ui(available, |ui| {
                    if let Some(settings) = self.document.state.project.flat_field() {
                        let path = self.document.source_path(settings.reference).ok();
                        let mut enabled = settings.enabled;
                        if ui.add_enabled(!self.tool_interacting(), egui::Checkbox::new(&mut enabled, "Enabled")).changed() {
                            self.change_correction(Some(enabled), cx);
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
                        self.cancel_control_edit();
                        FlatTool::open(self, path, None, cx);
                    }
                });
            },
        );
        if action.reset {
            self.change_correction(None, cx);
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
            editor.status = "Preparing uncorrected flat-field reference…".into();
            tool.install(editor);
        }
    }

    fn preview_input(editor: &Editor, path: PathBuf) -> source::Request {
        let expected = editor
            .document
            .find_source(&path)
            .and_then(|id| editor.document.state.project.source(id).ok())
            .cloned();
        // A retained record is either an ordered photo or the current flat-field reference;
        // Both belong to the current stage; reuse already-resolved header gains when available.
        let wb = expected
            .as_ref()
            .and_then(|source| editor.document.source_wb(source.id))
            .or_else(|| editor.document.project.acquisition_wb());
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
                let state = self.state.borrow();
                let Some(reference) = &state.view.ready else {
                    return;
                };
                let Some(coverage) = state.region else { return };
                let Some(region) = coverage.exact_rectangle(reference.area) else {
                    editor.status = "Flat-field support must be an integer source rectangle".into();
                    editor.support_failed(editor.status.clone());
                    return;
                };
                let change = match Change::begin(editor) {
                    Ok(change) => change,
                    Err(error) => {
                        editor.status = error.to_string();
                        return;
                    }
                };
                let input = CorrectionInput {
                    region,
                    document: change.edit.initial().as_ref().clone(),
                    source: source::Context {
                        source: reference.developed.clone(),
                        reference: reference.reference.clone(),
                        correction: None,
                    },
                    record: reference.record.clone(),
                };
                editor.end_tool();
                editor.document.flat_correction = cx.spawn(move |cx| async move {
                    let result = async {
                        let document = input.estimate().await?;
                        change.prepare(document).await
                    }
                    .await;
                    change.publish(result, &cx);
                });
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
                fail(&cx, error);
                return;
            }
        };
        cx.update(|editor| {
            let mut edit = owner.borrow_mut();
            edit.region = edit.region.or(reference.region.map(Coverage::rectangle));
            edit.view.ready = Some(reference.ready.clone());

            editor.status = if edit.region.is_some() {
                "Proposed illuminated rectangle: adjust edges, then Apply."
            } else {
                "Illuminated area is ambiguous; draw a rectangle around the usable light."
            }
            .into();
        });
    })
}

impl Change {
    fn begin(editor: &mut Editor) -> Result<Self, Error> {
        let edit = editor.edit_document("Correction")?;
        let device = editor
            .document
            .visit
            .working_view
            .device()
            .expect("correction requires the editor GPU")
            .clone();
        Ok(Self {
            edit,
            device,
            previous: editor.document.correction.prepared().cloned(),
            activity: editor
                .activities
                .start(ActivityOptions::new("Applying correction…").modal()),
        })
    }

    async fn prepare(
        &self,
        document: Document,
    ) -> Result<(Document, Option<Arc<Correction>>), String> {
        let device = self.device.clone();
        let previous = self.previous.clone();
        crate::background::run(move |cancelled| {
            if cancelled.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let correction = Correction::prepare(&device, &document, previous.as_ref())?;
            Ok((document, correction))
        })
        .await
        .map_err(|_| "Correction preparation worker panicked".to_owned())?
        .map_err(|error: Error| error.to_string())
    }

    fn publish(
        mut self,
        result: Result<(Document, Option<Arc<Correction>>), String>,
        cx: &UiContext<'_, Editor>,
    ) {
        drop(self.activity);
        cx.update(|editor| {
            if !editor.owns_document_edit(&self.edit) {
                return;
            }
            editor.document.flat_correction = None;
            match result {
                Ok((mut document, correction)) => {
                    document.capture_exposure(&editor.document.exposures);
                    self.edit.replace_snapshot(Arc::new(document));
                    editor.commit_document(self.edit).expect("current document edit");
                    editor.document.correction = correction.into();
                    editor.observe_gamma_pivot();
                    editor.status = "Project ready. Flat-field changes keep Dmin/Dmax RGB; re-pick calibration for this setup.".into();
                    editor.acquire_working(cx);
                }
                Err(error) => {
                    editor.status = error.clone();
                    editor.correction_failed(error);
                }
            }
        });
    }
}

fn fail(cx: &UiContext<'_, Editor>, error: String) {
    cx.update(|editor| {
        editor.end_tool();
        editor.status = error.clone();
        editor.reference_failed(error);
    });
}

pub(super) struct CorrectionInput {
    pub region: Rect,
    pub document: Document,
    pub source: source::Context,
    pub record: Source,
}

impl CorrectionInput {
    async fn estimate(mut self) -> Result<Document, String> {
        crate::background::run(move |cancelled| {
            if cancelled.is_cancelled() {
                return Err(Error::UnusableSample("correction retired"));
            }
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
            let mut settings = crate::project::FlatFieldSettings::new(
                self.document.project.source(reference)?,
                map,
            );
            settings.enabled = enabled;
            self.document.project.set_flat_field(Some(settings));
            Ok(self.document)
        })
        .await
        .map_err(|_| "Reference estimation worker panicked".to_owned())?
        .map_err(|error: Error| error.to_string())
    }
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
mod resources_tests;
#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

impl Editor {
    pub(super) fn reference_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not prepare reference",
            explanation: "Your existing correction and photographic settings are unchanged. Check the reference file and try again.",
            details,
        }));
    }
    pub(super) fn support_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Warning,
            title: "Area not accepted",
            explanation: "No correction has been applied. The current area and accepted settings are unchanged. Dismiss this message to adjust the rectangle.",
            details,
        }));
    }
    pub(super) fn correction_failed(&mut self, details: String) {
        drop(self.dialogs.report(dialogs::Message {
            severity: super::presentation::Severity::Error,
            title: "Could not change correction",
            explanation: "The correction change was not applied. Your existing correction, calibration and Photo layers are unchanged. Review the details before trying again.",
            details,
        }));
    }
}
