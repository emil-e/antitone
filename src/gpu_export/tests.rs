use super::*;

pub(crate) fn context() -> (wgpu::Device, wgpu::Queue, Arc<GpuRenderer>) {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        (device, queue, renderer)
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn exr_pixels(path: &Path) -> Vec<[f32; 3]> {
    use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
    let image = read_first_flat_layer_from_file(path).unwrap();
    let mut pixels = vec![[0.; 3]; image.layer_data.size.0 * image.layer_data.size.1];
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        match &channel.sample_data {
            FlatSamples::F16(values) => {
                for (p, v) in pixels.iter_mut().zip(values) {
                    p[c] = v.to_f32();
                }
            }
            FlatSamples::F32(values) => {
                for (p, v) in pixels.iter_mut().zip(values) {
                    p[c] = *v;
                }
            }
            _ => panic!("unexpected EXR sample type"),
        }
    }
    pixels
}

#[cfg(target_os = "macos")]
pub(crate) fn tiff_pixels(path: &Path) -> Vec<u16> {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    match decoder.read_image().unwrap() {
        tiff::decoder::DecodingResult::U16(values) => values,
        _ => panic!("expected RGB16"),
    }
}
