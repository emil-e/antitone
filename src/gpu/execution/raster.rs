//! Source interpretation, photographic grade, original repairs and ordered pixel stages.
use crate::{
    Error,
    acquisition::{Plan, Request as AcquisitionRequest, Sampling},
    frame_raster::FrameRaster,
    geometry::Composition,
    gpu::{ProcessingInput, ProcessingSource, pixel_pipeline::Prepared, raster::Raster},
    renderer::GpuRenderer,
};

#[cfg(all(test, target_os = "macos", feature = "render"))]
#[path = "raster_tests.rs"]
mod tests;

/// A resident camera grid is the complete representation supplied by its adapter.
/// Original tiles instead supply their acquisition plan, including real-source support.
pub(crate) fn resident_plan(
    source: &ProcessingSource,
    input: &ProcessingInput,
) -> Result<Option<Plan>, Error> {
    input.require_source(source)?;
    let stages = input.pixel_pipeline()?;
    let Some(geometry) = source.geometry() else {
        return if stages.is_empty() {
            Ok(None)
        } else {
            Err(Error::Gpu(
                "pixel processing requires declared raster geometry".into(),
            ))
        };
    };
    let region = geometry.region();
    let size = geometry.size();
    let composition = Composition::new(region, Default::default(), Default::default())?;
    let output = FrameRaster {
        output: size,
        ..FrameRaster::full(composition)?
    };
    Plan::new(AcquisitionRequest {
        output,
        stages: &stages,
        sampling: Sampling::CameraCells { region, size },
    })
    .map(Some)
}

pub(crate) struct Request<'a> {
    pub source: &'a ProcessingSource,
    pub input: &'a ProcessingInput,
    pub plan: Option<&'a Plan>,
}

impl<'a> Request<'a> {
    pub fn new(
        source: &'a ProcessingSource,
        input: &'a ProcessingInput,
        plan: Option<&'a Plan>,
    ) -> Self {
        Self {
            source,
            input,
            plan,
        }
    }

    pub fn encode(
        self,
        renderer: &GpuRenderer,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        working: &wgpu::Buffer,
    ) -> Result<Option<Prepared>, Error> {
        self.input.require_source(self.source)?;
        if let Some(plan) = self.plan {
            let geometry = self.source.geometry().ok_or(Error::InvalidParameter(
                "planned processing requires source geometry",
            ))?;
            let (region, size) = match (plan.input(), plan.sampling()) {
                (crate::acquisition::ProcessingRaster::Originals(region), _) => {
                    (region, [region.width, region.height])
                }
                (_, Sampling::CameraCells { region, size }) => (region, size),
                _ => {
                    return Err(Error::InvalidParameter(
                        "camera raster execution requires original or resident cells",
                    ));
                }
            };
            if geometry.region() != region || geometry.size() != size {
                return Err(Error::InvalidParameter(
                    "processing source layout differs from plan",
                ));
            }
        }
        let raster = self.plan.map(Raster::planned).unwrap_or_else(|| {
            self.source
                .geometry()
                .map(|geometry| Raster::region(geometry.region(), geometry.size()))
                .unwrap_or_else(|| Raster::native([0, 0, self.source.pixel_count(), 1].into()))
        });
        let stages = self
            .plan
            .map(|plan| plan.pipeline().clone())
            .unwrap_or_default();
        let recipe = crate::gpu::evaluation::Recipe::with_stages(self.input, stages);
        let root = recipe.resident(device, renderer, self.source, raster)?;
        let prepared =
            Prepared::from_composition(device, raster, root, None, renderer.evaluation.clone())?;
        prepared.encode_from(device, encoder, &self.source.input, working)?;
        Ok(Some(prepared))
    }
}
