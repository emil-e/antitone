//! One immutable camera proxy, with only the active target's mutable output retained.
use super::*;
#[cfg(test)]
#[path = "viewer_proxy_tests.rs"]
mod tests;

#[derive(Clone)]
pub(crate) struct GpuProxy(Arc<ProxyState>);

struct ProxyState {
    device: wgpu::Device,
    renderer: Arc<GpuRenderer>,
    source: Arc<ProcessingSource>,
    size: [u32; 2],
    active: std::sync::Mutex<Option<(OutputTarget, Arc<GpuImage>)>>,
}

impl GpuProxy {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        renderer: Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        pixels: &[[u16; 4]],
        matrix: DMat3,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::InvalidParameter(
                "source dimensions do not match the GPU image",
            ));
        }
        let mut source = ProcessingSource::new(device, pixels, matrix)?;
        source.set_flat_field(field, geometry)?;
        Ok(Self(Arc::new(ProxyState {
            device: device.clone(),
            renderer,
            source: Arc::new(source),
            size,
            active: Default::default(),
        })))
    }

    pub(crate) fn image(&self, target: OutputTarget) -> Result<Arc<GpuImage>, Error> {
        let mut active = self.0.active.lock().unwrap();
        if let Some((current, image)) = &*active
            && *current == target
        {
            return Ok(image.clone());
        }
        let image = Arc::new(pollster::block_on(GpuImage::create(
            &self.0.device,
            self.0.size[0],
            self.0.size[1],
            Some((
                self.0.renderer.clone(),
                ImageSource::Shared(self.0.source.clone()),
            )),
            self.0.renderer.display.clone(),
            target,
        ))?);
        *active = Some((target, image.clone()));
        Ok(image)
    }

    pub(crate) fn release_output(&self) {
        self.0.active.lock().unwrap().take();
    }

    pub(crate) fn byte_len(&self) -> u64 {
        let active = self.0.active.lock().unwrap();
        self.0.source.byte_len()
            + active.as_ref().map_or(0, |(_, image)| {
                image.processing_byte_len().1
                    + u64::from(self.0.size[0]) * u64::from(self.0.size[1]) * 16
            })
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn output_retained(&self) -> bool {
        self.0.active.lock().unwrap().is_some()
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
