use super::*;
use crate::{processing::InputSettings, project::Project};
use std::{
    cell::RefCell,
    future::Future,
    ops::{Deref, DerefMut},
    pin::Pin,
    rc::{Rc, Weak},
    sync::Arc,
};

#[cfg(test)]
mod tests;

/// An immutable model version shared by accepted state, readers and undo history.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::editor) struct ProjectVersion(Arc<Project>);

impl ProjectVersion {
    pub(in crate::editor) fn snapshot(&self) -> Arc<Project> {
        self.0.clone()
    }
}

impl Default for ProjectVersion {
    fn default() -> Self {
        Project::default().into()
    }
}

impl serde::Serialize for ProjectVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_ref().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ProjectVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Project::deserialize(deserializer).map(Into::into)
    }
}

impl From<Project> for ProjectVersion {
    fn from(project: Project) -> Self {
        Self(Arc::new(project))
    }
}

impl From<Arc<Project>> for ProjectVersion {
    fn from(project: Arc<Project>) -> Self {
        Self(project)
    }
}

impl Deref for ProjectVersion {
    type Target = Project;
    fn deref(&self) -> &Project {
        &self.0
    }
}

impl DerefMut for ProjectVersion {
    fn deref_mut(&mut self) -> &mut Project {
        Arc::make_mut(&mut self.0)
    }
}

impl AsRef<Project> for ProjectVersion {
    fn as_ref(&self) -> &Project {
        self
    }
}

impl PartialEq<Project> for ProjectVersion {
    fn eq(&self, other: &Project) -> bool {
        self.as_ref() == other
    }
}

impl PartialEq<ProjectVersion> for Project {
    fn eq(&self, other: &ProjectVersion) -> bool {
        self == other.as_ref()
    }
}

/// The scoped owner of a working version. Readers retain data, not editing authority.
pub(in crate::editor) struct Edit<T> {
    values: Rc<RefCell<Arc<T>>>,
    before: Arc<T>,
    label: &'static str,
    photo: Option<crate::project::SourceId>,
}

pub(in crate::editor) struct Preview<T> {
    values: Weak<RefCell<Arc<T>>>,
}

impl<T> Default for Preview<T> {
    fn default() -> Self {
        Self {
            values: Weak::new(),
        }
    }
}

impl<T> Clone for Preview<T> {
    fn clone(&self) -> Self {
        Self {
            values: self.values.clone(),
        }
    }
}

impl<T> Edit<T> {
    pub(in crate::editor) fn from_snapshot(
        values: Arc<T>,
        label: &'static str,
        photo: Option<crate::project::SourceId>,
    ) -> Self {
        Self {
            before: values.clone(),
            values: Rc::new(RefCell::new(values)),
            label,
            photo,
        }
    }

    pub(in crate::editor) fn initial(&self) -> Arc<T> {
        self.before.clone()
    }
    pub(in crate::editor) fn preview(&self) -> Preview<T> {
        Preview {
            values: Rc::downgrade(&self.values),
        }
    }
    pub(in crate::editor) fn rename(&mut self, label: &'static str) {
        self.label = label;
    }
    pub(super) fn description(&self) -> (&'static str, Option<crate::project::SourceId>) {
        (self.label, self.photo)
    }
    pub(in crate::editor) fn replace_snapshot(&mut self, values: Arc<T>) {
        *self.values.borrow_mut() = values;
    }
    pub(super) fn into_snapshot(self) -> Arc<T> {
        self.values.borrow().clone()
    }

    #[cfg(test)]
    pub(in crate::editor) fn replace(&mut self, values: T) {
        self.replace_snapshot(Arc::new(values));
    }
}

impl<T: Clone> Edit<T> {
    pub(in crate::editor) fn update<R>(&mut self, update: impl FnOnce(&mut T) -> R) -> R {
        update(Arc::make_mut(&mut self.values.borrow_mut()))
    }

    #[cfg(test)]
    pub(in crate::editor) fn values(&self) -> T {
        self.values.borrow().as_ref().clone()
    }
}

