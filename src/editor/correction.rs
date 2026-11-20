//! Captured flat-field resources shared by photographic consumers.
use super::Error;
use crate::{gpu::flat::GpuFlatField, project::FlatFieldSettings};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::sync::Arc;

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
    pub(super) fn prepare(
        settings: Option<FlatFieldSettings>,
        device: Option<wgpu::Device>,
    ) -> Self {
        Self::Preparing(
            async move {
                let device = device.ok_or_else(|| "Correction resources unavailable".to_owned())?;
                crate::background::run(move |_| {
                    Correction::prepare(&device, settings.as_ref(), None)
                })
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
        settings: Option<&FlatFieldSettings>,
        previous: Option<&Arc<Self>>,
    ) -> Result<Option<Arc<Self>>, Error> {
        let Some(settings) = settings else {
            return Ok(None);
        };
        if let Some(previous) = previous.filter(|previous| previous.settings == *settings) {
            return Ok(Some(previous.clone()));
        }
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu: Arc::new(GpuFlatField::new(device, settings.map.clone())?),
        })))
    }
}
