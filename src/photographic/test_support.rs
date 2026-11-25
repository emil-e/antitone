use crate::render_fixture::RenderFixture;
use std::sync::Arc;

pub(crate) fn render_context() -> (
    wgpu::Device,
    crate::gpu::submission::Queue,
    Arc<RenderFixture>,
) {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = crate::gpu::submission::request_device(&adapter, &Default::default())
            .await
            .unwrap();
        let renderer = RenderFixture::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    })
}
