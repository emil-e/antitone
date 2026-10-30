//! Codec-independent bounded original-sample execution and destination-only readback.

use super::{OutputBlock, Request, SampleEncoding, WorkBudget};
use crate::gpu::{ProcessingInput, SubmissionGate, evaluation, rgb16};
use crate::{
    Error,
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
    renderer::GpuRenderer,
    rendering::{OutputReference, OutputTarget},
};
#[cfg(all(test, target_os = "macos"))]
use crate::{gpu::ProcessingSource, scan::Scan};
use std::sync::Arc;

#[cfg(feature = "export")]
use crate::export_timing::timed;

#[cfg(not(feature = "export"))]
macro_rules! timed {
    ($stage:ident, $body:expr) => {
        $body
    };
}

// Bounded execution granularity, independent of file codec tile layout.
pub(crate) const TILE: [u32; 2] = [128; 2];

pub(crate) type RenderBlock<'a> = dyn FnMut([u32; 2], [u32; 2]) -> Result<OutputBlock, Error> + 'a;

pub(crate) type Complete<'a> = dyn FnMut() -> Result<(), Error> + 'a;

use crate::{
    acquisition::{
        Plan, Request as AcquisitionRequest, Sampling, SourceRead, SourceSupport, ValidOutput,
    },
    geometry::FramePoint,
};
#[cfg(test)]
use wgpu::util::DeviceExt;

#[cfg(all(test, target_os = "macos"))]
#[path = "film_tests.rs"]
mod film_tests;

#[cfg(all(test, target_os = "macos"))]
#[path = "intent_tests.rs"]
mod intent_tests;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Mapping {
    sizes: [u32; 4],
    origin: [f32; 4],
    dx: [f32; 4],
    dy: [f32; 4],
    space: [u32; 4],
}

pub(crate) struct Unit {
    pub bounds: SourceRegion,
    mapping: Mapping,
    support: Option<Vec<u32>>,
}

impl Unit {
    #[cfg(test)]
    pub(crate) fn new(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
    ) -> Result<Self, Error> {
        Self::planned(composition, origin, extent, &Default::default()).map(|(unit, _)| unit)
    }

    #[cfg(test)]
    fn planned(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
        pipeline: &crate::pixel_pipeline::PixelPipeline,
    ) -> Result<(Self, Plan), Error> {
        let plan = Self::plan(
            composition,
            origin,
            extent,
            pipeline,
            WorkBudget {
                source_pixels: u64::MAX,
            },
        )?;
        Ok((Self::from_plan(&plan, origin)?, plan))
    }

    // Geometry and admission precede all source-sized preparation, including coverage masks.
    fn plan(
        composition: Composition,
        origin: [u32; 2],
        extent: [u32; 2],
        pipeline: &crate::pixel_pipeline::PixelPipeline,
        budget: WorkBudget,
    ) -> Result<Plan, Error> {
        if extent.contains(&0) || (0..2).any(|i| extent[i] > TILE[i]) {
            return Err(Error::InvalidParameter(
                "invalid processing destination unit",
            ));
        }
        let raster = FrameRaster::new(
            composition,
            FramePoint(origin.map(f64::from)),
            FramePoint(std::array::from_fn(|i| {
                f64::from(origin[i]) + f64::from(extent[i])
            })),
            extent,
        )?;
        // The source adapter receives the planned read; it does not propagate stage margins.
        let plan = Plan::new(AcquisitionRequest {
            output: raster,
            stages: pipeline,
            sampling: Sampling::Originals {
                reconstruction: SourceSupport([1; 2]),
            },
        })?;
        let SourceRead::Originals(bounds) = plan.source_read() else {
            unreachable!("original-sample processing plan")
        };
        budget.require_source([bounds.width, bounds.height])?;
        Ok(plan)
    }

