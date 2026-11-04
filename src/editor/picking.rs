use super::*;
use futures_channel::oneshot;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
};

mod interaction;
pub(super) mod measurement;
#[cfg(test)]
mod tests;
pub(super) use interaction::{PickTool, paint_selection};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum View {
    Working,
    Source,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Display {
    pub label: &'static str,
    pub view: View,
    pub area: bool,
    pub lock_layer_selection: bool,
    pub help: Option<&'static str>,
}

impl Display {
    pub const fn sample(label: &'static str, view: View) -> Self {
        Self {
            label,
            view,
            area: false,
            lock_layer_selection: false,
            help: None,
        }
    }
}

struct Interaction {
    display: Display,
    region: Option<Coverage>,
    drag: Option<PickDrag>,
    submit: Option<oneshot::Sender<Coverage>>,
    request: Option<u64>,
}

impl Interaction {
    fn activity(&self) -> Option<TaskRequest> {
        self.request
            .filter(|_| self.submit.is_none())
            .map(TaskRequest::Measurement)
    }

    fn submit(&mut self, editor: &mut Editor) {
        if let Some(region) = self.region
            && let Some(send) = self.submit.take()
        {
            let id = self.request.expect("measurement owns its request");
            editor
                .tasks
                .begin(Task::Measurement(id, self.display.label));
            editor.busy = true;
            editor.status = "Measuring full-resolution patch…".into();
            let _ = send.send(region);
        }
    }
}

/// Read-only observation of the measurement picker; the tool owns the interaction.
#[derive(Default)]
pub(super) struct State {
    active: Weak<RefCell<Interaction>>,
    #[cfg(test)]
    pub measurement_gate: Option<measurement::testing::Gate>,
}

impl State {
    pub fn display(&self) -> Option<Display> {
        self.active.upgrade().map(|state| state.borrow().display)
    }

    #[cfg(test)]
    pub fn region(&self) -> Option<Coverage> {
        self.active
            .upgrade()
            .and_then(|state| state.borrow().region)
    }

    #[cfg(test)]
    pub fn set_region(&self, region: Option<Coverage>) {
        self.active
            .upgrade()
            .expect("active picker fixture")
            .borrow_mut()
            .region = region;
    }
}

fn install(
    editor: &mut Editor,
    display: Display,
    previous: Option<(crate::viewport::CanvasLayout, f32)>,
    submit: Option<oneshot::Sender<Coverage>>,
    request: Option<u64>,
    task: Option<async_executor::Task<()>>,
) {
    let state = Rc::new(RefCell::new(Interaction {
        display,
        region: None,
        drag: None,
        submit,
        request,
    }));
    let tool = PickTool::new(editor, state.clone(), previous, task);
    editor.document.thumbnails.cancel();
    editor
        .document
        .set_hosted_tool(tool::Placement::Photo, Box::new(tool));
    editor.document.visit.picker.active = Rc::downgrade(&state);
}

#[cfg(test)]
pub(super) fn preview(editor: &mut Editor, display: Display) {
    let previous = editor.document.visit.canvas.last_layout;
    if editor.prepare_tool() {
        install(editor, display, previous, None, None, None);
    }
}

impl Editor {
    pub(super) fn picking(&self) -> bool {
        self.document.visit.picker.display().is_some()
    }

    #[cfg(test)]
    pub(super) fn measurement_pending(&self) -> bool {
        matches!(
            self.hosted_tool_activity(),
            Some(TaskRequest::Measurement(_))
        )
    }
}
