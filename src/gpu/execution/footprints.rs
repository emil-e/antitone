//! Bounded source acquisition at neutral-footprint and supplied-camera sampling boundaries.

use crate::{
    Error,
    frame_raster::FrameRaster,
    geometry::Composition,
    gpu::{GpuFlatField, ProcessingSource, SourceGeometry, SubmissionGate},
    scan::Scan,
};
use std::sync::Arc;
use wgpu::util::DeviceExt;

mod processing;

pub(crate) use processing::Processed;

mod repairs;

mod request;

pub(crate) use request::Request;

mod kernels;

pub(crate) use kernels::{Kernels, float_buffer, uniform};

pub(crate) struct Acquired {
    pub spec: Request,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    pub kernels: Arc<Kernels>,
    pub pixels: wgpu::Buffer,
}

pub(crate) enum Input<'a> {
    Originals {
        scan: &'a Scan,
        field: Option<Arc<GpuFlatField>>,
        requests: Vec<Request>,
    },
    Camera {
        proxy: &'a crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        request: Request,
    },
    Resident {
        source: Arc<ProcessingSource>,
        request: Request,
    },
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    gate: &SubmissionGate,
    renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<Kernels>,
    input: Input<'_>,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Vec<Arc<Acquired>>>, Error> {
    if cancelled() {
        return Ok(None);
    }
    match input {
        Input::Originals {
            scan,
            field,
            requests,
        } => Acquired::acquire_many(
            device, queue, gate, renderer, kernels, scan, field, requests, cancelled,
        ),
        Input::Camera {
            proxy,
            field,
            request,
        } => {
            let source = Acquired::from_camera_proxy(
                device, queue, gate, renderer, kernels, proxy, field, request,
            )?;
            Ok((!cancelled()).then_some(vec![source]))
        }
        Input::Resident { source, request } => {
            let source =
                Acquired::from_processing(device, queue, gate, renderer, kernels, source, request)?;
            Ok((!cancelled()).then_some(vec![source]))
        }
    }
}

fn encode_neutral(
    device: &wgpu::Device,
    renderer: &crate::renderer::GpuRenderer,
    source: &ProcessingSource,
    spec: &Request,
    output: &wgpu::Buffer,
    encoder: &mut wgpu::CommandEncoder,
) -> Result<(), Error> {
    let geometry = source
        .geometry()
        .ok_or(Error::InvalidParameter("neutral source requires geometry"))?;
    let raster = crate::gpu::raster::Raster::region(geometry.region(), geometry.size());
    let root = crate::gpu::evaluation::Neutral {
        parameters: spec.neutral,
        matrix: spec.matrix,
        field: source.flat_field.clone(),
        repairs: spec.repairs.clone(),
    }
    .resident(device, &renderer.camera, source, raster)?;
    crate::gpu::pixel_pipeline::Prepared::from_composition(
        device,
        raster,
        root,
        None,
        renderer.evaluation.clone(),
    )?
    .encode_from(device, encoder, &source.input, output)
}

