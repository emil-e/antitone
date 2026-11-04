//! Source interpretation, photographic grade, original repairs and ordered pixel stages.

use crate::Error;
use crate::acquisition::Plan;
use crate::acquisition::Request as AcquisitionRequest;
use crate::acquisition::Sampling;
use crate::frame_raster::FrameRaster;
use crate::geometry::Composition;
use crate::gpu::pixel_pipeline::Prepared;
use crate::gpu::raster::Raster;
use crate::gpu::source::ProcessingSource;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::renderer::GpuRenderer;

#[cfg(all(test, target_os = "macos", feature = "render"))]
#[path = "raster_tests.rs"]
mod tests;

/// A resident camera grid is the complete representation supplied by its adapter.
/// Original tiles instead supply their acquisition plan, including real-source support.
pub(crate) fn resident_plan(
    source: &ProcessingSource,
    input: &AuthoredPhotograph,
) -> Result<Option<Plan>, Error> {
    input.require_source(source)?;
    let stages = input.pixel_pipeline();
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
    pub input: &'a AuthoredPhotograph,
    pub plan: Option<&'a Plan>,
    /// Supplied sample arrangement, not a fallback for original-coordinate geometry.
    pub layout: [u32; 2],
}

impl<'a> Request<'a> {
    pub fn new(
        source: &'a ProcessingSource,
        input: &'a AuthoredPhotograph,
        plan: Option<&'a Plan>,
        layout: [u32; 2],
    ) -> Self {
        Self {
            source,
            input,
            plan,
            layout,
        }
    }

    pub fn encode(
        self,
        renderer: &GpuRenderer,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        working: &wgpu::Buffer,
    ) -> Result<Prepared, Error> {
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
                .unwrap_or_else(|| Raster::native([0, 0, self.layout[0], self.layout[1]].into()))
        });
        let stages = self
            .plan
            .map(|plan| plan.pipeline().clone())
            .unwrap_or_default();
        let root = crate::gpu::photograph::PhotographGraph::from_resident_with_stages(
            &device.limits(),
            self.source,
            self.input,
            raster,
            &stages,
        )?
        .processed;
        let prepared =
            Prepared::from_composition(device, raster, root, renderer.evaluation.clone())?;
        prepared.encode_from(device, encoder, &self.source.input, working)?;
        Ok(prepared)
    }
}