    fn from_plan(plan: &Plan, origin: [u32; 2]) -> Result<Self, Error> {
        let SourceRead::Originals(bounds) = plan.source_read() else {
            unreachable!("original-sample processing plan")
        };
        let composition = plan.output().composition;
        let extent = plan.output().output;
        let ValidOutput::Reconstruction {
            affine: [start, dx, dy],
        } = plan.valid_output()
        else {
            unreachable!("original-sample output reconstruction")
        };
        let frame = composition.size();
        let edge = (0..2).any(|i| origin[i] == 0 || origin[i] + extent[i] == frame[i]);
        let support = if edge {
            let coverage =
                composition.coverage(FramePoint([0.; 2]), FramePoint(frame.map(f64::from)))?;
            let count = u64::from(bounds.width) * u64::from(bounds.height);
            let len = usize::try_from(count.div_ceil(32))
                .map_err(|_| Error::InvalidParameter("processing mask exceeds address space"))?;
            let mut bits = vec![0u32; len];
            for row in 0..bounds.height {
                for col in 0..bounds.width {
                    let index = u64::from(row) * u64::from(bounds.width) + u64::from(col);
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
            origin: [start[0], start[1], 0., 0.],
            dx: [dx[0], dx[1], 0., 0.],
            dy: [dy[0], dy[1], 0., 0.],
            space: [0, u32::from(support.is_some()), 0, 0],
        };
        Ok(Self {
            bounds,
            mapping,
            support,
        })
    }
}

/// Original-cell affine reconstruction owns its demand mapping and shader, not the planner.
struct Area {
    composition: Composition,
    stages: crate::pixel_pipeline::PixelPipeline,
    budget: WorkBudget,
    unused_texture: wgpu::TextureView,
}

impl evaluation::Operation for Area {
    fn input(&self, output: SourceRegion) -> Result<Option<SourceRegion>, Error> {
        let origin = [output.x, output.y];
        let extent = [output.width, output.height];
        // Admit the complete original read before allocating coverage or repair payloads.
        Unit::plan(self.composition, origin, extent, &self.stages, self.budget)?;
        let plan = Unit::plan(
            self.composition,
            origin,
            extent,
            &Default::default(),
            self.budget,
        )?;
        let SourceRead::Originals(bounds) = plan.source_read() else {
            unreachable!()
        };
        Ok(Some(bounds))
    }

    fn lower(
        &self,
        fragment: &mut evaluation::Fragment,
        input: Option<evaluation::PlannedImage>,
        output: evaluation::PlannedImage,
    ) -> Result<(), Error> {
        let input = input.unwrap();
        let plan = Unit::plan(
            self.composition,
            [output.region.x, output.region.y],
            output.size(),
            &Default::default(),
            self.budget,
        )?;
        let mut unit = Unit::from_plan(&plan, [output.region.x, output.region.y])?;
        if unit.bounds != input.region {
            return Err(Error::InvalidParameter(
                "reconstruction input differs from demand",
            ));
        }
        // The common reconstruction shader reads the buffer for either linear reference.
        // Its legacy texture binding is deliberately unused in this implementation.
        unit.mapping.space[0] = 1;
        let mapping = fragment.uniform(&unit.mapping);
        let support = fragment.resource(evaluation::Storage::Data(
            bytemuck::cast_slice(unit.support.as_deref().unwrap_or(&[u32::MAX])).into(),
        ));
        fragment.work.push(evaluation::Work::Compute {
            code: include_str!("../../shaders/export_area.wgsl").into(),
            entry: "main",
            initialize: None,
            bindings: vec![
                (
                    0,
                    evaluation::Binding::Shared(evaluation::Shared::Texture(
                        self.unused_texture.clone(),
                    )),
                ),
                (1, evaluation::Binding::Read(input.resource)),
                (2, evaluation::Binding::Write(output.resource)),
                (3, mapping),
                (4, evaluation::Binding::Read(support)),
            ],
            groups: [
                output.size()[0].div_ceil(8),
                output.size()[1].div_ceil(8),
                1,
            ],
        });
        Ok(())
    }
}

pub(super) struct Reconstructor {
    #[cfg(test)]
    pipeline: Option<wgpu::ComputePipeline>,
    #[cfg(test)]
    output: Option<wgpu::Buffer>,
    readback: wgpu::Buffer,
    rgb16: Option<(OutputTarget, rgb16::Rgb16, wgpu::Buffer)>,
}

impl Reconstructor {
    #[cfg(test)]
    pub fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Self::new_test(device, None)
    }

    #[cfg(test)]
    fn new_test(device: &wgpu::Device, target: Option<OutputTarget>) -> Result<Self, Error> {
        let mut reconstruction = Self::new_output(device, target)?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-local export area reconstruction"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/export_area.wgsl").into()),
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

