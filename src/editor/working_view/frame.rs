use super::*;
use crate::gpu::photograph::request::PhotographRequest;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;

/// One captured view request and the immutable pixels needed to produce it.
#[derive(Clone)]
pub(in crate::editor) struct Demand {
    pub spec: SourceRequest,
    pub pixels: source::Context,
    pub capture: PhotographRequest,
}

pub(super) struct Calculation {
    pub image: Arc<crate::gpu::evaluation::ImageNode>,
    pub native: Arc<crate::gpu::evaluation::ImageNode>,
    pub plan: crate::acquisition::Plan,
}

impl Demand {
    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.capture.processing(limits, &self.spec.repairs)
    }

    pub(super) fn same_calculation(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
            || self.pixels == other.pixels
                && self.spec == other.spec
                && self.capture == other.capture
    }

    #[cfg(test)]
    pub(super) fn fixture(spec: SourceRequest, pixels: source::Context) -> Self {
        let processing = spec.processing_fixture();
        Self {
            spec,
            pixels,
            capture: PhotographRequest::Fixture(processing),
        }
    }

    pub(super) fn same_source(&self, other: &Self) -> bool {
        self.pixels.source.identity() == other.pixels.source.identity()
            && self.capture.is_inspection() == other.capture.is_inspection()
    }

    pub(super) fn calculation(&self, source: &ViewSource) -> Result<Calculation, Error> {
        let limits = source.renderer.evaluation().device().limits();
        let scan = self.pixels.scan()?;
        let field = self
            .pixels
            .correction
            .as_ref()
            .map(|field| field.gpu.clone());
        if let PhotographRequest::Recipe(recipe) = &self.capture {
            let (photo, plan) = crate::gpu::photograph::PhotographGraph::preview(
                &limits,
                &scan,
                field,
                recipe,
                &self.spec.repairs,
                source.spec.frame(),
                Some(source.spec.plan.output()),
            )?;
            return Ok(Calculation {
                image: photo.processed,
                native: photo.neutral,
                plan,
            });
        }
        let spec = source.spec.clone().with_repairs(self.spec.repairs.clone());
        let native = self
            .capture
            .source_graphs(&scan, field, std::slice::from_ref(&spec))?
            .remove(0);
        let (image, plan) = self.capture.filtered(&limits, &spec, native.clone())?;
        Ok(Calculation {
            image,
            native,
            plan,
        })
    }

    pub(super) fn can_use(&self, source: &ViewSource) -> Result<bool, Error> {
        let Calculation { image, native, .. } = self.calculation(source)?;
        let Some(region) = image.region_for(&native, image.domain().finite()?)? else {
            return Ok(false);
        };
        Ok(source.supplied()?.covers(&native, region))
    }
}

/// Related supplied values, their complete processed calculations and captured provenance.
pub(super) struct Frame {
    pub source: Arc<ViewSource>,
    pub underlay: Option<Arc<ViewSource>>,
    pub request: Arc<Demand>,
    pub processed: Vec<Arc<crate::gpu::view::processing::ProcessedView>>,
}
impl Frame {
    pub(super) fn satisfies(&self, wanted: &Demand) -> bool {
        self.request.same_calculation(wanted) && self.source.spec.frame() == wanted.spec.frame()
    }

    #[cfg(test)]
    pub(super) fn processing(&self, limits: &wgpu::Limits) -> Result<AuthoredPhotograph, Error> {
        self.request.processing(limits)
    }

    pub(super) fn regrade(&self, request: Arc<Demand>) -> Result<Option<Self>, Error> {
        let sources: Vec<_> = std::iter::once(&self.source)
            .chain(self.underlay.as_ref())
            .collect();
        let calculations = sources
            .iter()
            .map(|source| request.calculation(source))
            .collect::<Result<Vec<_>, _>>()?;
        for (source, calculation) in sources.iter().zip(&calculations) {
            let region = calculation
                .image
                .region_for(&calculation.native, calculation.image.domain().finite()?)?
                .ok_or(Error::InvalidParameter("preview has no neutral dependency"))?;
            if !source.supplied()?.covers(&calculation.native, region) {
                return Ok(None);
            }
        }
        let device = self.source.renderer.evaluation().device();
        let processed = sources
            .into_iter()
            .zip(calculations)
            .enumerate()
            .map(|(index, (source, calculation))| {
                crate::gpu::view::processing::ProcessedView::prepare_calculation(
                    device,
                    source.acquired(),
                    calculation.image,
                    calculation.plan,
                    self.processed.get(index),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(Self {
            source: self.source.clone(),
            underlay: self.underlay.clone(),
            request,
            processed,
        }))
    }

    pub(super) fn supports(&self, wanted: &Demand) -> bool {
        // The producing request includes resolved arguments as well as intent: unchanged
        // recipes can acquire new repair values without changing their accepted instructions.
        if self.request.same_calculation(wanted) {
            return true;
        }
        std::iter::once(&self.source)
            .chain(self.underlay.as_ref())
            .all(|source| wanted.can_use(source).unwrap_or(false))
    }
}
