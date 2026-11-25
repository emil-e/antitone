//! Explicit prefix requests used by numerical, spatial and lifetime fixtures.

use super::*;
use crate::gpu::evaluation as e;

mod authoring;

pub(crate) type FixtureDisplay = (
    Arc<crate::photographic::stages::look::Looks>,
    Arc<crate::photographic::stages::display_rendering::Targets>,
);

impl SourceRequest {
    pub fn underlay(&self) -> Result<Self, Error> {
        self.clone().with_frame(underlay_frame(self.frame())?)
    }
}

impl ViewImage {
    pub(super) fn from_neutral_samples(
        spec: SourceRequest,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        pixels: wgpu::Buffer,
    ) -> Self {
        let region = spec.input_region().expect("declared sample grid");
        let definition = e::raster_input(e::Domain {
            bounds: Some(region),
            grid: e::Grid {
                coverage: 8,
                ..e::Grid::from_frame(spec.plan().output())
            },
        });
        Self {
            spec,
            context: renderer.evaluation().clone(),
            fixture_display: Some((renderer.looks().clone(), renderer.drt.clone())),
            kernels,
            definition,
            region,
            pixels: pixels.into(),
            pending: Mutex::new(None),
            argument: None,
        }
    }

    pub(crate) fn from_recorded_fixture(
        context: e::Context,
        display: FixtureDisplay,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: e::Value,
        region: e::CellRegion,
        recorded: e::RecordedImage,
    ) -> Result<Arc<Self>, Error> {
        let mut image = Self::from_recorded(context, kernels, spec, definition, region, recorded)?;
        Arc::get_mut(&mut image).unwrap().fixture_display = Some(display);
        Ok(image)
    }

    pub(super) fn fixture_input(&self) -> Arc<Self> {
        let (spec, supplied) = self.argument.as_ref().expect("explicit fixture argument");
        Self::from_recorded_fixture(
            self.context.clone(),
            self.fixture_display.clone().expect("authored view fixture"),
            self.kernels.clone(),
            spec.clone(),
            supplied.definition().clone(),
            supplied.layout().region,
            e::RecordedImage {
                pixels: supplied.pixels().clone().into(),
                commands: None,
            },
        )
        .unwrap()
    }

    pub fn same_pixels_as(&self, other: &Self) -> bool {
        self.pixels == other.pixels
    }

