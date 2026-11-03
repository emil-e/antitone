//! Immutable physical-view acquisition and rendering. Originals remain reference/export authority.

use super::*;
#[cfg(test)]
use crate::adjustment_layers::AdjustmentLayerSnapshot;
#[cfg(test)]
use crate::grain::GrainSettings;
#[cfg(test)]
use crate::scan::Scan;
use crate::{
    frame_raster::FrameRaster, geometry::TextureMapping, rendering::OutputTarget, viewer::GpuImage,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(test)]
use wgpu::util::DeviceExt;

mod capture;
mod composite;
pub(crate) use capture::Capture;

mod repair_reconstruction;

pub(crate) use crate::gpu::execution::footprints::SourceRequest;

use crate::gpu::execution::footprints::uniform;

pub(crate) struct ViewKernels {
    compose: wgpu::RenderPipeline,
}

impl ViewKernels {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compose = composite::pipeline(
            device,
            "coherent navigation coverage",
            include_str!("../shaders/view_composite.wgsl"),
        );
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self { compose }))
    }
}

pub(crate) struct ViewSource {
    kernels: Arc<ViewKernels>,
    data: Arc<crate::gpu::execution::footprints::Acquired>,
}

impl std::ops::Deref for ViewSource {
    type Target = crate::gpu::execution::footprints::Acquired;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl ViewSource {
    /// Primitive fixtures declare already-neutral samples as their actual input boundary.
    #[cfg(test)]
    fn from_neutral_samples(
        spec: SourceRequest,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        pixels: wgpu::Buffer,
    ) -> Self {
        let region = spec.input_region().expect("authored neutral-sample grid");
        let definition = crate::gpu::evaluation::raster_input(crate::gpu::evaluation::Domain {
            bounds: Some(region),
            grid: crate::gpu::evaluation::Grid {
                coverage: 8,
                ..crate::gpu::evaluation::Grid::from_frame(spec.plan.output())
            },
            meaning: crate::gpu::evaluation::Meaning::Neutral,
        });
        Self {
            data: Arc::new(crate::gpu::execution::footprints::Acquired {
                spec: spec.clone(),
                renderer,
                pixels: pixels.into(),
                basis: definition.clone(),
                definition,
            }),
            kernels,
        }
    }

    #[cfg(test)]
    pub(crate) fn cached_neutral(
        cache: &crate::gpu::evaluation::Materializations,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        wanted: &SourceRequest,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
    ) -> Option<Arc<Self>> {
        use crate::gpu::evaluation as e;
        if !cache.belongs_to(&renderer.evaluation)
            || !wanted.repairs.0.is_empty()
            || wanted.plan.sampling() != crate::acquisition::Sampling::NeutralFootprints
        {
            return None;
        }
        let roots = crate::gpu::execution::footprints::Acquired::definitions(
            scan,
            field,
            &wanted.processing_fixture(),
            std::slice::from_ref(wanted),
        )
        .ok()?;
        let root = &roots[0];
        let region = wanted.input_region().ok()?;
        let found = cache.find_ready::<e::ImageLayout>(root.identity()?, |layout| {
            layout.domain == root.domain() && layout.region == region
        })?;
        let data = Arc::new(crate::gpu::execution::footprints::Acquired {
            spec: wanted.clone(),
            renderer,
            pixels: found.try_into().ok()?,
            basis: root.clone(),
            definition: root.clone(),
        });
        Some(Self::from_acquired(kernels, data))
    }

    #[cfg(test)]
    pub(crate) fn same_pixels_as(&self, other: &Self) -> bool {
        self.pixels == other.pixels
    }

    fn from_acquired(
        kernels: Arc<ViewKernels>,
        acquired: Arc<crate::gpu::execution::footprints::Acquired>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kernels,
            data: acquired,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn camera_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        input: &ProcessingInput,
    ) -> Result<(Arc<Self>, Arc<crate::gpu::execution::footprints::Processed>), Error> {
        let (acquired, processed) = crate::gpu::execution::footprints::Acquired::camera_preview(
            device,
            queue,
            submissions,
            renderer,
            proxy,
            field,
            spec,
            input,
        )?;
        Ok((Self::from_acquired(kernels, acquired), processed))
    }

    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(crate) fn from_processing(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        input: &ProcessingInput,
        spec: SourceRequest,
    ) -> Result<Arc<Self>, Error> {
        let acquired = crate::gpu::execution::footprints::Acquired::from_processing(
            device,
            queue,
            submissions,
            renderer,
            source,
            input,
            spec,
        )?;
        Ok(Self::from_acquired(kernels, acquired))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn processing_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        input: &ProcessingInput,
        spec: SourceRequest,
    ) -> Result<(Arc<Self>, Arc<crate::gpu::execution::footprints::Processed>), Error> {
        let (acquired, processed) =
            crate::gpu::execution::footprints::Acquired::processing_preview(
                device,
                queue,
                submissions,
                renderer,
                source,
                input,
                spec,
            )?;
        Ok((Self::from_acquired(kernels, acquired), processed))
    }

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
        spec: SourceRequest,
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn acquire_images(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        specs: Vec<SourceRequest>,
        requests: Vec<crate::gpu::evaluation::Evaluation>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        renderer.evaluation.require_device(device)?;
        let sources = crate::gpu::execution::footprints::Acquired::materialize_many(
            queue,
            submissions,
            renderer,
            specs,
            requests,
            scope,
            cancelled,
        )
        .await?;
        Ok(sources.map(|sources| {
            sources
                .into_iter()
                .map(|source| Self::from_acquired(kernels.clone(), source))
                .collect()
        }))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_many(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        let scope = renderer.evaluation.test_materializations(false);
        pollster::block_on(Self::acquire_originals_fixture(
            device,
            queue,
            submissions,
            renderer,
            kernels,
            scan,
            field,
            specs,
            &scope,
            cancelled,
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn acquire_originals_fixture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        let sources = crate::gpu::execution::footprints::Acquired::acquire_many(
            device,
            queue,
            submissions,
            renderer,
            scan,
            field,
            specs,
            scope,
            cancelled,
        )
        .await?;
        Ok(sources.map(|sources| {
            sources
                .into_iter()
                .map(|source| Self::from_acquired(kernels.clone(), source))
                .collect()
        }))
    }
}

struct GradedView {
    source: Arc<ViewSource>,
    data: Arc<crate::gpu::execution::footprints::Processed>,
}

impl std::ops::Deref for GradedView {
    type Target = crate::gpu::execution::footprints::Processed;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

/// One immutable target-linear raster and its exact logical authority. Viewer/scopes share this.
pub(crate) struct RenderedView {
    pub image: Arc<GpuImage>,
    pub spec: SourceRequest,
    pub capture: Capture,
    pub target: OutputTarget,
    graded: Option<Arc<GradedView>>,
    group: Option<crate::renderer::RenderBinding>,
    composite: Option<composite::Composite>,
    submitted: Mutex<bool>,
    complete: Arc<AtomicBool>,
}

impl RenderedView {
    pub(crate) fn processing(&self) -> ProcessingInput {
        self.capture
            .processing(self.image.device_limits(), &self.spec.repairs)
            .expect("rendered capture was admitted")
    }

    pub(crate) fn look_id(&self) -> Option<crate::looks::LookId> {
        self.capture.look_id()
    }

    pub(crate) fn lut_resources(&self) -> Vec<Arc<super::lut3d::Lut>> {
        self.group
            .as_ref()
            .map_or_else(Vec::new, |binding| binding.lut_resources())
    }

    /// Thumbnail payload beyond the target image, including shared parents and LUT storage.
    pub(crate) fn photographic_bytes(&self) -> u64 {
        self.group
            .as_ref()
            .map_or(0, |binding| binding.reference_bytes())
            + self.graded.as_ref().map_or(0, |graded| {
                graded.source.pixels.size()
                    + crate::gpu::evaluation::Photograph::resident_camera_bytes(
                        &graded.source.definition,
                    )
                    + graded.pixels.size()
                    + graded.stages.as_ref().map_or(0, |stages| stages.byte_len())
                    + graded.source.spec.repairs.byte_len()
                    + (self.spec.repairs.0.capacity() + graded.source.spec.repairs.0.len()) as u64
                        * std::mem::size_of::<Arc<super::repairs::Patch>>() as u64
                    + graded.grade.as_ref().map_or(0, |grade| {
                        2 * (grade.byte_len() + grade.snapshot().retained_bytes() as u64)
                    })
            })
    }

    #[cfg(test)]
    pub(crate) fn new(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        if source.spec.is_inspection() {
            return Err(Error::Gpu("physical-view source is stale".into()));
        }
        Self::prepare(
            device,
            source,
            input,
            target,
            previous,
            Capture::Primitive(input.clone()),
        )
    }

    #[cfg(test)]
    pub(crate) fn inspection(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        if !source.spec.is_inspection() {
            return Err(Error::Gpu(
                "physical-view source inspection identity differs".into(),
            ));
        }
        Self::prepare(
            device,
            source,
            input,
            target,
            previous,
            Capture::Primitive(input.clone()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn reprocess(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        source: Arc<ViewSource>,
        capture: Capture,
        field: Option<Arc<GpuFlatField>>,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
    ) -> Result<(Arc<ViewSource>, Arc<Self>), Error> {
        let input = capture.processing(&device.limits(), &source.spec.repairs)?;
        let program = input.source_program().ok_or(Error::InvalidParameter(
            "positive preview requires captured source interpretation",
        ))?;
        let native = crate::gpu::evaluation::Neutral {
            interpretation: crate::gpu::SourceInterpretation::Photograph(*program),
            field,
            repairs: input.repairs.clone(),
        }
        .recompose_resident(&source.definition)?;
        let prior = previous
            .and_then(|view| view.graded.as_ref())
            .map(|graded| &graded.data);
        let (data, processed) =
            source
                .acquired()
                .reprocess(device, queue, submissions, native, &input, prior)?;
        let source = if Arc::ptr_eq(&data, &source.data) {
            source
        } else {
            ViewSource::from_acquired(source.kernels.clone(), data)
        };
        let rendered =
            Self::from_processed(device, source.clone(), capture, target, processed, previous)?;
        Ok((source, rendered))
    }

    #[cfg(test)]
    fn prepare(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
        capture: Capture,
    ) -> Result<Arc<Self>, Error> {
        let prior = previous.and_then(|p| p.graded.as_ref());
        let data = crate::gpu::execution::footprints::Processed::prepare(
            device,
            source.acquired(),
            input,
            prior.map(|g| &g.data),
        )?;
        let graded = match prior.filter(|p| Arc::ptr_eq(&p.data, &data)) {
            Some(prior) => prior.clone(),
            None => Arc::new(GradedView {
                source: source.clone(),
                data,
            }),
        };
        Self::from_graded(device, source, capture, target, graded)
    }

    fn from_graded(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        capture: Capture,
        target: OutputTarget,
        graded: Arc<GradedView>,
    ) -> Result<Arc<Self>, Error> {
        let [width, height] = source.spec.frame().output;
        let image = Arc::new(pollster::block_on(GpuImage::new_rendered_shared(
            &source.renderer,
            device,
            width,
            height,
            target,
        ))?);
        let group = source.renderer.render_binding(
            device,
            crate::gpu::evaluation::SuppliedImage::from_buffer(
                graded.definition.clone(),
                graded.definition.domain().finite()?,
                graded.pixels.clone(),
            )?,
            target,
            capture.luts(),
            &image.linear,
        )?;
        Ok(Arc::new(Self {
            image,
            spec: source.spec.clone(),
            capture,
            target,
            graded: Some(graded),
            group: Some(group),
            composite: None,
            submitted: Mutex::new(false),
            complete: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub(crate) fn from_processed(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        capture: Capture,
        target: OutputTarget,
        data: Arc<crate::gpu::execution::footprints::Processed>,
        previous: Option<&Arc<Self>>,
    ) -> Result<Arc<Self>, Error> {
        let graded = previous
            .and_then(|view| view.graded.as_ref())
            .filter(|graded| Arc::ptr_eq(&graded.data, &data))
            .cloned()
            .unwrap_or_else(|| {
                Arc::new(GradedView {
                    source: source.clone(),
                    data,
                })
            });
        Self::from_graded(device, source, capture, target, graded)
    }

    pub(crate) fn reference(
        image: Arc<GpuImage>,
        spec: SourceRequest,
        capture: Capture,
        target: OutputTarget,
    ) -> Arc<Self> {
        Arc::new(Self {
            image,
            spec,
            capture,
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
        spec: &SourceRequest,
        processing: &ProcessingInput,
        target: OutputTarget,
    ) -> bool {
        self.spec == *spec
            && self.target == target
            && self.capture.look_id() == processing.look_id()
            && if let Some(graded) = &self.graded {
                graded.matches(
                    &graded.source.renderer.evaluation().device().limits(),
                    processing,
                )
            } else {
                self.processing() == *processing
            }
    }

    pub(crate) fn uses_processed(
        &self,
        value: &Arc<crate::gpu::execution::footprints::Processed>,
    ) -> bool {
        self.graded
            .as_ref()
            .is_some_and(|graded| Arc::ptr_eq(&graded.data, value))
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
            if self.look_id().is_some() {
                self.image
                    .set_look_check(Some(composite.check(device, &mut encoder, &self.image)));
            }
        } else {
            let graded = self.graded.as_ref().expect("unsubmitted working view");
            if !**initialized.as_ref().unwrap() {
                graded.data.encode(device, &mut encoder);
            }
            let binding = self.group.as_ref().unwrap();
            self.image.set_look_check(
                binding
                    .encode(device, &mut encoder)
                    .expect("validated physical-view reference composition"),
            );
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
        if let Some(graded) = &self.graded {
            if let Some(stages) = &graded.stages {
                stages.submitted();
            }
            graded
                .pixels
                .submitted(&graded.source.renderer.evaluation.materializations());
        }
        *submitted = true;
    }

    /// Map a new visible frame into the retained result while original support is being replaced.
    pub(crate) fn mapping(&self, frame: FrameRaster) -> TextureMapping {
        composite::mapping(self.spec.frame(), frame)
    }
}

#[cfg(test)]
use crate::gpu::execution::footprints::float_buffer;

#[cfg(test)]
mod tests;
