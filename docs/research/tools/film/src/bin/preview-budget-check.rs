use antitone::resample::AreaAverage;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await?;
        let (default_device, _) = adapter.request_device(&Default::default()).await?;
        let source = [8640, 5760];
        let canvas = [2560f32, 1707f32];
        let scale = (canvas[0] / source[0] as f32).min(canvas[1] / source[1] as f32);
        let output = source.map(|axis| ((axis as f32 * scale * 2.).ceil() as u32).clamp(1, axis));
        let bytes = u64::from(output[0]) * u64::from(output[1]) * 16;
        let budget = default_device.limits().max_storage_buffer_binding_size;
        println!(
            "adapter={:?} fit_canvas_logical={canvas:?} pixels_per_point=2 source={source:?} output={output:?} full_frame_sum_bytes={bytes} storage_binding_budget_bytes={budget}",
            adapter.get_info()
        );
        assert!(bytes > budget);
        let error = AreaAverage::new(&default_device, source, output)
            .await
            .err()
            .expect("default limits reproduce the oversized fit rejection");
        println!("default_limits: {error}");
        drop(default_device);
        let (device, _) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: antitone::platform::device_limits(adapter.limits()),
                ..Default::default()
            })
            .await?;
        println!(
            "negotiated_storage_binding_bytes={} negotiated_buffer_bytes={}",
            device.limits().max_storage_buffer_binding_size,
            device.limits().max_buffer_size
        );
        AreaAverage::new(&device, source, output).await?;
        println!("large valid fit request accepted with adapter-supported buffer limits");
        Ok(())
    })
}
