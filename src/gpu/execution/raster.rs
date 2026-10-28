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
    #[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
    pub processing_queries: Option<(&'a wgpu::QuerySet, u32)>,
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
            #[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
            processing_queries: None,
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
        let prepared = self
            .plan
            .filter(|plan| !plan.pipeline().is_empty())
            .map(|plan| {
                if plan
                    .input()
                    .size()
                    .into_iter()
                    .map(u64::from)
                    .product::<u64>()
                    != u64::from(self.source.pixel_count())
                {
                    return Err(Error::InvalidParameter(
                        "processing source layout differs from plan",
                    ));
                }
                Prepared::with_programs(
                    device,
                    plan.pipeline(),
                    Raster::planned(plan),
                    None,
                    renderer.pull_programs.clone(),
                )
            })
            .transpose()?;
        match self.input.pipeline() {
            Some(grade) => {
                #[cfg(all(test, target_os = "macos", feature = "raw", feature = "export"))]
                if let Some(queries) = self.processing_queries {
                    renderer.kernel.encode_grade_with_query(
                        device,
                        encoder,
                        self.source,
                        working,
                        grade,
                        queries,
                    )?;
                } else {
                    renderer
                        .kernel
                        .encode_grade(device, encoder, self.source, working, grade)?;
                }
                #[cfg(not(all(test, target_os = "macos", feature = "raw", feature = "export")))]
                renderer
                    .kernel
                    .encode_grade(device, encoder, self.source, working, grade)?;
            }
            None => renderer.kernel.encode(
                device,
                encoder,
                self.source,
                working,
                self.input.parameters(),
            ),
        }
        #[cfg(any(feature = "editor", all(feature = "raw", feature = "render")))]
        if !self.input.repairs.0.is_empty() {
            let geometry = self.source.geometry().ok_or_else(|| {
                Error::Gpu("repair rendering needs original source geometry".into())
            })?;
            if geometry.region().width == geometry.size()[0]
                && geometry.region().height == geometry.size()[1]
            {
                renderer.repairs.original(
                    device,
                    encoder,
                    &renderer.kernel,
                    working,
                    self.input,
                    geometry,
                )?;
            }
        }

        if let Some(prepared) = &prepared {
            renderer
                .pixel_kernels(device)?
                .encode(device, encoder, prepared, working)?;
        }
        Ok(prepared)
    }
}
