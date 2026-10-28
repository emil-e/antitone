use crate::renderer::GpuRenderer;
use std::sync::Arc;

#[cfg(feature = "editor")]
thread_local! {
    static ORIGINAL_IMAGE_CREATIONS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0, 0]) };
}

#[cfg(feature = "editor")]
pub(crate) fn original_image_created(size: [u32; 2]) {
    ORIGINAL_IMAGE_CREATIONS.with(|count| {
        let [sets, cells] = count.get();
        count.set([sets + 1, cells + u64::from(size[0]) * u64::from(size[1])]);
    });
}

#[cfg(feature = "editor")]
pub(crate) fn take_original_image_creations() -> [u64; 2] {
    ORIGINAL_IMAGE_CREATIONS.with(|count| count.replace([0, 0]))
}

pub(crate) fn render_context() -> (wgpu::Device, wgpu::Queue, Arc<GpuRenderer>) {
    pollster::block_on(async {
        let instance = super::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    })
}