impl<T> Preview<T> {
    pub(in crate::editor) fn active(&self) -> bool {
        self.values.strong_count() != 0
    }
    pub(in crate::editor) fn same_edit(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.values, &other.values)
    }
    pub(in crate::editor) fn snapshot(&self) -> Option<Arc<T>> {
        self.values.upgrade().map(|values| values.borrow().clone())
    }
}

pub(in crate::editor) type ProjectEdit = Edit<Project>;
pub(in crate::editor) type DocumentEdit = Edit<Document>;

impl Editor {
    pub(in crate::editor) fn edit_document(
        &mut self,
        label: &'static str,
    ) -> Result<DocumentEdit, Error> {
        if self.editing() {
            return Err(Error::InvalidParameter("finish or cancel the current edit"));
        }
        let edit = Edit::from_snapshot(Arc::new(self.document.capture()), label, None);
        self.document.document_preview = edit.preview();
        Ok(edit)
    }

    pub(in crate::editor) fn owns_document_edit(&self, edit: &DocumentEdit) -> bool {
        self.document.document_preview.same_edit(&edit.preview())
    }

    // Resource-dependent observers run at feature publication, after the caller installs
    // those resources in the same UI transaction as this metadata/history commit.
    pub(in crate::editor) fn commit_document(&mut self, edit: DocumentEdit) -> Result<(), Error> {
        if !self.owns_document_edit(&edit) {
            return Err(Error::InvalidParameter("edit scope has retired"));
        }
        let before = self.document.state.project.clone();
        let (label, photo) = edit.description();
        self.document
            .install(Arc::unwrap_or_clone(edit.into_snapshot()));
        self.record_edit(before, label, photo);
        Ok(())
    }

    pub(in crate::editor) fn edit_project(
        &mut self,
        label: &'static str,
    ) -> Result<ProjectEdit, Error> {
        if self.editing() {
            return Err(Error::InvalidParameter("finish or cancel the current edit"));
        }
        let edit = Edit::from_snapshot(self.document.state.project.snapshot(), label, None);
        self.document.project_preview = edit.preview();
        Ok(edit)
    }

    pub(in crate::editor) fn edit_photo(
        &mut self,
        label: &'static str,
    ) -> Result<ProjectEdit, Error> {
        if self.editing() {
            return Err(Error::InvalidParameter("finish or cancel the current edit"));
        }
        let source = self
            .document
            .visit
            .current_source
            .ok_or(Error::InvalidParameter("no photo"))?;
        let edit = Edit::from_snapshot(self.document.state.project.snapshot(), label, Some(source));
        self.document.visit.project_preview = edit.preview();
        Ok(edit)
    }

    pub(in crate::editor) fn commit_project(&mut self, edit: ProjectEdit) -> Result<(), Error> {
        let preview = edit.preview();
        let current = if self.document.project_preview.active() {
            &self.document.project_preview
        } else {
            &self.document.visit.project_preview
        };
        if !current.same_edit(&preview) {
            return Err(Error::InvalidParameter("edit scope has retired"));
        }
        let project = preview.snapshot().expect("owned edit");
        if project.as_ref() == self.document.state.project.as_ref() {
            return Ok(());
        }
        let (label, photo) = edit.description();
        let before = std::mem::replace(
            &mut self.document.state.project,
            edit.into_snapshot().into(),
        );
        self.observe_gamma_pivot();
        self.record_edit(before, label, photo);
        Ok(())
    }
}

pub(super) struct ControlEdit {
    pub values: ProjectEdit,
    pub input: egui::Id,
    pub gamma_request: Option<InputSettings>,
    pub calculation: Option<async_executor::Task<()>>,
    pub finished: bool,
}

impl ControlEdit {
    pub fn finished(&self) -> bool {
        self.finished
    }
}

