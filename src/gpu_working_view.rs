//! Immutable physical-view acquisition and rendering. Originals remain reference/export authority.
use super::*;
use crate::{
    frame_raster::FrameRaster, geometry::TextureMapping, local_adjustments::PhotoGradeSnapshot,
    rendering::OutputTarget, scan::Scan, viewer::GpuImage,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use wgpu::util::DeviceExt;
#[path = "gpu_working_view_composite.rs"]
mod composite;
#[path = "gpu_repair_reconstruction.rs"]
mod repair_reconstruction;

#[derive(Clone)]
pub(crate) struct ViewSourceSpec {
    pub authority: u64,
    pub frame: FrameRaster,
    pub(crate) repairs: super::repairs::Snapshot,
    area: crate::raw::Rect,
    matrix: DMat3,
    neutral: ProcessingParameters,
    inspection: bool,
    field: Option<Arc<crate::flat_field::FlatField>>,
}

impl PartialEq for ViewSourceSpec {
    fn eq(&self, other: &Self) -> bool {
        self.authority == other.authority
            && self.frame == other.frame
            && self.repairs == other.repairs
            && self.area == other.area
            && self.matrix == other.matrix
            && self.inspection == other.inspection
            && self.field == other.field
            && bytemuck::bytes_of(&self.neutral) == bytemuck::bytes_of(&other.neutral)
    }
}

impl ViewSourceSpec {
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        let mut other = other.clone();
        other.frame = self.frame;
        other.repairs = self.repairs.clone();
        self == &other
    }

    pub(crate) fn with_repairs(mut self, repairs: super::repairs::Snapshot) -> Self {
        self.repairs = repairs;
        self
    }

    pub(crate) fn underlay(&self) -> Result<Self, Error> {
        let composition = self.frame.composition;
        let mut frame = FrameRaster::full(composition)?;
        // This bounds only coarse navigation coverage, never the selected detail raster.
        let scale = (f64::from(frame.output[0].max(frame.output[1])) / 1024.).max(1.);
        frame.output = frame.output.map(|n| (f64::from(n) / scale).ceil() as u32);
        Ok(Self {
            frame,
            ..self.clone()
        })
    }

    // Calibration is already normalized by the correction owner, exactly as for the precise path.
    pub(crate) fn new(
        authority: u64,
        frame: FrameRaster,
        calibration: Calibration,
        roll: RollSettings,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        let neutral = ProcessingParameters::positive(
            calibration,
            roll.inversion_only(),
            PhotoSettings::default(),
            matrix,
        )?;
        Self::with_parameters(authority, frame, matrix, neutral, field, false)
    }

    pub(crate) fn inspection(
        authority: u64,
        frame: FrameRaster,
        matrix: DMat3,
        field: Option<Arc<crate::flat_field::FlatField>>,
    ) -> Result<Self, Error> {
        Self::with_parameters(
            authority,
            frame,
            matrix,
            ProcessingParameters::source(),
            field,
            true,
        )
    }

    pub(crate) fn is_inspection(&self) -> bool {
        self.inspection
    }

    fn with_parameters(
        authority: u64,
        frame: FrameRaster,
        matrix: DMat3,
        neutral: ProcessingParameters,
        field: Option<Arc<crate::flat_field::FlatField>>,
        inspection: bool,
    ) -> Result<Self, Error> {
        let area = frame.composition.area();
        frame.bounds()?;
        let a = frame.affine()?;
        let determinant = a[1][0] * a[2][1] - a[1][1] * a[2][0];
        if !determinant.is_finite() || determinant == 0. {
            return Err(Error::Gpu(
                "physical-view footprint exceeds GPU precision".into(),
            ));
        }
        SourceGeometry::new(
            [area.x, area.y, area.width, area.height],
            [area.width, area.height],
        )?;
        Ok(Self {
            authority,
            frame,
            repairs: Default::default(),
            area,
            matrix,
            neutral,
            field,
            inspection,
        })
    }
}

pub(crate) fn colour_shader() -> String {
    [
        ProcessingKernel::shader(true),
        include_str!("shaders/view_geometry.wgsl").to_owned(),
        include_str!("shaders/view_colour.wgsl").to_owned(),
    ]
    .join("\n")
}

pub(crate) struct ViewKernels {
    grade: wgpu::ComputePipeline,
    inspection: wgpu::ComputePipeline,
    filter: wgpu::ComputePipeline,
    copy: wgpu::RenderPipeline,
    compose: wgpu::RenderPipeline,
}

