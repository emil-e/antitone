use super::*;
use crate::{renderer::GpuRenderer, rendering::OutputTarget};
use wgpu::util::DeviceExt;

#[cfg(feature = "ocio-reference")]
#[path = "gpu_aces_execution_tests.rs"]
mod aces_execution_tests;

struct InputRaster {
    size: [u32; 2],
    area: SourceRegion,
    region: SourceRegion,
    pixels: Vec<[u16; 4]>,
    calibration: Calibration,
    matrix: DMat3,
    gather_ms: f64,
}
impl InputRaster {
    fn synthetic(size: [u32; 2]) -> Self {
        let start = Instant::now();
        let pixels = (0u32..size[0] * size[1])
            .map(|i| {
                [
                    i.wrapping_mul(1871) as u16,
                    i.wrapping_mul(7919) as u16,
                    i.wrapping_mul(3413) as u16,
                    42,
                ]
            })
            .collect();
        Self {
            size,
            area: area(),
            region: area(),
            pixels,
            calibration: calibration(),
            matrix: camera_matrix(),
            gather_ms: start.elapsed().as_secs_f64() * 1000.,
        }
    }
    #[cfg(feature = "raw")]
    fn raw(scan: &crate::scan::Scan, calibration: Calibration, size: [u32; 2]) -> Self {
        let start = Instant::now();
        let area = scan.area;
        // A declared 16:9 centre crop fills the benchmark raster; this is not a RAW resample.
        let height = area.width * size[1] / size[0];
        assert!(height <= area.height);
        let region = SourceRegion {
            y: area.y + (area.height - height) / 2,
            height,
            ..area
        };
        let geometry =
            SourceGeometry::new([region.x, region.y, region.width, region.height], size).unwrap();
        let mut pixels = Vec::with_capacity((size[0] * size[1]) as usize);
        for y in 0..size[1] {
            let sy = geometry.pixel(y * size[0]).unwrap()[1];
            let row = scan
                .tile_region(region, 0, sy - region.y, region.width, 1)
                .unwrap();
            for x in 0..size[0] {
                let sx = geometry.pixel(y * size[0] + x).unwrap()[0];
                pixels.push(row[(sx - region.x) as usize]);
            }
        }
        Self {
            size,
            area,
            region,
            pixels,
            calibration,
            matrix: scan.camera_to_ap1,
            gather_ms: start.elapsed().as_secs_f64() * 1000.,
        }
    }
}

struct ScreenTarget {
    texture: wgpu::Texture,
    group: wgpu::BindGroup,
    target: OutputTarget,
}
impl ScreenTarget {
    fn new(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        output: &wgpu::Buffer,
        size: [u32; 2],
        target: OutputTarget,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("screen probe linear target RGB"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Self::with_texture(device, renderer, output, texture, target)
    }
    fn with_texture(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        output: &wgpu::Buffer,
        texture: wgpu::Texture,
        target: OutputTarget,
    ) -> Self {
        let size = [texture.width(), texture.height()];
        let resources = renderer.target(target);
        let dimensions = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("screen probe dimensions"),
            contents: bytemuck::cast_slice(&[size[0], size[1], 0, 0]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("screen probe production ACES bindings"),
            layout: &resources.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&resources.tables[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&resources.tables[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&resources.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: dimensions.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: resources.hues.as_entire_binding(),
                },
            ],
        });
        Self {
            texture,
            group,
            target,
        }
    }
    fn encode(&self, renderer: &GpuRenderer, encoder: &mut wgpu::CommandEncoder) {
        let view = self.texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("screen probe pinned ACES"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&renderer.target(self.target).pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        pass.draw(0..3, 0..1);
    }
    fn words(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<u32> {
        let row = (self.texture.width() * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen probe target readback"),
            size: u64::from(row) * u64::from(self.texture.height()),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(self.texture.height()),
                },
            },
            self.texture.size(),
        );
        queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        read.map_async(wgpu::MapMode::Read, .., move |result| {
            send.send(result).unwrap()
        });
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        receive.recv().unwrap().unwrap();
        let mapped = read.get_mapped_range(..).unwrap();
        let mut words =
            Vec::with_capacity((self.texture.width() * self.texture.height() * 4) as usize);
        for bytes in mapped.chunks(row as usize) {
            words.extend_from_slice(bytemuck::cast_slice(
                &bytes[..self.texture.width() as usize * 16],
            ));
        }
        drop(mapped);
        read.unmap();
        words
    }
}