impl Acquired {
    /// Navigation positives process the already-sampled camera raster, not the originals.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_camera_proxy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<Kernels>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: Request,
    ) -> Result<Arc<Self>, Error> {
        let spec = spec.capture(crate::gpu::execution::source::Identity::proxy(proxy))?;
        let mut frame = FrameRaster::full(Composition::new(
            spec.area,
            Default::default(),
            Default::default(),
        )?)?;
        frame.output = proxy.dimensions();
        if spec.is_inspection()
            || !spec.repairs.0.is_empty()
            || proxy.width.max(proxy.height) > crate::scan::CameraProxy::MAX_EDGE
            || proxy.pixels.len() as u64 != u64::from(proxy.width) * u64::from(proxy.height)
            || !spec
                .program
                .as_ref()
                .is_some_and(|program| program.matches_proxy(proxy).unwrap_or(false))
            || spec.frame() != frame
            || spec.field.as_ref() != field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu(
                "camera proxy does not match thumbnail source raster".into(),
            ));
        }
        let area = spec.area;
        let spec = spec.with_sampling(crate::acquisition::Sampling::CameraCells {
            region: area,
            size: proxy.dimensions(),
        })?;
        let mut camera = ProcessingSource::new_continuous(device, &proxy.pixels, spec.matrix)?;
        camera.set_flat_field(
            field,
            SourceGeometry::new(
                [spec.area.x, spec.area.y, spec.area.width, spec.area.height],
                proxy.dimensions(),
            )?,
        )?;
        let pixels = float_buffer(device, proxy.dimensions())?;
        let mut encoder = device.create_command_encoder(&Default::default());
        encode_neutral(device, &renderer, &camera, &spec, &pixels, &mut encoder)?;
        kernels.cover_proxy(&renderer.evaluation, &mut encoder, &pixels)?;
        renderer
            .evaluation
            .submit(queue, submissions, encoder, || false)?;
        Ok(Arc::new(Self {
            spec,
            pixels,
            renderer,
            kernels,
        }))
    }

    /// Neutralise and compose an already-decoded camera proxy in original source coordinates.
    /// Repaired previews use original-coordinate acquisition, not this reduced-raster adapter.
    pub(crate) fn from_processing(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<Kernels>,
        source: Arc<ProcessingSource>,
        spec: Request,
    ) -> Result<Arc<Self>, Error> {
        if !spec.repairs.0.is_empty() {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
        let spec = if spec.identity.captured() {
            spec
        } else {
            spec.capture(crate::gpu::execution::source::Identity::Resident(
                Arc::downgrade(&source),
            ))?
        };
        let source = source.with_matrix(device, spec.matrix)?;
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        if geometry.region() != [spec.area.x, spec.area.y, spec.area.width, spec.area.height].into()
        {
            return Err(Error::Gpu(
                "preview source geometry differs from the view frame".into(),
            ));
        }
        // Map accepted original-source footprints into the proxy's actual raster. Independent
        // axis scales account for integer-rounded proxy dimensions without re-rounding a crop
        // or assuming its bounding box starts at the texture origin.
        let raster = crate::geometry::SourceRaster::new(spec.area, geometry.size())?;
        let area = spec.area;
        let spec = spec.with_sampling(crate::acquisition::Sampling::CameraProxy {
            region: area,
            size: geometry.size(),
        })?;
        let frame = spec.input_frame()?;
        let point = |p| raster.raster_point(frame.source_point(p)).0;
        let start = point([0., 0.]);
        let x = point([1., 0.]);
        let y = point([0., 1.]);
        let affine = [
            start.map(|v| v as f32),
            std::array::from_fn(|i| (x[i] - start[i]) as f32),
            std::array::from_fn(|i| (y[i] - start[i]) as f32),
        ];
        let working = float_buffer(device, geometry.size())?;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("provisional neutral AP1 acquisition"),
            size: wgpu::Extent3d {
                width: geometry.size()[0],
                height: geometry.size()[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let pixels = float_buffer(device, frame.output)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        encode_neutral(device, &renderer, &source, &spec, &working, &mut encoder)?;
        kernels.copy(&renderer.evaluation, &mut encoder, &working, &texture)?;
        kernels.filter_raster(
            &renderer.evaluation,
            &mut encoder,
            &texture,
            &pixels,
            geometry.size(),
            frame.output,
            affine,
            [0, 0],
            ([0, 0], frame.output),
        )?;
        submissions.submit(queue, [encoder.finish()]);
        Ok(Arc::new(Self {
            spec,
            renderer,
            kernels,
            pixels,
        }))
    }

    // Borrow the worker's authoritative Scan. Destinations share the original traversal
    // while filtering independent exact footprints.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_many(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<Kernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<Request>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        let identity = crate::gpu::execution::source::Identity::camera(scan.source().identity());
        let specs: Vec<_> = specs
            .into_iter()
            .map(|spec| spec.capture(identity.clone()))
            .collect::<Result<_, _>>()?;
        let spec = specs
            .first()
            .ok_or_else(|| Error::Gpu("empty physical-view acquisition".into()))?;
        scan.validate_area(spec.area)?;
        if scan.area != spec.area
            || (if spec.is_inspection() {
                scan.camera_to_ap1 != spec.matrix
            } else {
                !spec
                    .program
                    .as_ref()
                    .is_some_and(|program| program.matches_native(scan).unwrap_or(false))
            })
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
        let batch = crate::acquisition::FootprintBatch::new(
            specs.iter().map(|spec| spec.plan.clone()).collect(),
            Default::default(),
        )?;
        let outputs = specs
            .iter()
            .map(|spec| {
                let crate::acquisition::SourceRead::Originals(bounds) = spec.plan.source_read()
                else {
                    unreachable!("validated original footprint batch")
                };
                Ok((bounds, float_buffer(device, spec.input_frame()?.output)?))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let neutral = crate::gpu::evaluation::Neutral {
            parameters: spec.neutral,
            matrix: spec.matrix,
            field: field.clone(),
            repairs: spec.repairs.clone(),
        }
        .originals(
            device,
            &renderer.camera,
            crate::gpu::evaluation::camera_source(scan)?,
        )?;
        let mut executor = renderer.evaluation.executor();
        // Plans own source traversal and bounded destination submissions; GPU ownership stays
        // within one admitted original unit, with cancellation between every submission.
        for unit in batch.units() {
            if cancelled() {
                return Ok(None);
            }
            let unit = unit?;
            if unit.submissions.is_empty() {
                continue;
            }
            let region = unit.read;
            let size = [region.width, region.height];
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
                usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });

            for (index, windows) in unit.submissions.into_iter().enumerate() {
                if cancelled() {
                    return Ok(None);
                }
                let filter = |encoder: &mut wgpu::CommandEncoder| -> Result<(), Error> {
                    for work in &windows {
                        let destination = work.destination;
                        kernels.filter(
                            &renderer.evaluation,
                            encoder,
                            &texture,
                            &outputs[destination].1,
                            specs[destination].input_frame()?,
                            [
                                region.x as i32 - outputs[destination].0.x as i32,
                                region.y as i32 - outputs[destination].0.y as i32,
                            ],
                            work.window,
                        )?;
                    }
                    Ok(())
                };
                if index == 0 {
                    let evaluation = renderer.evaluation.request(
                        neutral.clone(),
                        crate::gpu::evaluation::Region {
                            bounds: region,
                            grid: neutral.domain.grid,
                        },
                        crate::gpu::evaluation::Budget {
                            tile: size,
                            scratch_bytes: (32 << 20).min(device.limits().max_buffer_size),
                        },
                    )?;
                    let plan = evaluation
                        .plans()
                        .next()
                        .ok_or(Error::InvalidParameter("empty source acquisition"))??;
                    if executor
                        .run_sink(
                            device,
                            queue,
                            submissions,
                            &plan,
                            &cancelled,
                            |encoder, _, pixels| {
                                kernels.copy(&renderer.evaluation, encoder, pixels, &texture)?;
                                filter(encoder)
                            },
                        )?
                        .is_none()
                    {
                        return Ok(None);
                    }
                } else {
                    let mut encoder = device.create_command_encoder(&Default::default());
                    filter(&mut encoder)?;
                    if renderer
                        .evaluation
                        .submit(queue, submissions, encoder, &cancelled)?
                        .is_none()
                    {
                        return Ok(None);
                    }
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