impl ViewKernels {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compute = |label, source: String, entry| {
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: None,
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let inspection = compute(
            "physical-view linear source inspection",
            [
                ProcessingKernel::shader(true),
                include_str!("shaders/view_geometry.wgsl").to_owned(),
                include_str!("shaders/view_inspection.wgsl").to_owned(),
            ]
            .join("\n"),
            "inspection_main",
        );
        let grade = compute("stable physical-view colour", colour_shader(), "view_main");
        let filter = compute(
            "original-cell physical-view acquisition",
            [
                include_str!("shaders/footprint.wgsl"),
                include_str!("shaders/view_filter.wgsl"),
            ]
            .join("\n"),
            "main",
        );
        let copy = composite::pipeline(device, "signed/HDR AP1 acquisition copy", COPY);
        let compose = composite::pipeline(
            device,
            "coherent navigation coverage",
            include_str!("shaders/view_composite.wgsl"),
        );
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self {
            grade,
            inspection,
            filter,
            copy,
            compose,
        }))
    }

    fn copy(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        buffer: &wgpu::Buffer,
        texture: &wgpu::Texture,
    ) {
        let size = uniform(
            device,
            bytemuck::cast_slice(&[texture.width(), texture.height(), 0, 0]),
        );
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("neutral AP1 copy resources"),
            layout: &self.copy.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: size.as_entire_binding(),
                },
            ],
        });
        let view = texture.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("neutral AP1 acquisition copy"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.copy);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }

    #[allow(clippy::too_many_arguments)]
    fn filter(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        image: &wgpu::Texture,
        output: &wgpu::Buffer,
        frame: FrameRaster,
        origin: [i32; 2],
        (lo, extent): crate::resample::windows::Window,
    ) -> Result<(), Error> {
        let bounds = frame.bounds()?;
        let affine = frame.affine()?;
        let mut bytes = bytemuck::cast_slice(&[
            bounds.width,
            bounds.height,
            frame.output[0],
            frame.output[1],
            origin[0] as u32,
            origin[1] as u32,
            image.width(),
            image.height(),
        ])
        .to_vec();
        bytes.extend_from_slice(bytemuck::cast_slice(&affine));
        bytes.extend_from_slice(bytemuck::cast_slice(&[0f32; 2]));
        bytes.extend_from_slice(bytemuck::cast_slice(&[lo[0], lo[1], extent[0], extent[1]]));
        let params = uniform(device, &bytes);
        let view = image.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("physical-view footprint resources"),
            layout: &self.filter.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.filter);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(extent[0].div_ceil(8), extent[1].div_ceil(8), 1);
        Ok(())
    }

    fn grade(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        graded: &GradedView,
    ) {
        let Some(grade) = &graded.grade else {
            let view = uniform(device, bytemuck::bytes_of(&graded.view));
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("immutable source-inspection exposure"),
                layout: &self.inspection.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: graded.pixels.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: view.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: graded.source.pixels.as_entire_binding(),
                    },
                ],
            });
            let groups = ((graded.pixels.size() / 16) as u32).div_ceil(64);
            let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.inspection);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
            return;
        };
        let params = uniform(device, bytemuck::bytes_of(&grade.parameters));
        let view = uniform(device, bytemuck::bytes_of(&graded.view));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("immutable physical-view grade resources"),
            layout: &self.grade.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: graded.pixels.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: grade.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: grade.context.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 10,
                    resource: view.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 11,
                    resource: graded.source.pixels.as_entire_binding(),
                },
            ],
        });
        let groups = ((graded.pixels.size() / 16) as u32).div_ceil(64);
        let x = groups.min(device.limits().max_compute_workgroups_per_dimension);
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&self.grade);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(x, groups.div_ceil(x), 1);
    }
}

pub(crate) struct ViewSource {
    pub spec: ViewSourceSpec,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<ViewKernels>,
    pixels: wgpu::Buffer,
}