impl Editor {
    pub(super) fn photo_settings(&self) -> AdjustmentLayerSettings {
        let project = self.document.project_snapshot();
        let Some(source) = self.document.visit.current_source else {
            return Default::default();
        };
        let adjustment_layers::Target::Layer(layer) = self.adjustment_layer_target() else {
            return Default::default();
        };
        project
            .source(source)
            .ok()
            .and_then(|s| s.adjustment_layers.as_ref())
            .and_then(|layers| layers.layers.iter().find(|record| record.id == layer))
            .map(|record| record.adjustment.photo)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn photo_settings_mut(&mut self) -> &mut AdjustmentLayerSettings {
        let source = self.document.visit.current_source.expect("selected photo");
        let adjustment_layers::Target::Layer(layer) = self.adjustment_layer_target() else {
            panic!("selected layer")
        };
        &mut self
            .document
            .state
            .project
            .source_mut(source)
            .expect("selected source")
            .adjustment_layers
            .as_mut()
            .expect("selected layer stack")
            .layers
            .iter_mut()
            .find(|record| record.id == layer)
            .expect("selected layer")
            .adjustment
            .photo
    }

    pub(super) fn photo_crop(&self) -> Option<Crop> {
        let project = self.document.project_snapshot();
        self.document
            .visit
            .current_source
            .and_then(|id| project.source(id).ok())
            .and_then(|source| source.crop)
    }

    #[cfg(test)]
    pub(super) fn photo_crop_mut(&mut self) -> &mut Option<Crop> {
        let id = self.document.visit.current_source.expect("selected photo");
        &mut self
            .document
            .state
            .project
            .source_mut(id)
            .expect("selected source")
            .crop
    }

    pub(super) fn photo_orientation(&self) -> PhotoOrientation {
        let project = self.document.project_snapshot();
        self.document
            .visit
            .current_source
            .and_then(|id| project.source(id).ok())
            .map(|source| source.orientation)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn photo_orientation_mut(&mut self) -> &mut PhotoOrientation {
        let id = self.document.visit.current_source.expect("selected photo");
        &mut self
            .document
            .state
            .project
            .source_mut(id)
            .expect("selected source")
            .orientation
    }

    pub(super) fn editing(&self) -> bool {
        self.document.document_preview.active()
            || self.document.project_preview.active()
            || self.document.visit.project_preview.active()
    }

    pub(super) fn controls_pending(&self) -> bool {
        self.document
            .controls_edit
            .as_ref()
            .is_some_and(|edit| edit.calculation.is_some())
    }

    pub(super) fn cancel_control_edit(&mut self) {
        self.document.controls_edit = None;
        self.document.visit.controls_edit = None;
    }

    pub(super) fn retire_control_input(&mut self) {
        self.document.visit.controls_edit = None;
        if self
            .document
            .controls_edit
            .as_ref()
            .is_some_and(|edit| !edit.finished())
        {
            self.document.controls_edit = None;
        }
    }

    pub(super) fn begin_control(
        &mut self,
        response: controls::ControlResponse,
        photo: bool,
    ) -> Option<ControlEdit> {
        let slot = if photo {
            &mut self.document.visit.controls_edit
        } else {
            &mut self.document.controls_edit
        };
        let owned = slot.as_ref().is_some_and(|edit| edit.input == response.id);
        if response.cancelled {
            if owned {
                *slot = None;
            }
            return None;
        }
        if !response.enabled {
            return None;
        }
        if owned {
            if slot.as_ref().unwrap().finished {
                return None;
            }
            return slot.take();
        }
        if !response.active && !response.changed {
            return None;
        }
        let values = if photo {
            self.edit_photo("Photo settings")
        } else {
            self.edit_project("Roll settings")
        };
        match values {
            Ok(values) => Some(ControlEdit {
                values,
                input: response.id,
                gamma_request: None,
                calculation: None,
                finished: false,
            }),
            Err(_) => None,
        }
    }

    pub(super) fn retain_control(
        &mut self,
        mut edit: ControlEdit,
        response: controls::ControlResponse,
        photo: bool,
    ) {
        edit.finished = !response.active;
        if edit.finished && edit.calculation.is_none() {
            if let Err(error) = self.commit_project(edit.values) {
                self.status = format!("Edit rejected: {error}");
            }
        } else if photo {
            self.document.visit.controls_edit = Some(edit);
        } else {
            self.document.controls_edit = Some(edit);
        }
    }

    pub(super) fn finish_document_controls(&mut self) {
        if self
            .document
            .controls_edit
            .as_ref()
            .is_some_and(|edit| edit.finished && edit.calculation.is_none())
        {
            let edit = self.document.controls_edit.take().unwrap();
            if let Err(error) = self.commit_project(edit.values) {
                self.status = format!("Edit rejected: {error}");
            }
        }
    }
}

pub(super) type Interaction =
    for<'scope> fn(UiContext<'scope, Editor>) -> Pin<Box<dyn Future<Output = ()> + 'scope>>;

impl Editor {
    /// Retain an otherwise-unowned interactive action until it finishes or its context retires.
    pub(super) fn start_interaction<'scope, F, Fut>(
        &mut self,
        cx: &UiContext<'scope, Self>,
        work: F,
    ) -> Result<(), String>
    where
        F: FnOnce(UiContext<'scope, Editor>) -> Fut + 'scope,
        Fut: Future<Output = ()> + 'scope,
    {
        if !self.prepare_tool() {
            return Ok(());
        }
        self.document.visit.interaction_action = Some(cx.spawn(work).ok_or("Application stopped")?);
        Ok(())
    }
}

impl Editor {
    pub(super) fn finish_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.analysis_review().is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review(cx);
        } else if self.document.visit.picker.display().is_some() {
            self.end_tool();
        }
    }

    pub(super) fn apply_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.current_tool().is_some() {
            self.dispatch_tool(tool::Action::Primary, cx);
        }
    }
}

