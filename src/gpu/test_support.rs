use crate::renderer::GpuRenderer;
use std::sync::Arc;

#[cfg(feature = "editor")]
thread_local! {
    static EVALUATION_BUFFER_CREATIONS: std::cell::Cell<[u64; 2]> = const { std::cell::Cell::new([0, 0]) };
}

#[cfg(feature = "editor")]
pub(crate) fn evaluation_buffer_created(bytes: u64) {
    EVALUATION_BUFFER_CREATIONS.with(|count| {
        let [buffers, total_bytes] = count.get();
        count.set([buffers + 1, total_bytes + bytes]);
    });
}

#[cfg(feature = "editor")]
pub(crate) fn take_evaluation_buffer_creations() -> [u64; 2] {
    EVALUATION_BUFFER_CREATIONS.with(|count| count.replace([0, 0]))
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
