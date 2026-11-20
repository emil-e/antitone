//! Immutable physical-view acquisition and rendering. Originals remain reference/export authority.

use crate::Error;
#[cfg(test)]
use crate::adjustment_layers::AdjustmentLayerSnapshot;
use crate::gpu::flat::GpuFlatField;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::source::ProcessingSource;
use crate::gpu::submission::SubmissionGate;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
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

mod composite;
#[cfg(test)]
mod fixtures;
mod image;
pub(crate) mod rendering;
pub mod request;
mod storage;

use crate::gpu::view::request::SourceRequest;
use storage::uniform;

pub struct ViewKernels {
    compose: wgpu::RenderPipeline,
}

impl ViewKernels {
    pub async fn new(device: &wgpu::Device) -> Result<Arc<Self>, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let compose = composite::pipeline(
            device,
            "coherent navigation coverage",
            include_str!("view/composite.wgsl"),
        );
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Arc::new(Self { compose }))
    }
}

pub use image::ViewImage;

/// One immutable target-linear raster and its exact logical authority. Viewer/scopes share this.
pub struct RenderedView {
    pub image: Arc<GpuImage>,
    pub spec: SourceRequest,
    pub capture: PhotographRequest,
    pub target: OutputTarget,
    product: Option<Arc<ViewImage>>,
    group: Option<crate::renderer::RenderBinding>,
    composite: Option<composite::Composite>,
    submitted: Mutex<bool>,
    complete: Arc<AtomicBool>,
}

impl RenderedView {
    pub fn has_correction(&self, field: Option<&Arc<GpuFlatField>>) -> bool {
        self.product.as_ref().is_some_and(|graded| {
            crate::gpu::photograph::PhotographGraph::has_correction(&graded.definition, field)
        })
    }

    #[cfg(test)]
    pub fn processing(&self) -> AuthoredPhotograph {
        self.capture
            .processing(self.image.device_limits())
            .expect("rendered capture was admitted")
    }

    pub fn look_id(&self) -> Option<crate::looks::LookId> {
        self.capture.look_id()
    }

    pub fn look_resources(&self) -> Vec<Arc<super::lut3d::Lut>> {
        self.group
            .as_ref()
            .map_or_else(Vec::new, |binding| binding.look_resources())
    }

    /// Thumbnail payload beyond the target image and shared LUT storage.
    pub fn photographic_bytes(&self) -> u64 {
        self.group
            .as_ref()
            .map_or(0, |binding| binding.retained_bytes())
            + self.product.as_ref().map_or(0, |image| {
                image.byte_len() + crate::gpu::grading::Grading::captured_bytes(&image.definition)
            })
    }

