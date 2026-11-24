//! Correction-owned GPU preparation and immutable inputs retained by its consumers.
use super::{Editor, UiContext};
pub(super) use crate::gpu::flat::resources::{Correction, Resources};
use crate::project::FlatFieldSettings;
use std::{cell::RefCell, sync::Arc};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct Entry {
    resources: Resources,
    observation: Option<async_executor::Task<()>>,
    failure_presented: bool,
}

/// A memo, not accepted state: every request resolves against the current map.
#[derive(Default)]
pub(super) struct State(RefCell<Entry>);

impl From<Option<Arc<Correction>>> for State {
    fn from(correction: Option<Arc<Correction>>) -> Self {
        Resources::from(correction).into()
    }
}

impl From<Resources> for State {
    fn from(resources: Resources) -> Self {
        Self(RefCell::new(Entry {
            resources,
            ..Default::default()
        }))
    }
}

impl State {
    fn get(&self, settings: Option<&FlatFieldSettings>, device: Option<wgpu::Device>) -> Resources {
        let mut entry = self.0.borrow_mut();
        if entry.resources.map() != settings.map(|settings| &settings.map) {
            *entry = Entry {
                resources: Resources::prepare(settings.cloned(), device),
                ..Default::default()
            };
        }
        entry.resources.clone()
    }

    pub(super) fn observe(&self, cx: &UiContext<'_, Editor>) {
        let mut entry = self.0.borrow_mut();
        if !entry.resources.pending() || entry.observation.is_some() {
            return;
        }
        let resources = entry.resources.clone();
        entry.observation = cx.spawn(move |cx| async move {
            let _ = resources.acquire().await;
            cx.update(|_| {});
        });
    }

    fn take_failure(&self) -> bool {
        let mut entry = self.0.borrow_mut();
        if entry.failure_presented || entry.resources.error().is_none() {
            return false;
        }
        entry.failure_presented = true;
        true
    }
}

impl Editor {
    pub(super) fn correction(&self) -> Resources {
        let device = self
            .document
            .visit
            .working_view
            .device()
            .cloned()
            .or_else(|| {
                self.source_services
                    .as_ref()
                    .map(|services| services.device.clone())
            });
        self.document
            .correction
            .get(self.document.project.active_flat_field(), device)
    }

    pub(super) fn correction_feedback(&mut self) {
        let _ = self.correction();
        if self.document.correction.take_failure() {
            self.document.source = true;
            self.preferences
                .sections
                .insert("roll-flat-field".into(), true);
        }
    }
}