#[derive(Clone, Copy)]
struct Timing {
    encode_ms: f64,
    completion_ms: f64,
}
#[derive(Clone, Copy, Debug)]
enum Cut {
    None,
    Log,
    Camera,
    BeforeLastLocal,
}
struct ScreenProbe {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Arc<GpuRenderer>,
    source: ProcessingSource,
    probe: Probe,
    calibration: Calibration,
    area: SourceRegion,
    peak_resource_bytes: usize,
}
impl ScreenProbe {
    async fn new(input: &InputRaster, corrected: bool) -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: crate::platform::device_limits(adapter.limits()),
                ..Default::default()
            })
            .await
            .unwrap();
        println!(
            "screen_adapter size={:?} corrected={corrected} adapter={:?} limits_storage={} limits_buffer={}",
            input.size,
            adapter.get_info(),
            device.limits().max_storage_buffer_binding_size,
            device.limits().max_buffer_size
        );
        let renderer = GpuRenderer::new(&device, &queue).await.unwrap();
        let start = Instant::now();
        let mut source = ProcessingSource::new(&device, &input.pixels, input.matrix).unwrap();
        let r = input.region;
        let geometry = SourceGeometry::new([r.x, r.y, r.width, r.height], input.size).unwrap();
        source.set_geometry(geometry).unwrap();
        let mut calibration = input.calibration;
        if corrected {
            let a = input.area;
            let field = Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [a.x, a.y, a.width, a.height],
                    [2, 2],
                    vec![
                        DVec3::new(1.2, 0.9, 1.1),
                        DVec3::new(0.95, 1.3, 1.05),
                        DVec3::new(1.1, 1.05, 0.88),
                        DVec3::new(1.25, 0.98, 1.15),
                    ],
                )
                .unwrap(),
            );
            calibration = field.processing_calibration(calibration).unwrap();
            source
                .set_flat_field(
                    Some(Arc::new(GpuFlatField::new(&device, field).unwrap())),
                    geometry,
                )
                .unwrap();
        }
        let source_prepare_ms = start.elapsed().as_secs_f64() * 1000.;
        let uploaded = queue.submit([device.create_command_encoder(&Default::default()).finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(uploaded),
                timeout: None,
            })
            .unwrap();
        let source_upload_completion_ms = start.elapsed().as_secs_f64() * 1000.;
        let probe = Probe::new(&device, input.pixels.len()).await;
        println!(
            "screen_storage size={:?} corrected={corrected} gather_ms={:.3} source_prepare_ms={source_prepare_ms:.3} source_upload_completion_ms={source_upload_completion_ms:.3} source_bytes={} working_bytes={} prefix_bytes={} probe_code_table_bytes={} correction={} source_prepare_includes=buffer_initialization excludes=shader_compilation",
            input.size,
            input.gather_ms,
            source.byte_len(),
            probe.output.size(),
            probe.prefix.size(),
            probe.response.size(),
            if corrected { "synthetic_2x2" } else { "none" }
        );
        Self {
            device,
            queue,
            renderer,
            source,
            probe,
            calibration,
            area: input.area,
            peak_resource_bytes: 0,
        }
    }
    fn grade(
        &self,
        snapshot: PhotoGradeSnapshot,
        roll: RollSettings,
    ) -> Result<GpuPhotoGrade, Error> {
        GpuPhotoGrade::new(
            &self.device,
            self.calibration,
            roll,
            snapshot,
            self.source.camera_to_ap1,
            self.area,
        )
    }
    fn run(
        &self,
        grade: Option<&GpuPhotoGrade>,
        steps: Option<&[usize]>,
        target: Option<&ScreenTarget>,
    ) -> Timing {
        if let Some(grade) = grade {
            grade.require_source(&self.source).unwrap();
        }
        let start = Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        if let Some(grade) = grade {
            if let Some(steps) = steps {
                self.probe
                    .encode(&self.device, &mut encoder, &self.source, grade, steps);
            } else {
                self.renderer
                    .kernel
                    .encode_grade(
                        &self.device,
                        &mut encoder,
                        &self.source,
                        &self.probe.output,
                        grade,
                    )
                    .unwrap();
            }
        }
        if let Some(target) = target {
            target.encode(&self.renderer, &mut encoder);
        }
        let encode_ms = start.elapsed().as_secs_f64() * 1000.;
        let submitted = self.queue.submit([encoder.finish()]);
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: None,
            })
            .unwrap();
        Timing {
            encode_ms,
            completion_ms: start.elapsed().as_secs_f64() * 1000.,
        }
    }
    fn sample_memory(&mut self, phase: &str) {
        let bytes = allocated_resource_bytes(&self.device).unwrap();
        self.peak_resource_bytes = self.peak_resource_bytes.max(bytes);
        println!(
            "screen_memory phase={phase} metal_resource_bytes={bytes} peak_sampled_bytes={} epoch_ns={}",
            self.peak_resource_bytes,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
    }
    fn build(&self, grade: &GpuPhotoGrade, cut: Cut) {
        match cut {
            Cut::Log => {
                self.run(Some(grade), Some(&[7]), None);
            }
            Cut::Camera => {
                self.run(Some(grade), Some(&[2]), None);
            }
            Cut::BeforeLastLocal => {
                let prefix = PhotoGradeSnapshot::new(
                    grade.snapshot().global(),
                    grade.snapshot().locals()[..1].to_vec(),
                )
                .unwrap();
                let prefix = self.grade(prefix, RollSettings::new_grade()).unwrap();
                self.run(Some(&prefix), Some(&[0, 6]), None);
            }
            Cut::None => {}
        }
    }
    fn suffix(&self, grade: &GpuPhotoGrade, cut: Cut) -> Option<GpuPhotoGrade> {
        if matches!(cut, Cut::BeforeLastLocal) {
            Some(
                self.grade(
                    PhotoGradeSnapshot::new(
                        PhotoSettings::default(),
                        grade.snapshot().locals()[1..].to_vec(),
                    )
                    .unwrap(),
                    RollSettings::new_grade(),
                )
                .unwrap(),
            )
        } else {
            None
        }
    }
    fn steps(cut: Cut) -> Option<&'static [usize]> {
        match cut {
            Cut::None => None,
            Cut::Log => Some(&[8]),
            Cut::Camera => Some(&[3]),
            Cut::BeforeLastLocal => Some(&[5]),
        }
    }
}

