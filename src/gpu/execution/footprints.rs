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

pub(crate) use request::SourceRequest;

mod storage;

pub(crate) use storage::{float_buffer, uniform};

pub(crate) struct Acquired {
    pub spec: SourceRequest,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    pub pixels: crate::gpu::evaluation::BufferImage,
    pub definition: Arc<crate::gpu::evaluation::ImageNode>,
    pub basis: Arc<crate::gpu::evaluation::ImageNode>,
}

fn neutral_definition(
    source: &ProcessingSource,
    interpretation: crate::gpu::SourceInterpretation,
    repairs: crate::gpu::repairs::Snapshot,
) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
    let geometry = source
        .geometry()
        .ok_or(Error::InvalidParameter("neutral source requires geometry"))?;
    let raster = crate::gpu::raster::Raster::region(geometry.region(), geometry.size());
    let root = crate::gpu::evaluation::Neutral {
        interpretation,
        field: source.flat_field.clone(),
        repairs,
    }
    .resident(source, raster)?;
    let output_domain = crate::gpu::evaluation::Domain {
        grid: crate::gpu::evaluation::Grid {
            coverage: 8,
            ..root.domain().grid
        },
        ..root.domain()
    };
    crate::gpu::evaluation::reconstruct(root, output_domain)
}

impl Acquired {
    pub(crate) fn supplied(&self) -> Result<crate::gpu::evaluation::SuppliedImage, Error> {
        let region = match &self.pixels {
            crate::gpu::evaluation::BufferImage::Produced(value) => value.descriptor.region,
            crate::gpu::evaluation::BufferImage::Submitted(_) => self.spec.input_region()?,
        };
        crate::gpu::evaluation::SuppliedImage::from_buffer(
            self.definition.clone(),
            region,
            self.pixels.clone(),
        )
    }

