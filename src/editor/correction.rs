//! Captured flat-field resources shared by photographic consumers.
use super::{Error, document::Document};
use crate::{
    gpu::{flat::GpuFlatField, source::SourceGeometry},
    project::FlatFieldSettings,
};
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
    pub(super) fn prepare(document: Document, device: Option<wgpu::Device>) -> Self {
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
        let Some(settings) = document.project.active_flat_field() else {
            return Ok(None);
        };
        let previous = previous.filter(|previous| previous.settings == *settings);
        let gpu = match previous {
            Some(previous) => previous.gpu.clone(),
            None => Arc::new(GpuFlatField::new(device, settings.map.clone())?),
        };
        for source in &document.project.sources {
            if document.project.order.contains(&source.id) {
                let development = crate::development_metadata::Development::captured(
                    source,
                    &document
                        .exposure_origin(source.id)
                        .expect("registered photo locator"),
                    document.project.input,
                    &document.development_metadata,
                );
                let Ok(development) = development.resolved() else {
                    continue;
                };
                document.require_scan_source(development)?;
                let matrix = glam::DMat3::from_cols_array_2d(&development.camera_to_ap1);
                crate::gpu::parameters::CameraMatrix::require_source(matrix, true)?;
                gpu.validate_matrix(matrix, crate::processing::MAX_SOURCE_GAIN)?;
                SourceGeometry::new(source.development.active_area, [1, 1])?;
            }
        }
        if let Some(previous) = previous {
            return Ok(Some(previous.clone()));
        }
        Ok(Some(Arc::new(Self {
            settings: settings.clone(),
            gpu,
        })))
    }
}