    #[cfg(test)]
    pub fn new(
        device: &wgpu::Device,
        source: Arc<ViewImage>,
        input: &AuthoredPhotograph,
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
            PhotographRequest::Fixture(input.clone()),
        )
    }

    #[cfg(test)]
    pub fn inspection(
        device: &wgpu::Device,
        source: Arc<ViewImage>,
        input: &AuthoredPhotograph,
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
            PhotographRequest::Fixture(input.clone()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_from_retained(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        source: Arc<ViewImage>,
        capture: PhotographRequest,
        field: Option<Arc<GpuFlatField>>,
        target: OutputTarget,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Self>, Error> {
        let (definition, spec) = capture.retained_output(
            &source.renderer,
            &source.definition,
            field,
            &source.spec,
            target,
        )?;
        let image = ViewImage::on_worker(
            source.renderer.clone(),
            source.kernels().clone(),
            spec,
            definition,
            (queue, submissions),
            cancelled,
        )?;
        Self::from_image(device, image, capture, target)
    }

    #[cfg(test)]
    fn prepare(
        device: &wgpu::Device,
        source: Arc<ViewImage>,
        input: &AuthoredPhotograph,
        target: OutputTarget,
        previous: Option<&Arc<Self>>,
        capture: PhotographRequest,
    ) -> Result<Arc<Self>, Error> {
        let product = ViewImage::filter_fixture(
            &source,
            input,
            previous.and_then(|view| view.product.as_ref()),
        )?;
        let endpoint = capture.display(&product.renderer, product.definition.clone(), target)?;
        Self::from_value(device, product, capture, target, endpoint)
    }

    pub fn from_image(
        device: &wgpu::Device,
        product: Arc<ViewImage>,
        capture: PhotographRequest,
        target: OutputTarget,
    ) -> Result<Arc<Self>, Error> {
        let endpoint = product.definition.clone();
        Self::from_value(device, product, capture, target, endpoint)
    }

    fn from_value(
        device: &wgpu::Device,
        product: Arc<ViewImage>,
        capture: PhotographRequest,
        target: OutputTarget,
        endpoint: crate::gpu::evaluation::Value,
    ) -> Result<Arc<Self>, Error> {
        let [width, height] = product.spec.frame().output;
        let image = Arc::new(pollster::block_on(GpuImage::new_rendered_shared(
            &product.renderer,
            device,
            width,
            height,
            target,
        ))?);
        let group = product.renderer.render_binding(
            device,
            product.supplied()?,
            endpoint,
            &image.linear,
        )?;
        Ok(Arc::new(Self {
            image,
            spec: product.spec.clone(),
            capture,
            target,
            product: Some(product),
            group: Some(group),
            composite: None,
            submitted: Mutex::new(false),
            complete: Arc::new(AtomicBool::new(false)),
        }))
    }

    pub fn reference(
        image: Arc<GpuImage>,
        spec: SourceRequest,
        capture: PhotographRequest,
        target: OutputTarget,
    ) -> Arc<Self> {
        Arc::new(Self {
            image,
            spec,
            capture,
            target,
            product: None,
            group: None,
            composite: None,
            submitted: Mutex::new(true),
            complete: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn matches_capture(
        &self,
        spec: &SourceRequest,
        capture: &PhotographRequest,
        target: OutputTarget,
    ) -> bool {
        self.spec == *spec && self.target == target && self.capture == *capture
    }

    #[cfg(test)]
    pub fn matches(
        &self,
        spec: &SourceRequest,
        processing: &AuthoredPhotograph,
        target: OutputTarget,
    ) -> bool {
        self.matches_capture(
            spec,
            &PhotographRequest::Fixture(processing.clone()),
            target,
        )
    }

    pub(crate) fn product(&self) -> Option<&Arc<ViewImage>> {
        self.product.as_ref()
    }

    pub fn uses_image(&self, value: &Arc<ViewImage>) -> bool {
        self.product
            .as_ref()
            .is_some_and(|image| Arc::ptr_eq(image, value))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub fn same_source_as(&self, other: &Self) -> bool {
        use crate::gpu::evaluation::{BufferImage, ImageLayout, ImageValue};
        if !self.same_source_definition_as(other) {
            return false;
        }
        let Some((a, b)) = self.product.as_ref().zip(other.product.as_ref()) else {
            return false;
        };
        if a.pixels == b.pixels
            || crate::gpu::photograph::PhotographGraph::shares_resident_camera(
                &a.definition,
                &b.definition,
            )
        {
            return true;
        }
        let inputs = |view: &ViewImage| -> Vec<Arc<ImageValue>> {
            let BufferImage::Produced(pixels) = &view.pixels else {
                return Vec::new();
            };
            let scope = view.renderer.evaluation.materializations();
            view.definition
                .ancestors()
                .filter(|node| {
                    // A common generated field alone cannot establish source reuse.
                    node.ancestors().any(|input| input.source().is_some())
                })
                .filter_map(|node| {
                    node.output(node.domain().bounds.unwrap_or(pixels.descriptor.region))
                        .ok()
                })
                .filter_map(|output| output.identity().cloned())
                .flat_map(|key| scope.ready::<ImageLayout>(&key))
                .filter(|stored| {
                    pixels
                        .inputs
                        .iter()
                        .any(|input| Arc::ptr_eq(input, &stored.image))
                })
                .map(|stored| stored.image)
                .collect()
        };
        let a = inputs(a);
        let b = inputs(b);
        a.iter()
            .any(|left| b.iter().any(|right| Arc::ptr_eq(left, right)))
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub fn same_source_definition_as(&self, other: &Self) -> bool {
        self.product
            .as_ref()
            .zip(other.product.as_ref())
            .is_some_and(|(a, b)| {
                if !a.spec.same_acquisition(&b.spec) {
                    return false;
                }
                if a.pixels == b.pixels && a.definition.same_definition(&b.definition) {
                    return true;
                }
                let neutral = |view: &ViewImage| {
                    view.definition
                        .ancestors()
                        .find(|node| {
                            node.domain().meaning == crate::gpu::evaluation::Meaning::Neutral
                        })
                        .cloned()
                };
                if let (Some(left), Some(right)) = (neutral(a), neutral(b)) {
                    return left.same_definition(&right);
                }
                a.definition.ancestors().any(|left| {
                    left.domain().meaning == crate::gpu::evaluation::Meaning::Inspection
                        && b.definition
                            .ancestors()
                            .any(|right| left.same_definition(right))
                })
            })
    }

    pub fn in_flight(&self) -> bool {
        *self.submitted.lock().unwrap()
            && (!self.complete.load(Ordering::Acquire) || self.image.look_result().is_none())
    }

    // Submission, not encoder creation, initializes the cache. Abandoned UI passes cannot poison it.
    pub fn ensure_submitted(
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
        let mut initialized = self.product.as_ref().map(|g| g.submitted.lock().unwrap());
        let mut encoder = device.create_command_encoder(&Default::default());
        if let Some(composite) = &self.composite {
            composite.ensure_parents(device, queue, submissions, context);
            composite.encode(&mut encoder, &self.image);
            if self.look_id().is_some() {
                self.image
                    .set_look_check(Some(composite.check(device, &mut encoder, &self.image)));
            }
        } else {
            let graded = self.product.as_ref().expect("unsubmitted working view");
            if !**initialized.as_ref().unwrap() {
                graded.encode(device, &mut encoder);
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
        let started = std::time::Instant::now();
        let repairs = self
            .capture
            .recipe()
            .map_or(0, |recipe| recipe.repairs.len());
        if repairs > 0 {
            log::info!(repairs = repairs; "working_colour_submitted");
        }
        let context = context.clone();
        let repaint = context.clone();
        submissions.submit_with_completion(queue, [encoder.finish()], move || {
            complete.store(true, Ordering::Release);
            if repairs > 0 { log::info!(repairs = repairs, seconds = started.elapsed().as_secs_f64(); "working_colour_complete"); }
            context.request_repaint();
        });
        self.image
            .begin_look_check_tracked(submissions, move || repaint.request_repaint());
        if let Some(initialized) = initialized.as_mut() {
            **initialized = true;
        }
        if let Some(graded) = &self.product {
            if let Some(stages) = &graded.commands {
                stages.submitted();
            }
            graded
                .pixels
                .submitted(&graded.renderer.evaluation.materializations());
        }
        *submitted = true;
    }

    /// Map a new visible frame into the retained result while original support is being replaced.
    pub fn mapping(&self, frame: FrameRaster) -> TextureMapping {
        composite::mapping(self.spec.frame(), frame)
    }
}

#[cfg(test)]
use crate::gpu::view::storage::float_buffer;

#[cfg(test)]
mod tests;