    /// Navigation positives process the already-sampled camera raster, not the originals.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn camera_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        input: &crate::gpu::ProcessingInput,
    ) -> Result<(Arc<Self>, Arc<Processed>), Error> {
        let program = input.source_program().ok_or(Error::InvalidParameter(
            "camera preview requires captured source interpretation",
        ))?;
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
            || !input.repairs.0.is_empty()
            || !program.matches_proxy(proxy)?
            || spec.frame() != frame
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
        let mut camera = ProcessingSource::new_continuous(device, &proxy.pixels, program.matrix)?;
        camera.set_flat_field(
            field,
            SourceGeometry::new(
                [spec.area.x, spec.area.y, spec.area.width, spec.area.height],
                proxy.dimensions(),
            )?,
        )?;
        let definition = neutral_definition(
            &camera,
            crate::gpu::SourceInterpretation::Photograph(*program),
            input.repairs.clone(),
        )?;
        let (root, plan) =
            Processed::definition(&device.limits(), &spec, definition.clone(), input)?;
        let geometry = camera.geometry().unwrap();
        let native = crate::gpu::pixel_pipeline::Prepared::from_composition(
            device,
            crate::gpu::raster::Raster::region(geometry.region(), geometry.size()),
            definition.clone(),
            renderer.evaluation.clone(),
        )?;
        let pixels = crate::gpu::evaluation::BufferImage::recorded(
            &definition,
            definition.domain().finite()?,
            float_buffer(device, proxy.dimensions())?,
        )?;
        let source = Arc::new(Self {
            pixels,
            spec,
            basis: definition.clone(),
            definition,
            renderer,
        });
        let processed = Processed::record(device, source.clone(), input, root, plan)?;
        Self::submit_preview(device, queue, submissions, source, native, processed)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn processing_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        source: Arc<ProcessingSource>,
        input: &crate::gpu::ProcessingInput,
        spec: SourceRequest,
    ) -> Result<(Arc<Self>, Arc<Processed>), Error> {
        let (spec, definition) = Self::processing_definition(&source, input, spec)?;
        let (root, plan) =
            Processed::definition(&device.limits(), &spec, definition.clone(), input)?;
        let (acquired, native) = Self::record_processing(device, renderer, spec, definition)?;
        let processed = Processed::record(device, acquired.clone(), input, root, plan)?;
        Self::submit_preview(device, queue, submissions, acquired, native, processed)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_processing(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        source: Arc<ProcessingSource>,
        input: &crate::gpu::ProcessingInput,
        spec: SourceRequest,
    ) -> Result<Arc<Self>, Error> {
        let (spec, definition) = Self::processing_definition(&source, input, spec)?;
        let (acquired, native) = Self::record_processing(device, renderer, spec, definition)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        native.encode_from(device, &mut encoder, &source.input, &acquired.pixels)?;
        submissions.submit(queue, [encoder.finish()]);
        native.submitted();
        acquired
            .pixels
            .submitted(&acquired.renderer.evaluation.materializations());
        Ok(acquired)
    }

    fn processing_definition(
        source: &Arc<ProcessingSource>,
        input: &crate::gpu::ProcessingInput,
        spec: SourceRequest,
    ) -> Result<(SourceRequest, Arc<crate::gpu::evaluation::ImageNode>), Error> {
        if !spec.repairs.0.is_empty() {
            return Err(Error::Gpu(
                "repaired previews require original-coordinate acquisition".into(),
            ));
        }
        let spec = if spec.identity.captured() {
            spec
        } else {
            spec.capture(crate::gpu::execution::source::Identity::Resident(
                Arc::downgrade(source),
            ))?
        };
        let interpretation = input.source_program().map_or(
            crate::gpu::SourceInterpretation::Inspection(source.camera_to_ap1),
            |program| crate::gpu::SourceInterpretation::Photograph(*program),
        );
        let source = source.with_matrix(interpretation.matrix())?;
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        if geometry.region() != [spec.area.x, spec.area.y, spec.area.width, spec.area.height].into()
        {
            return Err(Error::Gpu(
                "preview source geometry differs from the view frame".into(),
            ));
        }
        let area = spec.area;
        let spec = spec.with_sampling(crate::acquisition::Sampling::CameraProxy {
            region: area,
            size: geometry.size(),
        })?;
        let input = crate::gpu::raster::Raster::region(geometry.region(), geometry.size());
        let root = crate::gpu::evaluation::Neutral {
            interpretation,
            field: source.flat_field.clone(),
            repairs: spec.repairs.clone(),
        }
        .resident(&source, input)?;
        let output = crate::gpu::evaluation::Domain {
            bounds: None,
            grid: crate::gpu::evaluation::Grid {
                coverage: 8,
                ..crate::gpu::evaluation::Grid::from_frame(spec.plan.output())
            },
            meaning: root.domain().meaning,
        };
        let root = crate::gpu::evaluation::reconstruct(root, output)?;
        Ok((spec, root))
    }

    pub(crate) fn reprocess(
        self: &Arc<Self>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        definition: Arc<crate::gpu::evaluation::ImageNode>,
        input: &crate::gpu::ProcessingInput,
        previous: Option<&Arc<Processed>>,
    ) -> Result<(Arc<Self>, Arc<Processed>), Error> {
        let (root, plan) =
            Processed::definition(&device.limits(), &self.spec, definition.clone(), input)?;
        let region = root
            .region_for(&definition, root.domain().finite()?)?
            .ok_or(Error::InvalidParameter("preview has no neutral dependency"))?;
        if self.supplied()?.covers(&definition, region) {
            let processed =
                Processed::prepare_calculation(device, self.clone(), input, root, plan, previous)?;
            return Ok((self.clone(), processed));
        }
        let (source, native) =
            Self::record_processing(device, self.renderer.clone(), self.spec.clone(), definition)?;
        let processed = Processed::record(device, source.clone(), input, root, plan)?;
        Self::submit_preview(device, queue, submissions, source, native, processed)
    }

    fn submit_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        source: Arc<Self>,
        native: crate::gpu::pixel_pipeline::Prepared,
        mut processed: Processed,
    ) -> Result<(Arc<Self>, Arc<Processed>), Error> {
        let mut encoder = device.create_command_encoder(&Default::default());
        native.encode_result(device, &mut encoder, &source.pixels)?;
        processed.encode(device, &mut encoder);
        submissions.submit(queue, [encoder.finish()]);
        native.submitted();
        processed.stages.as_ref().unwrap().submitted();
        let scope = source.renderer.evaluation.materializations();
        source.pixels.submitted(&scope);
        processed.pixels.submitted(&scope);
        processed.stages = None;
        *processed.submitted.get_mut().unwrap() = true;
        Ok((source, Arc::new(processed)))
    }

    fn record_processing(
        device: &wgpu::Device,
        renderer: Arc<crate::renderer::GpuRenderer>,
        spec: SourceRequest,
        root: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<(Arc<Self>, crate::gpu::pixel_pipeline::Prepared), Error> {
        let raster = crate::gpu::raster::Raster::frame(spec.input_frame()?);
        let prepared = crate::gpu::pixel_pipeline::Prepared::record_composition(
            device,
            raster,
            root.clone(),
            spec.input_region()?,
            renderer.evaluation.clone(),
            device.limits().max_buffer_size.saturating_mul(4),
            Vec::new(),
        )?;
        let pixels = crate::gpu::evaluation::BufferImage::recorded(
            &root,
            spec.input_region()?,
            float_buffer(device, spec.input_frame()?.output)?,
        )?;
        Ok((
            Arc::new(Self {
                spec,
                renderer,
                basis: root.clone(),
                definition: root,
                pixels,
            }),
            prepared,
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn acquire_many(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        renderer.evaluation.require_device(device)?;
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
                scan.camera_to_ap1 != spec.matrix()
            } else {
                !spec
                    .program()
                    .is_some_and(|program| program.matches_native(scan).unwrap_or(false))
            })
            || scan.flat_field() != spec.field.as_ref()
            || spec.field.as_ref() != field.as_ref().map(|f| f.field())
            || specs.iter().any(|s| {
                !s.same_source(spec)
                    || s.repairs != spec.repairs
                    || s.plan.sampling() != crate::acquisition::Sampling::NeutralFootprints
            })
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        let roots = Self::definitions(scan, field, &spec.processing_fixture(), &specs)?;
        let requests = roots
            .iter()
            .cloned()
            .zip(&specs)
            .map(|(root, spec)| {
                let domain = root.domain();
                let region = spec.input_region()?;
                renderer.evaluation.request(
                    root,
                    crate::gpu::evaluation::Region {
                        bounds: region,
                        grid: domain.grid,
                    },
                    crate::gpu::evaluation::Budget {
                        tile: [region.width, region.height],
                        scratch_bytes: 512 << 20,
                    },
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Self::materialize_many(
            queue,
            submissions,
            renderer,
            specs,
            requests,
            scope,
            cancelled,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn materialize_many(
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        specs: Vec<SourceRequest>,
        requests: Vec<crate::gpu::evaluation::Evaluation>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if specs.len() != requests.len() {
            return Err(Error::InvalidParameter("batch omitted a requested image"));
        }
        let roots: Vec<_> = requests
            .iter()
            .map(|request| request.image().clone())
            .collect();
        let futures = renderer
            .evaluation
            .materialize_many(queue, submissions, scope, requests)?;
        // Demand retirement drops only these waiters, not independent consumers of shared work.
        let mut pending = Box::pin(futures_util::future::try_join_all(futures));
        let result = futures_util::future::poll_fn(|cx| {
            if cancelled() {
                return std::task::Poll::Ready(None);
            }
            std::future::Future::poll(pending.as_mut(), cx).map(Some)
        })
        .await;
        let Some(result) = result else {
            return Ok(None);
        };
        let images = match result {
            Ok(images) => images,
            Err(error) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
            Err(error) => return Err(Error::Gpu(error.to_string())),
        };
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(
            specs
                .into_iter()
                .zip(images)
                .zip(roots)
                .map(|((spec, image), definition)| {
                    Ok(Arc::new(Self {
                        spec,
                        basis: definition.clone(),
                        definition,
                        pixels: image.try_into()?,
                        renderer: renderer.clone(),
                    }))
                })
                .collect::<Result<Vec<_>, Error>>()?,
        ))
    }

    pub(crate) fn definitions(
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        input: &crate::gpu::ProcessingInput,
        specs: &[SourceRequest],
    ) -> Result<Vec<Arc<crate::gpu::evaluation::ImageNode>>, Error> {
        use crate::gpu::evaluation as e;
        let spec = specs
            .first()
            .ok_or(Error::InvalidParameter("empty physical-view acquisition"))?;
        let neutral = e::Neutral {
            interpretation: input.source_program().map_or(
                crate::gpu::SourceInterpretation::Inspection(scan.camera_to_ap1),
                |program| crate::gpu::SourceInterpretation::Photograph(*program),
            ),
            field,
            repairs: spec.repairs.clone(),
        }
        .originals(e::camera_source(scan)?)?;
        specs
            .iter()
            .map(|spec| {
                let mut grid = e::Grid::from_frame(spec.plan.output());
                grid.coverage = 8;
                e::reconstruct(
                    neutral.clone(),
                    e::Domain {
                        bounds: None,
                        grid,
                        meaning: e::Meaning::Neutral,
                    },
                )
            })
            .collect()
    }
}
