//! Shared bounded original-sample TIFF/EXR engine; destination-only readback.
use super::*;
use crate::geometry::FramePoint;
use wgpu::util::DeviceExt;

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_film_emulation_tests.rs"]
mod film_tests;

// A rotated 128-square spans at most 128*sqrt(2) source units. Fractional placement
// can cover 183 cells, plus the two real-cell halo edges: 185, not 184.
const MAX_SOURCE_SIDE: u32 = 185;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Mapping {
    sizes: [u32; 4],
    origin: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
    space: [u32; 4],
}

pub(super) struct Unit {
    pub bounds: SourceRegion,
    mapping: Mapping,
    support: Option<Vec<u32>>,
}

impl Unit {
    #[cfg(test)]
    pub fn new(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
    ) -> Result<Self, Error> {
        Self::with_support(
            composition,
            origin,
            extent,
            crate::processing_support::Support::NONE,
        )
    }

    fn with_support(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
        support: crate::processing_support::Support,
    ) -> Result<Self, Error> {
        if extent.contains(&0) || (0..2).any(|i| extent[i] > TILE[i]) {
            return Err(Error::InvalidParameter("invalid export destination unit"));
        }
        let raster = FrameRaster::new(
            composition,
            FramePoint(origin.map(f64::from)),
            FramePoint(std::array::from_fn(|i| {
                f64::from(origin[i]) + f64::from(extent[i])
            })),
            extent,
        )?;
        let area = composition.area();
        let exact = raster.bounds()?;
        // One real-cell halo absorbs native mapping roundoff without omitting support/repairs.
        // Composition maps the developed source at unit pitch; lens development is upstream.
        let support = support.then(crate::processing_support::Support([1, 1]))?;
        let bounds = support.expand(exact, area).validate()?;
        if bounds.width > MAX_SOURCE_SIDE.saturating_add(support.0[0].saturating_mul(2))
            || bounds.height > MAX_SOURCE_SIDE.saturating_add(support.0[1].saturating_mul(2))
        {
            return Err(Error::Gpu(
                "affine source unit exceeds bounded export scratch".into(),
            ));
        }
        let p = raster.source_point([0., 0.]).0;
        let px = raster.source_point([1., 0.]).0;
        let py = raster.source_point([0., 1.]).0;
        let frame = composition.size();
        let edge = (0..2).any(|i| origin[i] == 0 || origin[i] + extent[i] == frame[i]);
        let support = if edge {
            let coverage =
                composition.coverage(FramePoint([0.; 2]), FramePoint(frame.map(f64::from)))?;
            let mut bits = vec![0u32; (bounds.width * bounds.height).div_ceil(32) as usize];
            for row in 0..bounds.height {
                for col in 0..bounds.width {
                    let index = row * bounds.width + col;
                    if coverage.weight([bounds.x + col, bounds.y + row]) > 0. {
                        bits[index as usize / 32] |= 1 << (index % 32);
                    }
                }
            }
            Some(bits)
        } else {
            None
        };
        let mapping = Mapping {
            sizes: [bounds.width, bounds.height, extent[0], extent[1]],
            origin: [
                (p[0] - f64::from(bounds.x)) as f32,
                (p[1] - f64::from(bounds.y)) as f32,
                0.,
                0.,
            ],
            dx: [(px[0] - p[0]) as f32, (px[1] - p[1]) as f32, 0., 0.],
            dy: [(py[0] - p[0]) as f32, (py[1] - p[1]) as f32, 0., 0.],
            space: [0, u32::from(support.is_some()), 0, 0],
        };
        Ok(Self {
            bounds,
            mapping,
            support,
        })
    }
}

pub(super) struct Reconstructor {
    pipeline: wgpu::ComputePipeline,
    output: wgpu::Buffer,
    readback: wgpu::Buffer,
    rgb16: Option<(OutputTarget, rgb16::Rgb16, wgpu::Buffer)>,
}