fn edited(case: &str, edit: usize) -> PhotoGradeSnapshot {
    let amount = edit as f64 * 0.0625;
    let global = match case {
        "exposure" => PhotoSettings {
            exposure: amount,
            ..Default::default()
        },
        "wb" => PhotoSettings {
            wb: Balance {
                warmth: 0.005 + amount * 0.01,
                tint: -0.003,
            },
            ..Default::default()
        },
        "global-photographic" => PhotoSettings {
            contrast: 1.01 + amount * 0.01,
            ..photographic()
        },
        _ => PhotoSettings::default(),
    };
    let mut locals = snapshot(global, usize::from(case.starts_with("local-")) * 2, true)
        .locals()
        .to_vec();
    if !locals.is_empty() {
        let index = usize::from(case == "local-last");
        locals[index].photo.exposure += amount;
    }
    PhotoGradeSnapshot::new(global, locals).unwrap()
}

async fn measure(input: InputRaster, corrected: bool, kind: &str) {
    let mut harness = ScreenProbe::new(&input, corrected).await;
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let rendered = ScreenTarget::new(
            &harness.device,
            &harness.renderer,
            &harness.probe.output,
            input.size,
            target,
        );
        harness.sample_memory("allocated");
        for case in [
            "exposure",
            "wb",
            "global-photographic",
            "local-first",
            "local-last",
        ] {
            let cut = if case == "local-last" {
                Cut::BeforeLastLocal
            } else if !corrected {
                Cut::None
            } else if case == "exposure" {
                Cut::Log
            } else {
                Cut::Camera
            };
            let grades = (0..4)
                .map(|i| harness.grade(edited(case, i), RollSettings::new_grade()))
                .collect::<Result<Vec<_>, _>>();
            let grades = match grades {
                Ok(grades) => grades,
                Err(error) => {
                    println!(
                        "screen_case kind={kind} size={:?} corrected={corrected} target={target:?} case={case} unsupported={error}",
                        input.size
                    );
                    continue;
                }
            };
            let mut full = Vec::new();
            let mut fresh = Vec::new();
            let mut full_processing = Vec::new();
            let mut aces = Vec::new();
            let mut warm = Vec::new();
            let mut cold = Vec::new();
            let mut encode = Vec::new();
            // Native log/camera cuts survive these downstream edits; the local cut has a stable preceding local.
            harness.run(Some(&grades[0]), None, Some(&rendered));
            harness.build(&grades[0], cut);
            for (i, grade) in grades[1..].iter().enumerate() {
                fresh.push(
                    harness
                        .run(Some(grade), None, Some(&rendered))
                        .completion_ms,
                );
                let expected = read_words(&harness.device, &harness.queue, &harness.probe.output);
                let target_expected = rendered.words(&harness.device, &harness.queue);
                let suffix = harness.suffix(grade, cut);
                let resumed = suffix.as_ref().unwrap_or(grade);
                if !matches!(cut, Cut::None) {
                    harness.run(Some(resumed), ScreenProbe::steps(cut), Some(&rendered));
                    exact(
                        case,
                        "screen warm AP1/flags",
                        &expected,
                        &read_words(&harness.device, &harness.queue, &harness.probe.output),
                    );
                    exact(
                        case,
                        "screen warm ACES",
                        &target_expected,
                        &rendered.words(&harness.device, &harness.queue),
                    );
                }
                drop(expected);
                drop(target_expected);
                let order = if i % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                };
                for reuse in order {
                    let timing = harness.run(
                        Some(if reuse { resumed } else { grade }),
                        if reuse { ScreenProbe::steps(cut) } else { None },
                        Some(&rendered),
                    );
                    if reuse {
                        warm.push(timing.completion_ms);
                    } else {
                        full.push(timing.completion_ms);
                        encode.push(timing.encode_ms);
                    }
                }
                full_processing.push(harness.run(Some(grade), None, None).completion_ms);
                aces.push(harness.run(None, None, Some(&rendered)).completion_ms);
                if !matches!(cut, Cut::None) {
                    let start = Instant::now();
                    harness.build(grade, cut);
                    harness.run(Some(resumed), ScreenProbe::steps(cut), Some(&rendered));
                    cold.push(start.elapsed().as_secs_f64() * 1000.);
                }
                harness.sample_memory("completed_edit");
            }
            println!(
                "screen_case kind={kind} size={:?} corrected={corrected} target={target:?} case={case} cut={cut:?} fresh_edits=3 full_processing_ms={:.3} aces_only_ms={:.3} fresh_processing_aces_ms={:.3} full_processing_aces_ms={:.3} warm_processing_aces_ms={:.3} rebuild_processing_aces_ms={} cpu_encode_ms={:.3} warm_reusable={} peak_sampled_metal_bytes={} timing=encode_submit_completion excludes=upload_compilation_readback_reconstruction_surface_present",
                input.size,
                median(&full_processing),
                median(&aces),
                median(&fresh),
                median(&full),
                median(&warm),
                if cold.is_empty() {
                    "-".to_owned()
                } else {
                    format!("{:.3}", median(&cold))
                },
                median(&encode),
                !matches!(cut, Cut::None),
                harness.peak_resource_bytes
            );
        }
        // An upstream gamma edit invalidates code tables and any response prefix.
        let mut churn = Vec::new();
        for i in 0..3 {
            let roll = RollSettings {
                gamma: 0.60 + i as f64 * 0.002,
                ..RollSettings::new_grade()
            };
            let grade = harness.grade(edited("exposure", 0), roll).unwrap();
            churn.push(
                harness
                    .run(Some(&grade), None, Some(&rendered))
                    .completion_ms,
            );
            harness.sample_memory("upstream_gamma");
        }
        println!(
            "screen_upstream kind={kind} size={:?} corrected={corrected} target={target:?} edit=gamma fresh_edits=3 full_processing_aces_ms={:.3}",
            input.size,
            median(&churn)
        );
    }
}