    pub fn cached_neutral(
        cache: &e::Materializations,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        wanted: &SourceRequest,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
    ) -> Option<Arc<Self>> {
        if !cache.belongs_to(renderer.evaluation())
            || wanted.plan().sampling() != crate::acquisition::Sampling::NeutralFootprints
        {
            return None;
        }
        let root = prefix_definitions(scan, field, std::slice::from_ref(wanted))
            .ok()?
            .remove(0);
        let region = wanted.input_region().ok()?;
        let found = cache.find_ready::<e::ImageLayout>(root.identity()?, |layout| {
            layout.domain == root.domain() && layout.region == region
        })?;
        Self::from_recorded_fixture(
            renderer.evaluation().clone(),
            (renderer.looks().clone(), renderer.drt.clone()),
            kernels,
            wanted.clone(),
            root,
            region,
            e::RecordedImage {
                pixels: found.try_into().ok()?,
                commands: None,
            },
        )
        .ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_processing(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        input: &AuthoredPhotograph,
        spec: SourceRequest,
    ) -> Result<Arc<Self>, Error> {
        renderer.evaluation().require_device(device)?;
        Self::from_processing_with_display(
            renderer.evaluation().clone(),
            (renderer.looks().clone(), renderer.drt.clone()),
            queue,
            kernels,
            source,
            input,
            spec,
        )
    }

    pub(crate) fn from_processing_with_display(
        context: e::Context,
        display: FixtureDisplay,
        queue: &crate::gpu::submission::Queue,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        input: &AuthoredPhotograph,
        spec: SourceRequest,
    ) -> Result<Arc<Self>, Error> {
        let device = context.device().clone();
        let capture = PhotographRequest::Fixture(input.clone());
        let spec = Self::proxy_request(&source, &capture, spec)?;
        let (spec, root) = capture.sampled_prefix(&source, input.field.clone(), spec)?;
        let region = spec.input_region()?;
        let evaluation = context.request(
            root.clone(),
            e::Region {
                bounds: region,
                grid: root.domain().grid,
            },
            e::Tiling {
                tile: [region.width, region.height],
            },
        )?;
        let image = context.record_supplied(evaluation, Vec::new())?;
        let image =
            Self::from_recorded_fixture(context, display, kernels, spec, root, region, image)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        image.encode(&device, &mut encoder);
        queue.submit([encoder.finish()]);
        if let Some(commands) = image.recording().take() {
            commands.submitted();
        }
        image.pixels.submitted(&image.context.materializations());
        Ok(image)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prefix(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Arc<Self>>, Error> {
        Self::prefixes(
            device,
            queue,
            renderer,
            kernels,
            scan,
            field,
            vec![spec],
            cancelled,
        )
        .map(|images| images.map(|mut images| images.remove(0)))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn prefixes(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        let scope = renderer.evaluation().test_materializations(false);
        pollster::block_on(Self::original_prefixes(
            device, queue, renderer, kernels, scan, field, specs, &scope, cancelled,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn original_prefixes(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
        renderer: Arc<crate::render_fixture::RenderFixture>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        scope: &e::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        renderer.evaluation().require_device(device)?;
        let identity = crate::gpu::source::identity::Identity::camera(scan.source().identity());
        let specs = specs
            .into_iter()
            .map(|spec| spec.capture(identity.clone()))
            .collect::<Result<Vec<_>, _>>()?;
        let spec = specs
            .first()
            .ok_or(Error::InvalidParameter("empty prefix request"))?;
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
            || specs.iter().any(|other| {
                !other.same_source(spec)
                    || other.plan().sampling() != crate::acquisition::Sampling::NeutralFootprints
            })
        {
            return Err(Error::Gpu(
                "prefix source authority differs from originals".into(),
            ));
        }
        let definitions = prefix_definitions(scan, field, &specs)?;
        let requests = definitions
            .into_iter()
            .zip(&specs)
            .map(|(root, spec)| {
                let domain = root.domain();
                let region = spec.input_region()?;
                renderer.evaluation().request(
                    root,
                    e::Region {
                        bounds: region,
                        grid: domain.grid,
                    },
                    e::Tiling {
                        tile: [region.width, region.height],
                    },
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let definitions = requests
            .iter()
            .map(|request| request.image().clone())
            .collect::<Vec<_>>();
        let futures = renderer
            .evaluation()
            .materialize_many(queue, scope, requests)?;
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
        specs
            .into_iter()
            .zip(definitions)
            .zip(images)
            .map(|((spec, root), image)| {
                Self::from_recorded_fixture(
                    renderer.evaluation().clone(),
                    (renderer.looks().clone(), renderer.drt.clone()),
                    kernels.clone(),
                    spec,
                    root,
                    image.descriptor.region,
                    e::RecordedImage {
                        pixels: image.try_into()?,
                        commands: None,
                    },
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }

    pub(super) fn filter_fixture(
        source: &Arc<Self>,
        input: &AuthoredPhotograph,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let capture = PhotographRequest::Fixture(input.clone());
        let (root, plan) = capture.filtered(
            &source.context.device().limits(),
            &source.spec,
            source.definition.clone(),
        )?;
        let region = root.domain().finite()?;
        if let Some(previous) = previous {
            let old = previous.definition.output(region)?;
            let wanted = root.output(region)?;
            let same = match (old.identity(), wanted.identity()) {
                (Some(a), Some(b)) => a == b,
                _ => {
                    old.same_value(&wanted)
                        && (previous.pixels == source.pixels
                            || previous
                                .argument
                                .as_ref()
                                .is_some_and(|(_, input)| input.pixels() == &*source.pixels))
                }
            };
            if same {
                return Ok(previous.clone());
            }
        }
        let context = &source.context;
        let image = context.record_supplied(
            Self::evaluation(context, root.clone())?,
            vec![source.supplied()?],
        )?;
        let spec = source.spec.clone().with_plan(plan);
        let mut image = Self::from_recorded_fixture(
            source.context.clone(),
            source
                .fixture_display
                .clone()
                .expect("authored view fixture"),
            source.kernels.clone(),
            spec,
            root,
            region,
            image,
        )?;
        Arc::get_mut(&mut image).unwrap().argument =
            Some((source.spec.clone(), source.supplied()?));
        Ok(image)
    }
}

pub(super) fn prefix_definitions(
    scan: &Scan,
    field: Option<Arc<GpuFlatField>>,
    specs: &[SourceRequest],
) -> Result<Vec<e::Value>, Error> {
    specs
        .iter()
        .map(|spec| {
            PhotographRequest::Fixture(spec.processing_fixture())
                .source_graphs(scan, field.clone(), std::slice::from_ref(spec))
                .map(|mut roots| roots.remove(0))
        })
        .collect()
}
