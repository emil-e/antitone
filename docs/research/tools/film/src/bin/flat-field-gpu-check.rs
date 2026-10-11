use antitone::{
    color::ap0_to_ap1,
    flat_field::FlatField,
    gpu::{GpuFlatField, ProcessingParameters, SourceGeometry},
    processing::{Calibration, PhotoSettings, Pipeline, Response, RollSettings},
    raw::Rect,
    rendering::{CpuRenderer, OutputTarget},
    scan::Scan,
    viewer::GpuImage,
};
use glam::DVec3;
use std::{path::Path, sync::Arc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 8 {
        return Err(
            "usage: flat-field-gpu-check REF1 NEGATIVE1 REF2 NEGATIVE2 x y width height".into(),
        );
    }
    let region = Rect {
        x: args[4].parse()?,
        y: args[5].parse()?,
        width: args[6].parse()?,
        height: args[7].parse()?,
    };
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await?;
        let (device, queue) = adapter.request_device(&Default::default()).await?;
        let mut wb = None;
        for pair in args[..4].chunks_exact(2) {
            let mut reference = Scan::open(Path::new(&pair[0]), wb)?;
            wb = Some(reference.fixed_wb);
            let field = Arc::new(reference.estimate_flat_field(region)?);
            let resource = Arc::new(GpuFlatField::new(&device, field.clone())?);
            let area = reference.area;
            let center = Rect {
                x: area.x + (area.width - 256) / 2,
                y: area.y + (area.height - 256) / 2,
                width: 256,
                height: 256,
            };
            let original = reference.reference(center)?;
            let original_y = (antitone::color::AP0_TO_XYZ * original).y;
            reference.set_flat_field(Some(field.clone()))?;
            let corrected = reference.reference(center)?;
            let center_ev = (corrected / DVec3::splat(original_y)).map(f64::log2);
            assert!(center_ev.abs().max_element() < 0.002);
            println!(
                "reference={} nodes={} CPU_bytes={} GPU_bytes={} center_neutral_RGB_EV={center_ev:?} base_floor={} corrected_floor={}",
                pair[0],
                field.nodes().len(),
                field.nodes().len() * 24,
                resource.byte_len(),
                reference.input_floor,
                reference.effective_floor()
            );
            check(&device, &queue, &mut reference, &field, &resource, &pair[0]).await?;
            let mut negative = Scan::open(Path::new(&pair[1]), wb)?;
            negative.set_flat_field(Some(field.clone()))?;
            check(&device, &queue, &mut negative, &field, &resource, &pair[1]).await?;
        }
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
async fn check(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    scan: &mut Scan,
    field: &Arc<FlatField>,
    resource: &Arc<GpuFlatField>,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let area = scan.area;
    let (w, h, pixels) = scan.preview_source(128)?;
    let geometry = SourceGeometry::new([area.x, area.y, area.width, area.height], [w, h])?;
    let corrected: Vec<_> = (0..w * h)
        .map(|i| {
            let [x, y] = geometry.pixel(i)?;
            scan.pixel(x, y)
        })
        .collect::<Result<_, _>>()?;
    let baseline = Calibration {
        dmin: DVec3::new(0.2, 0.15, 0.1),
        dmax: DVec3::new(0.008, 0.005, 0.002),
        input_floor: scan.input_floor,
    };
    let calibration = scan.processing_calibration(baseline)?;
    assert_eq!(
        calibration,
        field.processing_calibration(scan.camera_to_ap0, baseline)?
    );
    let mut cases = vec![(
        "source",
        ProcessingParameters::corrected_source(0.)?,
        corrected
            .iter()
            .map(|p| ap0_to_ap1() * p)
            .collect::<Vec<_>>(),
    )];
    for response in [Response::Original, Response::film()] {
        let roll = RollSettings {
            response,
            ..RollSettings::default()
        };
        let pipeline = Pipeline::new(calibration, roll, PhotoSettings::default())?;
        let working = corrected
            .iter()
            .map(|p| pipeline.apply(*p).map(|p| p.rgb))
            .collect::<Result<Vec<_>, _>>()?;
        cases.push((
            if response == Response::Original {
                "original"
            } else {
                "film"
            },
            ProcessingParameters::positive(calibration, roll, PhotoSettings::default())?,
            working,
        ));
    }
    let bad = Arc::new(FlatField::from_nodes(
        [area.x + area.width, area.y, 1, 1],
        [2, 2],
        vec![DVec3::ONE; 4],
    )?);
    assert!(scan.set_flat_field(Some(bad)).is_err());
    assert_eq!(scan.flat_field(), Some(field));
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let mut image =
            GpuImage::new(device, queue, w, h, &pixels, scan.camera_to_ap0, target).await?;
        image.set_flat_field(Some(resource.clone()), geometry)?;
        for (mode, parameters, working) in &cases {
            let mut encoder = device.create_command_encoder(&Default::default());
            image.encode(device, &mut encoder, *parameters);
            queue.submit([encoder.finish()]);
            let actual = read(device, queue, &image.linear)?;
            let expected = CpuRenderer::new(target)?.render(working)?;
            let max = actual
                .iter()
                .zip(expected)
                .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
                .fold(0., f32::max);
            assert!(
                max < 2e-5,
                "source={name} target={target:?} mode={mode} error={max}"
            );
            println!(
                "source={name} target={target:?} mode={mode} pixels={} max_linear_error={max:.9e}",
                working.len()
            );
        }
    }
    scan.set_flat_field(None)?;
    let [x, y] = geometry.pixel(0)?;
    assert_eq!(
        scan.pixel(x, y)?,
        scan.camera_to_ap0
            * (DVec3::new(
                pixels[0][0] as f64,
                pixels[0][1] as f64,
                pixels[0][2] as f64
            ) / 65535.)
    );
    assert_eq!(scan.effective_floor(), scan.input_floor);
    Ok(())
}
fn read(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let row = (texture.width() * 16).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |r| send.send(r).unwrap());
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receive.recv()??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let values = mapped
        .chunks_exact(row as usize)
        .flat_map(|r| {
            r[..texture.width() as usize * 16]
                .chunks_exact(16)
                .map(|p| {
                    std::array::from_fn(|c| {
                        f32::from_ne_bytes(p[c * 4..c * 4 + 4].try_into().unwrap())
                    })
                })
        })
        .collect();
    drop(mapped);
    buffer.unmap();
    Ok(values)
}