        reconstruction.pipeline = Some(pipeline);
        reconstruction.output = Some(output);
        Ok(reconstruction)
    }

    fn new_output(device: &wgpu::Device, target: Option<OutputTarget>) -> Result<Self, Error> {
        #[cfg(test)]
        let (pipeline, output) = (None, None);
        let bytes = u64::from(TILE[0]) * u64::from(TILE[1]) * 16;
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
            #[cfg(test)]
            pipeline,
            #[cfg(test)]
            output,
            readback,
            rgb16,
        })
    }

    #[cfg(test)]
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
            layout: &self.pipeline.as_ref().unwrap().get_bind_group_layout(0),
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
                    resource: self.output.as_ref().unwrap().as_entire_binding(),
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
            pass.set_pipeline(self.pipeline.as_ref().unwrap());
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(
                unit.mapping.sizes[2].div_ceil(8),
                unit.mapping.sizes[3].div_ceil(8),
                1,
            );
        }
        self.encode_pixels(
            device,
            encoder,
            self.output.as_ref().unwrap(),
            unit.mapping.sizes[2] * unit.mapping.sizes[3],
        )
    }

    fn encode_pixels(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        pixels: &wgpu::Buffer,
        count: u32,
    ) -> Result<(), Error> {
        let output = if let Some((_, kernel, packed)) = &self.rgb16 {
            kernel.encode(device, encoder, pixels, packed, count)?;
            packed
        } else {
            pixels
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
        // Shader tests inspect raw failure markers; consumers receive validated blocks.
        match self.read_raw_output(device, submission, extent)? {
            OutputBlock::Linear(pixels) => Ok(pixels),
            _ => Err(Error::Gpu(
                "expected linear reconstruction test output".into(),
            )),
        }
    }

    #[cfg(test)]
    fn read_output(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
        extent: [u32; 2],
    ) -> Result<OutputBlock, Error> {
        let block = self.read_raw_output(device, submission, extent)?;
        block.validate()?;
        Ok(block)
    }

    #[cfg(test)]
    fn read_raw_output(
        &self,
        device: &wgpu::Device,
        submission: wgpu::SubmissionIndex,
        extent: [u32; 2],
    ) -> Result<OutputBlock, Error> {
        let rx = self.arm(extent);
        timed!(
            DeviceWait,
            device.poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10))
            })
        )
        .map_err(|e| Error::Gpu(e.to_string()))?;
        self.mapped_output(extent, rx)
    }

    fn arm(
        &self,
        extent: [u32; 2],
    ) -> std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>> {
        let bytes = u64::from(extent[0]) * u64::from(extent[1]) * self.bytes_per_pixel();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        timed!(
            MapArm,
            self.readback
                .map_async(wgpu::MapMode::Read, 0..bytes, move |result| {
                    let _ = tx.send(result);
                })
        );

        rx
    }

    fn mapped_output(
        &self,
        extent: [u32; 2],
        rx: std::sync::mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    ) -> Result<OutputBlock, Error> {
        let bytes = u64::from(extent[0]) * u64::from(extent[1]) * self.bytes_per_pixel();
        rx.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|e| Error::Gpu(e.to_string()))?
            .map_err(|e| Error::Gpu(e.to_string()))?;
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

#[cfg(all(test, target_os = "macos"))]
#[derive(Default)]
pub(crate) struct Stats {
    pub source_pixels: u64,
    pub destination_pixels: u64,
    pub submissions: u64,
    pub readback_bytes: u64,
    pub peak_unit_payload: u64,
    pub scratch_buffer_allocations: u64,
    pub source_layout_changes: u64,
    pub look_payload_bytes: u64,
    pub retained_repair_payload_bytes: u64,
    pub peak_repair_upload_bytes: u64,
    pub rgb16_payload_bytes: u64,
}

#[cfg(not(all(test, target_os = "macos")))]
#[derive(Default)]
pub(crate) struct Stats {}

