//! Correction-owned GPU preparation and immutable inputs retained by its consumers.
use super::{Editor, Error, UiContext};
use crate::{gpu::flat::GpuFlatField, project::FlatFieldSettings};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::{cell::RefCell, sync::Arc};

#[cfg(test)]
mod tests;

type PreparedCorrection = Result<Option<Arc<Correction>>, String>;

#[derive(Clone)]
enum Preparation {
    Ready(PreparedCorrection),
    Pending(Shared<BoxFuture<'static, PreparedCorrection>>),
}

/// The requested map remains known before its shared GPU preparation finishes.
#[derive(Clone)]
pub(super) struct Resources {
    map: Option<Arc<crate::flat_field::FlatField>>,
    preparation: Preparation,
}

impl Default for Resources {
    fn default() -> Self {
        Self {
            map: None,
            preparation: Preparation::Ready(Ok(None)),
        }
    }
}

impl From<Option<Arc<Correction>>> for Resources {
    fn from(correction: Option<Arc<Correction>>) -> Self {
        Self {
            map: correction.as_ref().map(|value| value.gpu.field().clone()),
            preparation: Preparation::Ready(Ok(correction)),
        }
    }
}

impl Resources {
    pub(super) fn prepare(
        settings: Option<FlatFieldSettings>,
        device: Option<wgpu::Device>,
    ) -> Self {
        let Some(settings) = settings else {
            return Self::default();
        };
        Self {
            map: Some(settings.map.clone()),
            preparation: Preparation::Pending(
                async move {
                    let device =
                        device.ok_or_else(|| "Correction resources unavailable".to_owned())?;
                    crate::background::run(move |_| {
                        Correction::prepare(&device, Some(&settings), None)
                    })
                    .await
                    .map_err(|_| "Correction preparation worker panicked".to_owned())?
                    .map_err(|error| error.to_string())
                }
                .boxed()
                .shared(),
            ),
        }
    }

    #[cfg(test)]
    pub(super) fn after(
        self,
        wait: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> Self {
        Self {
            map: self.map.clone(),
            preparation: Preparation::Pending(
                async move {
                    wait.await;
                    self.acquire().await
                }
                .boxed()
                .shared(),
            ),
        }
    }

    pub(super) fn map(&self) -> Option<&Arc<crate::flat_field::FlatField>> {
        self.map.as_ref()
    }

    fn result(&self) -> Option<&PreparedCorrection> {
        match &self.preparation {
            Preparation::Ready(result) => Some(result),
            Preparation::Pending(preparation) => preparation.peek(),
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

    pub(super) fn gpu(&self) -> Result<Option<Arc<GpuFlatField>>, Error> {
        match self.result() {
            Some(Ok(correction)) => Ok(correction.as_ref().map(|value| value.gpu.clone())),
            Some(Err(error)) => Err(Error::Gpu(error.clone())),
            None => Err(Error::Gpu("Correction resources are preparing".into())),
        }
    }

    pub(super) async fn acquire(self) -> PreparedCorrection {
        match self.preparation {
            Preparation::Ready(result) => result,
            Preparation::Pending(preparation) => preparation.await,
        }
    }
}

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

pub(super) struct Correction {
    pub gpu: Arc<GpuFlatField>,
}

impl Correction {
    pub fn prepare(
        device: &wgpu::Device,
        settings: Option<&FlatFieldSettings>,
        previous: Option<&Arc<Self>>,
    ) -> Result<Option<Arc<Self>>, Error> {
        let Some(settings) = settings else {
            return Ok(None);
        };
        if let Some(previous) = previous.filter(|previous| previous.gpu.field() == &settings.map) {
            return Ok(Some(previous.clone()));
        }
        Ok(Some(Arc::new(Self {
            gpu: Arc::new(GpuFlatField::new(device, settings.map.clone())?),
        })))
    }
}