#[test]
#[ignore = "4K immutable prefix versions retained by deferred and abandoned consumers; not an editor memory budget"]
fn screen_prefix_lifetime() {
    pollster::block_on(async {
        let input = InputRaster::synthetic([3840, 2160]);
        let mut h = ScreenProbe::new(&input, false).await;
        let grade = |gamma| {
            h.grade(
                edited("exposure", 0),
                RollSettings {
                    gamma,
                    ..RollSettings::new_grade()
                },
            )
            .unwrap()
        };
        let first = grade(0.60);
        let second = grade(0.65);
        let third = grade(0.70);
        h.run(Some(&first), None, None);
        let expected = read_words(&h.device, &h.queue, &h.probe.output);
        let bytes = h.probe.prefix.size();
        let baseline = allocated_resource_bytes(&h.device).unwrap();
        let fresh = || {
            h.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("4K immutable prefix version"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };
        let initialize = |probe: &Probe, grade: &GpuPhotoGrade| {
            grade.require_source(&h.source).unwrap();
            let mut encoder = h.device.create_command_encoder(&Default::default());
            probe.encode(&h.device, &mut encoder, &h.source, grade, &[7]);
            h.queue.submit([encoder.finish()])
        };
        initialize(&h.probe, &first);
        let mut deferred = h.device.create_command_encoder(&Default::default());
        h.probe
            .encode(&h.device, &mut deferred, &h.source, &first, &[8]);
        h.probe.prefix = fresh();
        initialize(&h.probe, &second);
        let mut abandoned = h.device.create_command_encoder(&Default::default());
        h.probe
            .encode(&h.device, &mut abandoned, &h.source, &second, &[8]);
        h.probe.prefix = fresh();
        let submitted = initialize(&h.probe, &third);
        h.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: None,
            })
            .unwrap();
        let three = allocated_resource_bytes(&h.device).unwrap();
        assert!(three >= baseline + 2 * bytes as usize);
        drop(abandoned);
        h.device.poll(wgpu::PollType::Poll).unwrap();
        let two = allocated_resource_bytes(&h.device).unwrap();
        assert!(three - two >= bytes as usize);
        let submitted = h.queue.submit([deferred.finish()]);
        h.device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submitted),
                timeout: None,
            })
            .unwrap();
        let one = allocated_resource_bytes(&h.device).unwrap();
        assert!(two - one >= bytes as usize);
        exact(
            "4K immutable prefix",
            "late consumer after newer initialization",
            &expected,
            &read_words(&h.device, &h.queue, &h.probe.output),
        );
        println!(
            "screen_prefix_lifetime prefix_bytes={bytes} baseline_metal_bytes={baseline} three_versions_metal_bytes={three} after_abandon_bytes={two} after_completion_bytes={one} excludes=cpu_driver_nonresource_memory"
        );
    });
}

#[test]
#[ignore = "screen-sized full processing/ACES and reusable-cut baseline; explicit host limits; not onscreen latency"]
fn screen_working_set_cost() {
    let sizes: &[[u32; 2]] = match std::env::var("ANTITONE_SCREEN_PERF_SIZE").as_deref() {
        Ok("1080p") => &[[1920, 1080]],
        Ok("4k") => &[[3840, 2160]],
        Err(std::env::VarError::NotPresent) => &[[1920, 1080], [3840, 2160]],
        other => panic!("invalid ANTITONE_SCREEN_PERF_SIZE: {other:?}"),
    };
    #[cfg(feature = "raw")]
    let raw =
        std::env::var_os("ANTITONE_SCREEN_RAW").map(|_| crate::raw_test_support::train_raw::load());
    for &size in sizes {
        for corrected in [false, true] {
            #[cfg(feature = "raw")]
            if let Some((scan, calibration)) = &raw {
                pollster::block_on(measure(
                    InputRaster::raw(scan, *calibration, size),
                    corrected,
                    "raw_nearest_centre_crop",
                ));
                continue;
            }
            pollster::block_on(measure(
                InputRaster::synthetic(size),
                corrected,
                "synthetic",
            ));
        }
    }
}