pub(super) enum PhotoAction {
    TurnLeft,
    TurnRight,
    FlipHorizontal,
    FlipVertical,
    Crop,
    Dust,
}

impl Editor {
    pub(super) fn apply_photo_action(&mut self, action: PhotoAction, cx: &UiContext<'_, Self>) {
        match action {
            PhotoAction::Crop => {
                if self.cropping() {
                    self.dispatch_tool(tool::Action::Dismiss, cx);
                } else {
                    self.begin_crop();
                }
            }
            PhotoAction::Dust => {
                if self.dusting() {
                    self.dispatch_tool(tool::Action::Dismiss, cx);
                } else {
                    self.begin_dust_cleanup();
                }
            }
            _ => {
                let result = self.edit_photo_geometry("Photo orientation", |orientation, _| {
                    match action {
                        PhotoAction::TurnLeft => orientation.turn_counterclockwise(),
                        PhotoAction::TurnRight => orientation.turn_clockwise(),
                        PhotoAction::FlipHorizontal => orientation.flips.horizontal ^= true,
                        PhotoAction::FlipVertical => orientation.flips.vertical ^= true,
                        _ => unreachable!(),
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    self.status = format!("Photo orientation rejected: {error}");
                }
            }
        }
    }

    pub(super) fn edit_photo_geometry(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut PhotoOrientation, &mut Option<Crop>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut draft = self.edit_photo(label)?;
        let mut orientation = self.photo_orientation();
        let mut crop = self.photo_crop();
        edit(&mut orientation, &mut crop)?;
        if let Some(candidate) = crop {
            let area = self
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .map(|ready| ready.area)
                .or_else(|| {
                    let source = self
                        .document
                        .state
                        .project
                        .source(self.document.visit.current_source?)
                        .ok()?;
                    Some(source.development.active_area.into())
                })
                .ok_or(Error::InvalidParameter(
                    "Photo geometry source is unavailable",
                ))?;
            candidate.region(area)?;
        }
        if orientation == self.photo_orientation() && crop == self.photo_crop() {
            return Ok(());
        }
        let source = self
            .document
            .visit
            .current_source
            .expect("current edit source");
        draft.update(|project| {
            let source = project.source_mut(source).expect("current edit source");
            source.orientation = orientation;
            source.crop = crop;
        });
        self.commit_project(draft)?;
        self.document.visit.canvas.viewport.fit();
        self.invalidate_original();
        Ok(())
    }

    pub(super) fn accept_crop(&mut self, draft: Crop) -> Result<(), Error> {
        let area = self
            .selected_area()
            .ok_or(Error::InvalidParameter("Crop source is unavailable"))?;
        self.edit_photo_geometry("Crop", |_, crop| {
            *crop = (draft != Crop::from_region(area)).then_some(draft);
            Ok(())
        })?;
        self.document.visit.canvas.viewport.fit();
        Ok(())
    }
}
