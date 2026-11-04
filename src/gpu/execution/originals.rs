//! Codec-independent bounded original-sample execution and destination-only readback.

use super::{OutputBlock, Request, SampleEncoding};
use crate::gpu::evaluation;
use crate::gpu::rgb16;
use crate::gpu::submission::SubmissionGate;
#[cfg(all(test, target_os = "macos"))]
use crate::scan::Scan;
use crate::{Error, renderer::GpuRenderer, rendering::OutputTarget};

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

#[cfg(test)]
use crate::{
    acquisition::ValidOutput,
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
};
#[cfg(test)]
use crate::{
    acquisition::{Plan, Request as AcquisitionRequest, Sampling, SourceRead, SourceSupport},
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

#[cfg(test)]
mod unit_fixture;
#[cfg(test)]
pub(crate) use unit_fixture::{Mapping, Unit};

/// Bounded readback and optional target encoding after reconstruction.
pub(super) struct Reconstructor {
    #[cfg(test)]
    context: Option<evaluation::Context>,
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
        let bytes = u64::from(TILE[0]) * u64::from(TILE[1]) * 16;
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bounded reconstructed destination"),
            size: bytes,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        reconstruction.context = Some(evaluation::Context::new(
            device,
            evaluation::CachePolicy { generated_bytes: 0 },
        ));
        reconstruction.output = Some(output);
        Ok(reconstruction)
    }

    fn new_output(device: &wgpu::Device, target: Option<OutputTarget>) -> Result<Self, Error> {
        #[cfg(test)]
        let (context, output) = (None, None);
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
            context,
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
        evaluation::encode_reconstruction_test(
            self.context.as_ref().unwrap(),
            encoder,
            (
                texture,
                working,
                unit.mapping.space[0] == 0,
                [unit.mapping.sizes[0], unit.mapping.sizes[1]],
            ),
            self.output.as_ref().unwrap(),
            (
                crate::gpu::evaluation::CellRegion::new(
                    [0, 0],
                    [unit.mapping.sizes[2], unit.mapping.sizes[3]],
                ),
                evaluation::ReconstructionMapping {
                    start: [unit.mapping.origin[0], unit.mapping.origin[1]].map(f64::from),
                    axes: [
                        [unit.mapping.dx[0], unit.mapping.dx[1]],
                        [unit.mapping.dy[0], unit.mapping.dy[1]],
                    ]
                    .map(|a| a.map(f64::from)),
                },
            ),
            unit.support
                .as_deref()
                .filter(|_| unit.mapping.space[1] != 0),
        )?;
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
    pub buffer_allocations: u64,
    pub generated_initializations: u64,
    pub planned_source_layout_changes: u64,
    pub look_payload_bytes: u64,
}

#[cfg(not(all(test, target_os = "macos")))]
#[derive(Default)]
pub(crate) struct Stats {}

pub(crate) fn execute(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    (queue, gate): (&wgpu::Queue, &SubmissionGate),
    request: Request,
    sink: impl FnOnce(&mut RenderBlock<'_>, &mut Complete<'_>) -> Result<(), Error>,
    mut check: impl FnMut() -> Result<(), Error>,
) -> Result<Stats, Error> {
    check()?;
    if matches!(request.encoding, SampleEncoding::Rgb16(_))
        && request.definition.domain().meaning != evaluation::Meaning::Display
    {
        return Err(Error::InvalidParameter(
            "RGB16 requires display-referred processing",
        ));
    }
    if request.definition.domain().finite()?.source()? != request.composition.area()
        || request.definition.domain().grid != evaluation::Grid::axis_aligned([0.; 2], [1.; 2])
    {
        return Err(Error::InvalidParameter(
            "original calculation and output source grid differ",
        ));
    }
    let checks = std::cell::RefCell::new(check);
    let check = || (checks.borrow_mut())();
    #[cfg(all(test, target_os = "macos"))]
    let look_payload_bytes = evaluation::Reference::image_lut_bytes(&request.definition);
    let original = request.definition;
    let target = match request.encoding {
        SampleEncoding::LinearFloat => None,
        SampleEncoding::Rgb16(target) => Some(target),
    };
    let reconstruction = timed!(Setup, Reconstructor::new_output(device, target))?;
    let size = request.composition.size();
    let domain = evaluation::Domain {
        bounds: Some(crate::gpu::evaluation::CellRegion::new([0, 0], size)),
        grid: evaluation::Grid {
            coverage: 8,
            ..evaluation::Grid::from_frame(crate::frame_raster::FrameRaster::full(
                request.composition,
            )?)
        },
        meaning: original.domain().meaning,
    };
    let root = evaluation::reconstruct_complete(original, domain)?;
    let mut executor = renderer.evaluation.executor();
    #[cfg(all(test, target_os = "macos"))]
    let mut stats = Stats {
        look_payload_bytes,
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
                            bounds: [origin[0], origin[1], extent[0], extent[1]].try_into()?,
                            grid: root.domain().grid
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
            let executed = renderer.evaluation.run_sink(
                &mut executor,
                (queue, gate),
                &plan,
                (
                    &|| match check() {
                        Ok(()) => false,
                        Err(error) => {
                            *interrupted.borrow_mut() = Some(error);
                            true
                        }
                    },
                    &|| {},
                ),
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
                stats.buffer_allocations += _executed.buffer_allocations as u64;
                stats.generated_initializations += _executed.initializations as u64;
                for work in plan.tiles.iter().flat_map(|t| &t.fragment.work) {
                    if let evaluation::Work::Upload { output, .. } = work
                        && previous_source_size != Some(output.size())
                    {
                        stats.planned_source_layout_changes += 1;
                        previous_source_size = Some(output.size());
                    }
                }
                stats.readback_bytes +=
                    u64::from(extent[0]) * u64::from(extent[1]) * reconstruction.bytes_per_pixel();
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
            "export_area source_pixels={} destination_pixels={} submissions={} readback_bytes={} job_look_payload_bytes={} evaluation_buffer_allocations={} generated_initializations={}",
            stats.source_pixels,
            stats.destination_pixels,
            stats.submissions,
            stats.readback_bytes,
            stats.look_payload_bytes,
            stats.buffer_allocations,
            stats.generated_initializations
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