impl Reconstructor {
    #[cfg(test)]
    pub fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Self::new_output(device, None)
    }

    fn new_output(device: &wgpu::Device, target: Option<OutputTarget>) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-local export area reconstruction"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/export_area.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("cell-local export area reconstruction"),
            layout: None,
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(Error::Gpu(error.to_string()));
        }
        let bytes = u64::from(TILE[0]) * u64::from(TILE[1]) * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded reconstructed destination"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("destination-only reconstruction readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let rgb16 = target
            .map(|target| {
                let kernel = rgb16::Rgb16::new(device, target)?;
                let packed = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("bounded RGB16 output records"),
                    size: u64::from(TILE[0]) * u64::from(TILE[1]) * 8,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
                Ok::<_, Error>((target, kernel, packed))
            })
            .transpose()?;
        Ok(Self {
            pipeline,
            output,
            readback,
            rgb16,
        })
    }

    fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        unit: &Unit,
        texture: &wgpu::Texture,
        working: &wgpu::Buffer,
    ) -> Result<(), Error> {
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile-local backward affine map"),
            contents: bytemuck::bytes_of(&unit.mapping),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        // Exact whole-frame relevance also governs native geometry: roundoff must not
        // admit an unresolved repair outside the captured composition at its outer edge.
        let mask = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("exact contributing source-cell mask"),
            contents: bytemuck::cast_slice(unit.support.as_deref().unwrap_or(&[u32::MAX])),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let view = texture.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("original-to-destination export area bindings"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: working.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: mask.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("export area reconstruction"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(
                unit.mapping.sizes[2].div_ceil(8),
                unit.mapping.sizes[3].div_ceil(8),
                1,
            );
        }
        let count = unit.mapping.sizes[2] * unit.mapping.sizes[3];
        let output = if let Some((_, kernel, packed)) = &self.rgb16 {
            kernel.encode(device, encoder, &self.output, packed, count)?;
            packed
        } else {
            &self.output
        };
        encoder.copy_buffer_to_buffer(
            output,
            0,
            &self.readback,
            0,
            u64::from(count) * self.bytes_per_pixel(),
        );
        Ok(())
    }

    fn bytes_per_pixel(&self) -> u64 {
        if self.rgb16.is_some() { 8 } else { 16 }
    }

    #[cfg(test)]
    fn read(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
        extent: [u32; 2],
    ) -> Result<Vec<[f32; 4]>, Error> {
        match self.read_output(device, submission, extent)? {
            OutputBlock::Linear(pixels) => Ok(pixels),
            _ => Err(Error::Gpu(
                "expected linear reconstruction test output".into(),
            )),
        }
    }

    fn read_output(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
        extent: [u32; 2],
    ) -> Result<OutputBlock, Error> {
        let bytes = u64::from(extent[0]) * u64::from(extent[1]) * self.bytes_per_pixel();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        timed!(
            MapArm,
            self.readback
                .map_async(wgpu::MapMode::Read, 0..bytes, move |result| {
                    let _ = tx.send(result);
                })
        );
        timed!(DeviceWait, {
            device
                .poll(wgpu::PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .map_err(|e| Error::Gpu(e.to_string()))?;
            rx.recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|e| Error::Gpu(e.to_string()))?
                .map_err(|e| Error::Gpu(e.to_string()))
        })?;
        let result = timed!(
            ReadbackCopy,
            self.readback
                .get_mapped_range(0..bytes)
                .map(|mapped| match &self.rgb16 {
                    Some((target, _, _)) => OutputBlock::Rgb16 {
                        target: *target,
                        words: bytemuck::cast_slice::<u8, [u32; 2]>(&mapped).to_vec(),
                    },
                    None =>
                        OutputBlock::Linear(bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()),
                })
                .map_err(|e| Error::Gpu(e.to_string()))
        );
        timed!(ReadbackUnmap, self.readback.unmap());
        result
    }
}

struct Originals {
    size: [u32; 2],
    source: ProcessingSource,
    working: wgpu::Buffer,
    texture: wgpu::Texture,
    binding: Option<crate::renderer::RenderBinding>,
}

