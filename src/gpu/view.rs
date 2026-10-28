//! Immutable physical-view acquisition and rendering. Originals remain reference/export authority.
use super::*;
#[cfg(test)]
use crate::adjustment_layers::AdjustmentLayerSnapshot;
#[cfg(test)]
use crate::film_emulation::FilmEmulationSettings;
use crate::{
    frame_raster::FrameRaster, geometry::TextureMapping, rendering::OutputTarget, scan::Scan,
    viewer::GpuImage,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(test)]
use wgpu::util::DeviceExt;
mod composite;
mod repair_reconstruction;

mod source_spec;
pub(crate) use source_spec::ViewSourceSpec;

#[cfg(test)]
pub(crate) use crate::gpu::execution::footprints::shader as view_shader;
use crate::gpu::execution::footprints::uniform;

pub(crate) struct ViewKernels {
    acquisition: Arc<crate::gpu::execution::footprints::Kernels>,
    compose: wgpu::RenderPipeline,
}
impl std::ops::Deref for ViewKernels {
    type Target = crate::gpu::execution::footprints::Kernels;
    fn deref(&self) -> &Self::Target {
        &self.acquisition
    }
}
impl ViewKernels {
    pub(crate) async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        let acquisition = crate::gpu::execution::footprints::Kernels::new(device).await?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compose = composite::pipeline(
            device,
            "coherent navigation coverage",
            include_str!("../shaders/view_composite.wgsl"),
        );
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self {
            acquisition,
            compose,
        }))
    }
}

pub(crate) struct ViewSource {
    pub spec: ViewSourceSpec,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<ViewKernels>,
    pixels: wgpu::Buffer,
}

impl ViewSource {
    pub(crate) fn published(self: &Arc<Self>, authority: u64) -> Arc<Self> {
        if self.spec.authority == authority {
            return self.clone();
        }
        let mut spec = self.spec.clone();
        spec.authority = authority;
        Arc::new(Self {
            spec,
            renderer: self.renderer.clone(),
            kernels: self.kernels.clone(),
            pixels: self.pixels.clone(),
        })
    }

    pub(crate) fn input_program(&self) -> Option<&GpuInputProgram> {
        self.spec.program.as_ref()
    }

    fn from_acquired(
        authority: u64,
        kernels: Arc<ViewKernels>,
        acquired: Arc<crate::gpu::execution::footprints::Acquired>,
    ) -> Arc<Self> {
        Arc::new(Self {
            spec: ViewSourceSpec {
                authority,
                request: acquired.spec.clone(),
            },
            renderer: acquired.renderer.clone(),
            kernels,
            pixels: acquired.pixels.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_camera_proxy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: ViewSourceSpec,
    ) -> Result<Arc<Self>, Error> {
        let authority = spec.authority;
        let acquired = crate::gpu::execution::footprints::execute(
            device,
            queue,
            submissions,
            renderer,
            kernels.acquisition.clone(),
            crate::gpu::execution::footprints::Input::Camera {
                proxy,
                field,
                request: spec.request,
            },
            || false,
        )?
        .expect("noncancellable camera acquisition")
        .remove(0);
        Ok(Self::from_acquired(authority, kernels, acquired))
    }

    pub(crate) fn from_processing(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        spec: ViewSourceSpec,
    ) -> Result<Arc<Self>, Error> {
        let authority = spec.authority;
        let acquired = crate::gpu::execution::footprints::execute(
            device,
            queue,
            submissions,
            renderer,
            kernels.acquisition.clone(),
            crate::gpu::execution::footprints::Input::Resident {
                source,
                request: spec.request,
            },
            || false,
        )?
        .expect("noncancellable resident acquisition")
        .remove(0);
        Ok(Self::from_acquired(authority, kernels, acquired))
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
        let authorities: Vec<_> = specs.iter().map(|spec| spec.authority).collect();
        let requests = specs.into_iter().map(|spec| spec.request).collect();
        crate::gpu::execution::footprints::execute(
            device,
            queue,
            submissions,
            renderer,
            kernels.acquisition.clone(),
            crate::gpu::execution::footprints::Input::Originals {
                scan,
                field,
                requests,
            },
            cancelled,
        )
        .map(|sources| {
            sources.map(|sources| {
                sources
                    .into_iter()
                    .zip(authorities)
                    .map(|(source, authority)| {
                        Self::from_acquired(authority, kernels.clone(), source)
                    })
                    .collect()
            })
        })
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
    pub(crate) fn look_lut(&self) -> Option<Arc<super::look::Lut>> {
        self.group.as_ref().and_then(|binding| binding.look_lut())
    }

    /// Thumbnail payload beyond the target image, including shared parents and LUT storage.
    pub(crate) fn photographic_bytes(&self) -> u64 {
        self.group
            .as_ref()
            .map_or(0, |binding| binding.look_bytes())
            + self.graded.as_ref().map_or(0, |graded| {
                graded.source.pixels.size()
                    + graded.pixels.size()
                    + graded.stages.byte_len()
                    + graded.source.spec.repairs.byte_len()
                    + (self.spec.repairs.0.capacity() + graded.source.spec.repairs.0.len()) as u64
                        * std::mem::size_of::<Arc<super::repairs::Patch>>() as u64
                    + graded.grade.as_ref().map_or(0, |grade| {
                        2 * (grade.byte_len() + grade.snapshot().retained_bytes() as u64)
                    })
            })
    }

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
        Self::prepare(device, source, input, target, previous)
    }

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
        Self::prepare(device, source, input, target, previous)
    }

    fn prepare(
        device: &wgpu::Device,
        source: Arc<ViewSource>,
        input: &ProcessingInput,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
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
                graded
                    .source
                    .kernels
                    .grade(device, &mut encoder, &graded.data);
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
        if let Some(graded) = &self.graded {
            graded.stages.submitted();
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