impl ViewSource {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        spec: ViewSourceSpec,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        Self::acquire_many(
            device,
            queue,
            submissions,
            renderer,
            kernels,
            scan,
            field,
            vec![spec],
            cancelled,
        )
        .map(|sources| sources.map(|mut sources| sources.remove(0)))
    }

    // Borrow the worker's authoritative Scan. Destinations share the original traversal
    // while filtering independent exact footprints.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_many(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<ViewSourceSpec>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let spec = specs
            .first()
            .ok_or_else(|| Error::Gpu("empty physical-view acquisition".into()))?;
        scan.validate_area(spec.area)?;
        if scan.area != spec.area
            || scan.camera_to_ap1 != spec.matrix
            || scan.flat_field() != spec.field.as_ref()
            || spec.field.as_ref() != field.as_ref().map(|f| f.field())
            || specs
                .iter()
                .any(|s| !s.same_source(spec) || s.repairs != spec.repairs)
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        let outputs = specs
            .iter()
            .map(|s| Ok((s.frame.bounds()?, float_buffer(device, s.frame.output)?)))
            .collect::<Result<Vec<_>, Error>>()?;
        let bounds = outputs
            .iter()
            .map(|(bounds, _)| *bounds)
            .reduce(|a, b| {
                let x = a.x.min(b.x);
                let y = a.y.min(b.y);
                crate::raw::Rect {
                    x,
                    y,
                    width: (a.x + a.width).max(b.x + b.width) - x,
                    height: (a.y + a.height).max(b.y + b.height) - y,
                }
            })
            .expect("nonempty acquisition");
        // Only one bounded original unit is submitted at a time; obsolete work cancels between units.
        // Grade changes deliberately do not cancel acquisition of the same neutral source.
        for y in (0..bounds.height).step_by(128) {
            for x in (0..bounds.width).step_by(512) {
                if cancelled() {
                    return Ok(None);
                }
                let size = [512.min(bounds.width - x), 128.min(bounds.height - y)];
                let region = crate::raw::Rect {
                    x: bounds.x + x,
                    y: bounds.y + y,
                    width: size[0],
                    height: size[1],
                };
                let mut windows = Vec::new();
                for (i, (s, (b, _))) in specs.iter().zip(&outputs).enumerate() {
                    let lo = [region.x.max(b.x), region.y.max(b.y)];
                    let hi = [
                        (region.x + region.width).min(b.x + b.width),
                        (region.y + region.height).min(b.y + b.height),
                    ];
                    if (0..2).any(|c| lo[c] >= hi[c]) {
                        continue;
                    }
                    windows.extend(
                        crate::resample::windows::bounded(crate::resample::windows::affine_output(
                            s.frame.affine()?,
                            s.frame.output,
                            [lo[0] - b.x, lo[1] - b.y],
                            [hi[0] - lo[0], hi[1] - lo[1]],
                        ))
                        .into_iter()
                        .map(|window| (i, window)),
                    );
                }
                if windows.is_empty() {
                    continue;
                }
                let originals = scan.tile_region(region, 0, 0, size[0], size[1])?;
                let mut source = ProcessingSource::new_continuous(device, &originals, spec.matrix)?;
                source.set_flat_field(
                    field.clone(),
                    SourceGeometry::new([region.x, region.y, region.width, region.height], size)?,
                )?;
                let working = float_buffer(device, size)?;
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("bounded neutral AP1 acquisition unit"),
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
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let mut windows = windows.into_iter().peekable();
                let mut initialize = true;
                while windows.peek().is_some() {
                    if cancelled() {
                        return Ok(None);
                    }
                    let mut encoder = device.create_command_encoder(&Default::default());
                    if initialize {
                        initialize = false;
                        renderer.kernel.encode(
                            device,
                            &mut encoder,
                            &source,
                            &working,
                            spec.neutral,
                        );
                        renderer.repairs.neutral(
                            device,
                            &mut encoder,
                            &working,
                            &spec.repairs,
                            SourceGeometry::new(
                                [region.x, region.y, region.width, region.height],
                                size,
                            )?,
                        )?;
                        kernels.copy(device, &mut encoder, &working, &texture);
                    }
                    // Share a submission when both filters fit the existing destination-work
                    // bound; a coarse underlay must not add a wait for every original unit.
                    let mut budget = 512 * 128;
                    while let Some(&(destination, window)) = windows.peek() {
                        let cells = window.1[0] * window.1[1];
                        if cells > budget {
                            break;
                        }
                        windows.next();
                        budget -= cells;
                        kernels.filter(
                            device,
                            &mut encoder,
                            &texture,
                            &outputs[destination].1,
                            specs[destination].frame,
                            [
                                region.x as i32 - outputs[destination].0.x as i32,
                                region.y as i32 - outputs[destination].0.y as i32,
                            ],
                            window,
                        )?;
                    }
                    // Bound destination work too: a tiny original ROI may fill a magnified display.
                    // Completion/temporary retirement is off the UI thread, never an interactive wait.
                    device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submissions.submit(queue, [encoder.finish()])),
                            timeout: None,
                        })
                        .map_err(|e| Error::Gpu(e.to_string()))?;
                }
            }
        }
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(
            specs
                .into_iter()
                .zip(outputs)
                .map(|(spec, (_, pixels))| {
                    Arc::new(Self {
                        spec,
                        pixels,
                        renderer: renderer.clone(),
                        kernels: kernels.clone(),
                    })
                })
                .collect(),
        ))
    }
}

