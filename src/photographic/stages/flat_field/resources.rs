//! Shared preparation and immutable flat-field GPU inputs.
use super::GpuFlatField;
use crate::{Error, project::FlatFieldSettings};
use futures_util::{
    FutureExt,
    future::{BoxFuture, Shared},
};
use std::sync::Arc;

type PreparedCorrection = Result<Option<Arc<Correction>>, String>;

#[derive(Clone)]
enum Preparation {
    Ready(PreparedCorrection),
    Pending(Shared<BoxFuture<'static, PreparedCorrection>>),
}

/// The requested map remains known before its shared GPU preparation finishes.
#[derive(Clone)]
pub(crate) struct Resources {
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
    pub(crate) fn prepare(
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
    pub(crate) fn after(
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

    pub(crate) fn map(&self) -> Option<&Arc<crate::flat_field::FlatField>> {
        self.map.as_ref()
    }

    fn result(&self) -> Option<&PreparedCorrection> {
        match &self.preparation {
            Preparation::Ready(result) => Some(result),
            Preparation::Pending(preparation) => preparation.peek(),
        }
    }

    pub(crate) fn prepared(&self) -> Option<&Arc<Correction>> {
        self.result()?.as_ref().ok()?.as_ref()
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.result()?.as_ref().err().map(String::as_str)
    }

    pub(crate) fn pending(&self) -> bool {
        self.result().is_none()
    }

    pub(crate) fn gpu(&self) -> Result<Option<Arc<GpuFlatField>>, Error> {
        match self.result() {
            Some(Ok(correction)) => Ok(correction.as_ref().map(|value| value.gpu.clone())),
            Some(Err(error)) => Err(Error::Gpu(error.clone())),
            None => Err(Error::Gpu("Correction resources are preparing".into())),
        }
    }

    pub(crate) async fn acquire(self) -> PreparedCorrection {
        match self.preparation {
            Preparation::Ready(result) => result,
            Preparation::Pending(preparation) => preparation.await,
        }
    }
}

pub(crate) struct Correction {
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