impl Originals {
    fn new(
        device: &wgpu::Device,
        renderer: &GpuRenderer,
        request: &Request<'_>,
        bounds: SourceRegion,
    ) -> Result<Self, Error> {
        let size = [bounds.width, bounds.height];
        let source = ProcessingSource::new_continuous(
            device,
            &vec![[0.; 4]; (size[0] * size[1]) as usize],
            request.scan.positive_matrix(&request.input)?,
        )?;
        let working = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded original graded AP1 scratch"),
            size: u64::from(size[0]) * u64::from(size[1]) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let binding = if let Format::Tiff(target) = request.format {
            Some(renderer.render_binding(
                device,
                &working,
                [size[0], size[1], 0, 0],
                target,
                request.look.as_ref(),
            )?)
        } else {
            None
        };
        let texture_size = if binding.is_some() { size } else { [1, 1] };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("bounded rendered original support"),
            size: wgpu::Extent3d {
                width: texture_size[0],
                height: texture_size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        Ok(Self {
            size,
            source,
            working,
            texture,
            binding,
        })
    }
}

#[cfg(all(test, target_os = "macos"))]
#[derive(Default)]
pub(super) struct Stats {
    pub source_pixels: u64,
    pub destination_pixels: u64,
    pub submissions: u64,
    pub readback_bytes: u64,
    pub peak_unit_payload: u64,
    pub source_allocations: u64,
    pub look_payload_bytes: u64,
    pub retained_repair_payload_bytes: u64,
    pub peak_repair_upload_bytes: u64,
    pub rgb16_payload_bytes: u64,
}

#[cfg(not(all(test, target_os = "macos")))]
#[derive(Default)]
pub(super) struct Stats {}

// One source bounding rectangle lives until its destination block completes; dimensions may
// change between blocks. No source atlas, full-photo resident buffer or speculative cache.
pub(super) fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    destination: (&Path, Overwrite),
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<Stats, Error> {
    check()?;
    let checks = std::cell::RefCell::new(check);
    let check = || (checks.borrow_mut())();
    if request.scan.area != request.composition.area()
        || request.scan.flat_field() != request.field.as_ref().map(|f| f.field())
    {
        return Err(Error::Gpu("export source/recipe mismatch".into()));
    }
    let input = match request.input.calibration() {
        Some(base) => request
            .input
            .with_calibration(request.scan.processing_calibration(base)?),
        None => request.input,
    };
    let grade = Arc::new(GpuAdjustmentLayers::new(
        device,
        input,
        request.roll,
        request.grade.clone(),
        request.scan.positive_matrix(&input)?,
        request.scan.area,
    )?);
    let pipeline = crate::pixel_pipeline::PixelPipeline::film(
        request.film_emulation,
        request.scan.area.height,
        request.film_seed,
    )?;
    let support = pipeline.support([1., 1.])?;
    let stage_kernels = (!pipeline.is_empty())
        .then(|| renderer.pixel_kernels(device))
        .transpose()?;
    let repairs = crate::gpu::repairs::Snapshot::logical(request.repairs.to_vec())?;
    repairs.require_area(request.scan.area)?;
    if !repairs.0.is_empty() {
        GpuAdjustmentLayers::validate_repaired(
            device,
            input,
            request.roll,
            &request.grade,
            request.scan.positive_matrix(&input)?,
            request.scan.area,
            repairs.bound(),
        )?;
    }
    let kernel = &renderer.kernel;
    let _look_resource = request
        .look
        .as_ref()
        .filter(|_| matches!(request.format, Format::Tiff(_)))
        .map(|asset| renderer.look_resource(device, asset))
        .transpose()?;
    let target = match request.format {
        Format::Tiff(target) => Some(target),
        Format::Exr(_) => None,
    };
    let reconstruction = timed!(Setup, Reconstructor::new_output(device, target))?;
    let mut originals: Option<Originals> = None;
    #[cfg(all(test, target_os = "macos"))]
    let mut stats = Stats {
        rgb16_payload_bytes: if reconstruction.rgb16.is_some() {
            rgb16::THRESHOLD_BYTES
        } else {
            0
        },
        retained_repair_payload_bytes: repairs.byte_len(),
        look_payload_bytes: _look_resource
            .as_ref()
            .map_or(0, |resource| resource.byte_len()),
        ..Default::default()
    };
    #[cfg(not(all(test, target_os = "macos")))]
    let stats = Stats::default();
    #[cfg(all(test, target_os = "macos"))]
    let clock = super::observation_clock::timebase();
    #[cfg(all(test, target_os = "macos"))]
    let timestamps = std::cell::RefCell::new(
        clock
            .clone()
            .map(|clock| crate::gpu::timestamp_ledger::TimestampProbe::new(device, clock)),
    );
    encode_with_checked(
        request.composition,
        request.format,
        destination,
        |origin, extent| {
            check()?;
            #[cfg(all(test, target_os = "macos"))]
            let mut timing = timestamps.borrow_mut();
            let mut unit = timed!(
                CoveragePlan,
                Unit::with_support(request.composition, origin, extent, support)
            )?;
            unit.mapping.space[0] = u32::from(matches!(request.format, Format::Exr(_)));
            let size = [unit.bounds.width, unit.bounds.height];
            if originals.as_ref().is_none_or(|s| s.size != size) {
                originals = None;
                #[cfg(all(test, target_os = "macos"))]
                {
                    stats.source_allocations += 1;
                }
                originals = Some(timed!(
                    Setup,
                    Originals::new(device, renderer, &request, unit.bounds)
                )?);
            }
            let source = originals.as_mut().unwrap();
            let geometry = SourceGeometry::original_tile(
                [
                    unit.bounds.x,
                    unit.bounds.y,
                    unit.bounds.width,
                    unit.bounds.height,
                ],
                size,
            )?;
            source
                .source
                .set_flat_field(request.field.clone(), geometry)?;
            let pixels = timed!(
                SourcePrepare,
                request.scan.tile_region(
                    request.scan.area,
                    unit.bounds.x - request.scan.area.x,
                    unit.bounds.y - request.scan.area.y,
                    size[0],
                    size[1]
                )
            )?;
            let encoder = timed!(CommandEncode, {
                let mut encoder = device.create_command_encoder(&Default::default());
                #[cfg(all(test, target_os = "macos"))]
                if let Some(probe) = timing.as_ref() {
                    timed!(SourceUpload, source.source.write_continuous(queue, &pixels))?;
                    kernel.encode_grade_with_query(
                        device,
                        &mut encoder,
                        &source.source,
                        &source.working,
                        &grade,
                        probe.processing_queries()?,
                    )?;
                } else {
                    timed!(SourceUpload, source.source.write_continuous(queue, &pixels))?;
                    kernel.encode_grade(
                        device,
                        &mut encoder,
                        &source.source,
                        &source.working,
                        &grade,
                    )?;
                }
                #[cfg(not(all(test, target_os = "macos")))]
                {
                    timed!(SourceUpload, source.source.write_continuous(queue, &pixels))?;
                    kernel.encode_grade(
                        device,
                        &mut encoder,
                        &source.source,
                        &source.working,
                        &grade,
                    )?;
                }
                if !repairs.0.is_empty() {
                    let mut input = crate::gpu::ProcessingInput::from(grade.clone());
                    input.repairs = repairs.clone();
                    renderer.repairs.original(
                        device,
                        &mut encoder,
                        kernel,
                        &source.working,
                        &input,
                        geometry,
                    )?;
                }
                if let Some(kernels) = stage_kernels.as_ref() {
                    let prepared = crate::gpu::pixel_pipeline::Prepared::new(
                        device,
                        &pipeline,
                        crate::gpu::raster::Raster::native(unit.bounds),
                        None,
                    )?;
                    kernels.encode(device, &mut encoder, &prepared, &source.working)?;
                }
                if let (Format::Tiff(target), Some(binding)) =
                    (request.format, source.binding.as_ref())
                {
                    binding.encode_look(renderer, device, &mut encoder)?;
                    let view = source.texture.create_view(&Default::default());
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("per-original ACES before area reconstruction"),
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
                        pass.set_pipeline(&renderer.target(target).pipeline);
                        pass.set_bind_group(0, &binding.group, &[]);
                        pass.draw(0..3, 0..1);
                    }
                }
                reconstruction.encode(
                    device,
                    &mut encoder,
                    &unit,
                    &source.texture,
                    &source.working,
                )?;
                Ok::<_, Error>(encoder)
            })?;
            #[cfg(all(test, target_os = "macos"))]
            let mut encoder = encoder;
            #[cfg(all(test, target_os = "macos"))]
            if let Some(probe) = timing.as_mut() {
                probe.resolve(&mut encoder);
                probe.submitting();
            }
            let submission = timed!(Submit, gate.submit(queue, [encoder.finish()]));
            #[cfg(all(test, target_os = "macos"))]
            if let Some(probe) = timing.as_mut() {
                probe.arm();
            }
            let pixels = timed!(
                TileAccess,
                reconstruction.read_output(device, submission, extent)
            )?;
            #[cfg(all(test, target_os = "macos"))]
            if let Some(probe) = timing.as_mut() {
                probe.complete(device)?;
            }
            check()?;
            #[cfg(all(test, target_os = "macos"))]
            {
                stats.source_pixels += u64::from(size[0]) * u64::from(size[1]);
                stats.destination_pixels += u64::from(extent[0]) * u64::from(extent[1]);
                stats.submissions += 1;
                stats.readback_bytes +=
                    u64::from(extent[0]) * u64::from(extent[1]) * reconstruction.bytes_per_pixel();
                let repair_upload = repairs.upload_scratch_bytes(unit.bounds);
                stats.peak_repair_upload_bytes = stats.peak_repair_upload_bytes.max(repair_upload);
                // Conservative explicit payload upper bound, not sampled process footprint. Count
                // GPU source/working/texture, host input, repair uploads, optional look scratch, GPU
                // destination/readback and host reconstruction/target-encoding temporaries. Some
                // host phases do not overlap; shared constants/LUTs and driver accounting are separate.
                stats.peak_unit_payload = stats.peak_unit_payload.max(
                    u64::from(size[0])
                        * u64::from(size[1])
                        * (16
                            + 16
                            + 16
                            + 16
                            + if !pipeline.is_empty() { 32 } else { 0 }
                            + if request.look.is_some() && matches!(request.format, Format::Tiff(_))
                            {
                                16
                            } else {
                                0
                            })
                        + u64::from(TILE[0])
                            * u64::from(TILE[1])
                            * (32 + if reconstruction.rgb16.is_some() { 8 } else { 0 })
                        + u64::from(extent[0]) * u64::from(extent[1]) * 42
                        + repair_upload
                        + unit
                            .support
                            .as_ref()
                            .map_or(4, |bits| bits.len() as u64 * 8),
                );
            }
            Ok(pixels)
        },
        || {
            #[cfg(all(test, target_os = "macos"))]
            {
                if let Some(probe) = timestamps.borrow_mut().as_mut() {
                    probe.flush(device, queue, gate)?;
                }
                if let Some(clock) = &clock {
                    clock.require_valid()?;
                }
            }
            check()
        },
    )?;
    #[cfg(all(test, target_os = "macos"))]
    {
        if let Some(probe) = timestamps.borrow().as_ref() {
            probe.stats.report();
        }
        println!(
            "export_area source_pixels={} destination_pixels={} submissions={} readback_bytes={} unit_payload_upper_bound_bytes={} job_look_payload_bytes={} source_allocations={} retained_repair_payload_bytes={} peak_repair_upload_bytes={} job_rgb16_payload_bytes={}",
            stats.source_pixels,
            stats.destination_pixels,
            stats.submissions,
            stats.readback_bytes,
            stats.peak_unit_payload,
            stats.look_payload_bytes,
            stats.source_allocations,
            stats.retained_repair_payload_bytes,
            stats.peak_repair_upload_bytes,
            stats.rgb16_payload_bytes
        );
    }
    Ok(stats)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_look_lifetime_tests.rs"]
mod look_lifetime_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_mask_tests.rs"]
mod mask_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_repair_tests.rs"]
mod repair_geometry_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_repair_payload_tests.rs"]
mod repair_payload_tests;
#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
#[path = "gpu_export_stage_oracle_tests.rs"]
mod stage_oracle_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_area_tests.rs"]
mod tests;