struct GradedView {
    source: Arc<ViewSource>,
    // None is the linear source-inspection bypass, never an identity photographic grade.
    grade: Option<Arc<GpuPhotoGrade>>,
    pixels: wgpu::Buffer,
    view: ViewUniform,
    submitted: Mutex<bool>,
}

/// One immutable target-linear raster and its exact logical authority. Viewer/scopes share this.
pub(crate) struct RenderedView {
    pub image: Arc<GpuImage>,
    pub spec: ViewSourceSpec,
    pub processing: ProcessingSignature,
    pub target: OutputTarget,
    graded: Option<Arc<GradedView>>,
    group: Option<crate::renderer::RenderBinding>,
    composite: Option<composite::Composite>,
    submitted: Mutex<bool>,
    complete: Arc<AtomicBool>,
}

impl RenderedView {
    /// Conservative thumbnail cache charge beyond the target image; shared parents are not deduplicated.
    pub(crate) fn photographic_bytes(&self) -> u64 {
        self.group
            .as_ref()
            .map_or(0, |binding| binding.look_bytes())
            + self.graded.as_ref().map_or(0, |graded| {
                graded.source.pixels.size()
                    + graded.pixels.size()
                    + graded.source.spec.repairs.byte_len()
                    + (self.spec.repairs.0.capacity() + graded.source.spec.repairs.0.len()) as u64
                        * std::mem::size_of::<Arc<super::repairs::Patch>>() as u64
                    + graded.grade.as_ref().map_or(0, |grade| {
                        2 * (grade.byte_len() + grade.snapshot().retained_bytes() as u64)
                    })
            })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        calibration: Calibration,
        roll: RollSettings,
        snapshot: PhotoGradeSnapshot,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let expected = ProcessingParameters::positive(
            calibration,
            roll,
            snapshot.global(),
            source.spec.matrix,
        )?;
        if bytemuck::bytes_of(&expected) != bytemuck::bytes_of(&input.parameters())
            || input
                .grade()
                .map_or(!snapshot.locals().is_empty(), |g| g.snapshot() != &snapshot)
        {
            return Err(Error::Gpu(
                "physical-view grade and processing identity differ".into(),
            ));
        }
        if ViewSourceSpec::new(
            source.spec.authority,
            source.spec.frame,
            calibration,
            roll,
            source.spec.matrix,
            source.spec.field.clone(),
        )?
        .with_repairs(source.spec.repairs.clone())
            != source.spec
        {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        if input.repairs != source.spec.repairs {
            return Err(Error::Gpu(
                "physical-view repairs differ from reconstructed originals".into(),
            ));
        }
        let processing = input.signature();
        // Validate the entire candidate even for target-only reuse, before allocating/publishing it.
        GpuPhotoGrade::validate_repaired(
            device,
            calibration,
            roll,
            &snapshot,
            source.spec.matrix,
            source.spec.area,
            input.repairs.bound(),
        )?;
        let graded = previous
            .filter(|p| p.spec == source.spec && p.processing.same_photographic(&processing))
            .and_then(|p| p.graded.clone());
        let graded = match graded {
            Some(graded) => graded,
            None => Arc::new(GradedView {
                pixels: float_buffer(device, source.spec.frame.output)?,
                grade: Some(Arc::new(GpuPhotoGrade::new(
                    device,
                    calibration,
                    roll,
                    snapshot.clone(),
                    source.spec.matrix,
                    source.spec.area,
                )?)),
                view: ViewUniform::new(source.spec.frame, roll, snapshot.global())?,
                source: source.clone(),
                submitted: Mutex::new(false),
            }),
        };
        Self::from_graded(device, source, input, target, graded)
    }

    pub(crate) fn inspection(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let parameters = input.parameters();
        let gain = f64::from(parameters.post[0].x)
            + f64::from(parameters.post_low[0].x)
            + f64::from(parameters.post_tail[0].x);
        let expected = ProcessingParameters::source_with_gain(gain)?;
        if !source.spec.inspection
            || input.grade().is_some()
            || gain <= 0.
            || bytemuck::bytes_of(&expected) != bytemuck::bytes_of(&parameters)
        {
            return Err(Error::Gpu(
                "physical-view source inspection identity differs".into(),
            ));
        }
        let graded = previous
            .filter(|p| p.spec == source.spec && p.processing == input.signature())
            .and_then(|p| p.graded.clone())
            .map(Ok)
            .unwrap_or_else(|| {
                Ok::<_, Error>(Arc::new(GradedView {
                    pixels: float_buffer(device, source.spec.frame.output)?,
                    grade: None,
                    view: ViewUniform {
                        dimensions: [
                            source.spec.frame.output[0],
                            source.spec.frame.output[1],
                            0,
                            0,
                        ],
                        gain: [
                            parameters.post[0].x,
                            parameters.post_low[0].x,
                            parameters.post_tail[0].x,
                            0.,
                        ],
                        ..bytemuck::Zeroable::zeroed()
                    },
                    source: source.clone(),
                    submitted: Mutex::new(false),
                }))
            })?;
        Self::from_graded(device, source, input, target, graded)
    }

    fn from_graded(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        graded: Arc<GradedView>,
    ) -> Result<Arc<Self>, Error> {
        let processing = input.signature();
        let [width, height] = source.spec.frame.output;
        let image = Arc::new(pollster::block_on(GpuImage::new_rendered_shared(
            &source.renderer,
            device,
            width,
            height,
            target,
        ))?);
        let group = source.renderer.render_binding(
            device,
            &graded.pixels,
            [width, height, 1, 0],
            target,
            input.look(),
        )?;
        Ok(Arc::new(Self {
            image,
            spec: source.spec.clone(),
            processing,
            target,
            graded: Some(graded),
            group: Some(group),
            composite: None,
            submitted: Mutex::new(false),
            complete: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub(crate) fn reference(
        image: Arc<GpuImage>,
        spec: ViewSourceSpec,
        processing: ProcessingSignature,
        target: OutputTarget,
    ) -> Arc<Self> {
        Arc::new(Self {
            image,
            spec,
            processing,
            target,
            graded: None,
            group: None,
            composite: None,
            submitted: Mutex::new(true),
            complete: Arc::new(AtomicBool::new(true)),
        })
    }

    pub(crate) fn matches(
        &self,
        spec: &ViewSourceSpec,
        processing: &ProcessingSignature,
        target: OutputTarget,
    ) -> bool {
        self.spec == *spec && self.processing == *processing && self.target == target
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn same_source_as(&self, other: &Self) -> bool {
        self.graded
            .as_ref()
            .zip(other.graded.as_ref())
            .is_some_and(|(a, b)| Arc::ptr_eq(&a.source, &b.source))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn same_grade_as(&self, other: &Self) -> bool {
        self.graded
            .as_ref()
            .zip(other.graded.as_ref())
            .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
    }

    pub(crate) fn in_flight(&self) -> bool {
        *self.submitted.lock().unwrap()
            && (!self.complete.load(Ordering::Acquire) || self.image.look_result().is_none())
    }

    // Submission, not encoder creation, initializes the cache. Abandoned UI passes cannot poison it.
    pub(crate) fn ensure_submitted(
        self: &Arc<Self>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        context: &egui::Context,
    ) {
        let mut submitted = self.submitted.lock().unwrap();
        if *submitted {
            return;
        }
        let mut initialized = self.graded.as_ref().map(|g| g.submitted.lock().unwrap());
        let mut encoder = device.create_command_encoder(&Default::default());
        if let Some(composite) = &self.composite {
            composite.ensure_parents(device, queue, submissions, context);
            composite.encode(&mut encoder, &self.image);
            if self.processing.look_id().is_some() {
                self.image
                    .set_look_check(Some(composite.check(device, &mut encoder, &self.image)));
            }
        } else {
            let graded = self.graded.as_ref().expect("unsubmitted working view");
            if !**initialized.as_ref().unwrap() {
                graded.source.kernels.grade(device, &mut encoder, graded);
            }
            let binding = self.group.as_ref().unwrap();
            self.image.set_look_check(
                binding
                    .encode_look(&graded.source.renderer, device, &mut encoder)
                    .expect("validated physical-view look dispatch"),
            );
            let view = self.image.linear.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("accepted physical-view ACES result"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&graded.source.renderer.target(self.target).pipeline);
            pass.set_bind_group(0, &binding.group, &[]);
            pass.draw(0..3, 0..1);
        }
        // Submitted bindings own their native versions until completion. The callback owns no
        // renderer/queue or GPU-resource lease: it may run under the shared submission gate.
        let complete = self.complete.clone();
        let context = context.clone();
        let repaint = context.clone();
        submissions.submit_with_completion(queue, [encoder.finish()], move || {
            complete.store(true, Ordering::Release);
            context.request_repaint();
        });
        self.image
            .begin_look_check_tracked(submissions, move || repaint.request_repaint());
        if let Some(initialized) = initialized.as_mut() {
            **initialized = true;
        }
        *submitted = true;
    }

    /// Map a new visible frame into the retained result while original support is being replaced.
    pub(crate) fn mapping(&self, frame: FrameRaster) -> TextureMapping {
        composite::mapping(self.spec.frame, frame)
    }
}

fn uniform(device: &wgpu::Device, bytes: &[u8]) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: None,
        contents: bytes,
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn view_bytes(device: &wgpu::Device, size: [u32; 2]) -> Result<u64, Error> {
    let bytes = u64::from(size[0])
        .checked_mul(u64::from(size[1]))
        .and_then(|n| n.checked_mul(16))
        .ok_or_else(|| Error::Gpu("physical-view size overflow".into()))?;
    if size.contains(&0)
        || size
            .iter()
            .any(|v| *v > device.limits().max_texture_dimension_2d)
        || bytes > device.limits().max_storage_buffer_binding_size
        || bytes > device.limits().max_buffer_size
    {
        return Err(Error::Gpu("physical view exceeds GPU limits".into()));
    }
    Ok(bytes)
}

fn float_buffer(device: &wgpu::Device, size: [u32; 2]) -> Result<wgpu::Buffer, Error> {
    let bytes = view_bytes(device, size)?;
    Ok(device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("physical-view signed/HDR AP1"),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    }))
}

fn split_pair(pair: [f64; 2]) -> [[f32; 4]; 3] {
    let h = pair.map(|v| v as f32);
    let l: [f32; 2] = std::array::from_fn(|c| (pair[c] - f64::from(h[c])) as f32);
    let t: [f32; 2] = std::array::from_fn(|c| (pair[c] - f64::from(h[c]) - f64::from(l[c])) as f32);
    [h, l, t].map(|v| [v[0], v[1], 0., 0.])
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    dimensions: [u32; 4],
    origin: [[f32; 4]; 3],
    dx: [[f32; 4]; 3],
    dy: [[f32; 4]; 3],
    gain: [f32; 4],
}

impl ViewUniform {
    fn new(frame: FrameRaster, roll: RollSettings, photo: PhotoSettings) -> Result<Self, Error> {
        let origin = frame.source_point([0.5, 0.5]).0;
        let x = frame.source_point([1.5, 0.5]).0;
        let y = frame.source_point([0.5, 1.5]).0;
        let gain = crate::processing::exposure_multiplier(roll.exposure)?
            * crate::processing::exposure_multiplier(photo.exposure)?;
        let high = gain as f32;
        let low = (gain - f64::from(high)) as f32;
        let tail = (gain - f64::from(high) - f64::from(low)) as f32;
        Ok(Self {
            dimensions: [frame.output[0], frame.output[1], 0, 0],
            origin: split_pair(origin),
            dx: split_pair(std::array::from_fn(|c| x[c] - origin[c])),
            dy: split_pair(std::array::from_fn(|c| y[c] - origin[c])),
            gain: [high, low, tail, 0.],
        })
    }
}
#[cfg(test)]
#[path = "gpu_working_view_editor_tests.rs"]
mod tests;
const COPY: &str = r#"
@group(0) @binding(0) var<storage,read> pixels:array<vec4f>;
@group(0) @binding(1) var<uniform> size:vec4u;
@vertex fn vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4f {
 let p=array<vec2f,3>(vec2f(-1.,-1.),vec2f(3.,-1.),vec2f(-1.,3.))[i];return vec4f(p,0.,1.);
}
@fragment fn fragment(@builtin(position) p:vec4f)->@location(0) vec4f {
 let xy=vec2u(p.xy);return pixels[xy.x+xy.y*size.x];
}"#;