// One source bounding rectangle lives until its destination block completes; dimensions may
// change between blocks. No source atlas, full-photo resident buffer or speculative cache.
pub(crate) fn execute(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    sink: impl FnOnce(&mut RenderBlock<'_>, &mut Complete<'_>) -> Result<(), Error>,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<Stats, Error> {
    check()?;
    if request.output == OutputReference::SceneReferred && request.encoding == SampleEncoding::Rgb16
    {
        return Err(Error::InvalidParameter(
            "RGB16 requires display-referred processing",
        ));
    }
    let checks = std::cell::RefCell::new(check);
    let check = || (checks.borrow_mut())();
    if request.scan.area != request.composition.area()
        || request.scan.flat_field() != request.field.as_ref().map(|f| f.field())
    {
        return Err(Error::Gpu("processing source/recipe mismatch".into()));
    }
    let program = crate::gpu::GpuInputProgram::for_scan(request.scan, request.input)?;
    let photographic = Arc::new(crate::gpu::Grading::new(
        device,
        request.roll,
        request.grade.clone(),
        request.scan.area,
    )?);
    let grade = Arc::new(ProcessingInput::compose(
        device,
        program,
        photographic,
        request.scan.area,
    )?);
    let pipeline = &request.stages;
    let repairs = crate::gpu::repairs::Snapshot::logical(request.repairs.to_vec())?;
    repairs.require_area(request.scan.area)?;
    if !repairs.0.is_empty() {
        grade.validate_repairs(device, repairs.bound())?;
    }
    let mut processing =
        crate::gpu::ProcessingInput::from(grade.clone()).with_look(request.look.clone());
    processing.repairs = repairs.clone();
    let _look_resource = request
        .look
        .as_ref()
        .filter(|_| request.output.display_target().is_some())
        .map(|asset| renderer.look_resource(device, asset))
        .transpose()?;
    let target = match request.encoding {
        SampleEncoding::LinearFloat => None,
        SampleEncoding::Rgb16 => request.output.display_target(),
    };
    let reconstruction = timed!(Setup, Reconstructor::new_output(device, target))?;
    let original = evaluation::original_chain(
        device,
        renderer,
        request.scan,
        request.field.clone(),
        &processing,
        request.output,
        pipeline,
    )?;
    let dummy = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("unused legacy reconstruction binding"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let size = request.composition.size();
    let root = evaluation::ImageNode::new(
        evaluation::Domain {
            bounds: [0, 0, size[0], size[1]].into(),
            grid: evaluation::Grid::from_frame(crate::frame_raster::FrameRaster::full(
                request.composition,
            )?),
            meaning: original.domain.meaning,
        },
        Some(original),
        Area {
            composition: request.composition,
            stages: pipeline.clone(),
            budget: WorkBudget {
                source_pixels: request
                    .budget
                    .source_pixels
                    .min(WorkBudget::for_device(device).source_pixels),
            },
            unused_texture: dummy.create_view(&Default::default()),
        },
    );
    let mut executor = renderer.evaluation.executor();
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
    let mut previous_source_size = None;
    #[cfg(all(test, target_os = "macos", feature = "export"))]
    let clock = crate::gpu_export::observation_clock::timebase();
    #[cfg(all(test, target_os = "macos", feature = "export"))]
    let timestamps = std::cell::RefCell::new(
        clock
            .clone()
            .map(|clock| crate::gpu::timestamp_ledger::TimestampProbe::new(device, clock)),
    );
    sink(
        &mut |origin, extent| {
            check()?;
            if extent.contains(&0) || (0..2).any(|i| extent[i] > TILE[i]) {
                return Err(Error::InvalidParameter(
                    "invalid processing destination unit",
                ));
            }
            #[cfg(all(test, target_os = "macos", feature = "export"))]
            let timing = std::cell::RefCell::new(timestamps.borrow_mut());
            let plan = timed!(
                CoveragePlan,
                renderer
                    .evaluation
                    .request(
                        root.clone(),
                        evaluation::Region {
                            bounds: [origin[0], origin[1], extent[0], extent[1]].into(),
                            grid: root.domain.grid
                        },
                        evaluation::Budget {
                            tile: TILE,
                            scratch_bytes: (32 << 20).min(device.limits().max_buffer_size)
                        }
                    )?
                    .plans()
                    .next()
                    .ok_or(Error::InvalidParameter("empty original evaluation"))?
            )?;
            #[cfg(all(test, target_os = "macos", feature = "export"))]
            {
                executor.timestamps = timing
                    .borrow()
                    .as_ref()
                    .map(|p| {
                        p.processing_queries()
                            .map(|(set, base)| (set.clone(), base))
                    })
                    .transpose()?;
            }
            let interrupted = std::cell::RefCell::new(None);
            let receiver = std::cell::RefCell::new(None);
            let executed = executor.run_sink_with(
                device,
                queue,
                gate,
                &plan,
                || match check() {
                    Ok(()) => false,
                    Err(error) => {
                        *interrupted.borrow_mut() = Some(error);
                        true
                    }
                },
                evaluation::Sink {
                    encode: |encoder: &mut wgpu::CommandEncoder,
                             image: evaluation::PlannedImage,
                             pixels: &wgpu::Buffer| {
                        reconstruction.encode_pixels(device, encoder, pixels, image.cells())?;
                        #[cfg(all(test, target_os = "macos", feature = "export"))]
                        if let Some(probe) = timing.borrow_mut().as_mut() {
                            probe.resolve(encoder);
                            probe.submitting();
                        }
                        Ok(())
                    },
                    submitted: || {
                        *receiver.borrow_mut() = Some(reconstruction.arm(extent));
                        #[cfg(all(test, target_os = "macos", feature = "export"))]
                        if let Some(probe) = timing.borrow_mut().as_mut() {
                            probe.arm();
                        }
                    },
                },
            )?;
            let Some(_executed) = executed else {
                return Err(interrupted.into_inner().unwrap_or(Error::Cancelled));
            };
            let pixels = timed!(
                TileAccess,
                reconstruction.mapped_output(
                    extent,
                    receiver
                        .into_inner()
                        .expect("submitted destination readback")
                )
            )?;
            pixels.validate()?;
            #[cfg(all(test, target_os = "macos", feature = "export"))]
            if let Some(probe) = timing.borrow_mut().as_mut() {
                probe.complete(device)?;
            }
            check()?;
            #[cfg(all(test, target_os = "macos"))]
            {
                stats.source_pixels += _executed.source_cells;
                stats.destination_pixels += u64::from(extent[0]) * u64::from(extent[1]);
                stats.submissions += _executed.submissions as u64;
                stats.scratch_buffer_allocations += _executed.buffer_allocations as u64;
                for work in plan.tiles.iter().flat_map(|t| &t.fragment.work) {
                    if let evaluation::Work::Upload { output, .. } = work {
                        if previous_source_size != Some(output.size()) {
                            stats.source_layout_changes += 1;
                            previous_source_size = Some(output.size());
                        }
                        stats.peak_repair_upload_bytes = stats
                            .peak_repair_upload_bytes
                            .max(repairs.upload_scratch_bytes(output.region));
                    }
                }
                stats.readback_bytes +=
                    u64::from(extent[0]) * u64::from(extent[1]) * reconstruction.bytes_per_pixel();
                let repair_upload = plan
                    .tiles
                    .iter()
                    .flat_map(|t| &t.fragment.resources)
                    .filter_map(|r| {
                        if let evaluation::Storage::Data(bytes) = r {
                            Some(bytes.len() as u64)
                        } else {
                            None
                        }
                    })
                    .sum::<u64>();
                // Explicit bounded payloads; immutable resources and driver overhead are separate.
                stats.peak_unit_payload = stats.peak_unit_payload.max(
                    _executed.scratch_bytes
                        + _executed.source_cells * 16
                        + u64::from(TILE[0])
                            * u64::from(TILE[1])
                            * (16 + if reconstruction.rgb16.is_some() { 8 } else { 0 })
                        + u64::from(extent[0]) * u64::from(extent[1]) * 42
                        + repair_upload,
                );
            }
            Ok(pixels)
        },
        &mut || {
            #[cfg(all(test, target_os = "macos", feature = "export"))]
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
        #[cfg(feature = "export")]
        if let Some(probe) = timestamps.borrow().as_ref() {
            probe.stats.report();
        }
        println!(
            "export_area source_pixels={} destination_pixels={} submissions={} readback_bytes={} unit_payload_upper_bound_bytes={} job_look_payload_bytes={} scratch_buffer_allocations={} retained_repair_payload_bytes={} peak_repair_upload_bytes={} job_rgb16_payload_bytes={}",
            stats.source_pixels,
            stats.destination_pixels,
            stats.submissions,
            stats.readback_bytes,
            stats.peak_unit_payload,
            stats.look_payload_bytes,
            stats.scratch_buffer_allocations,
            stats.retained_repair_payload_bytes,
            stats.peak_repair_upload_bytes,
            stats.rgb16_payload_bytes
        );
    }
    Ok(stats)
}

#[cfg(all(test, target_os = "macos"))]
#[path = "mask_tests.rs"]
mod mask_tests;

#[cfg(all(test, target_os = "macos"))]
#[path = "repair_tests.rs"]
mod repair_tests;

#[cfg(all(test, target_os = "macos"))]
#[path = "tests.rs"]
mod tests;
